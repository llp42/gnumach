// SPDX-License-Identifier: CMU-Mach
// Derived from device/ds_routines.c:
//   Copyright (c) 1993,1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1996 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The native Mach device service, which `device/ds_routines.c` used to
//! define and <device/ds_routines.h> declares.
//!
//! The `ds_device_*` entry points themselves are in [`ds_routines_ffi`]; this
//! module carries the emulation dispatch, the device open/close/read/write
//! paths, the request completion callbacks and the io-done thread.
//!
//! [`ds_routines_ffi`]: crate::device::ds_routines_ffi

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::i386::irq;
use crate::arch::i386::percpu::current_thread;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::config::NINTR;
use crate::device::dev_lookup;
use crate::device::dev_lookup_ffi;
use crate::device::r#return::{DeviceError, IoResultExt};
use crate::glue;
use crate::ipc::ipc_port_ffi;
use crate::ipc::{IpcPort, MachMsgHeader, ipc_object, ipc_port, ipc_space};
use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_sleep,
    thread_wakeup_prim,
};
use crate::kern::slab::KmemCache;
use crate::kern::slab_ffi::{
    kalloc, kfree, kmem_cache_alloc, kmem_cache_free, kmem_cache_init,
};
use crate::kern::thread_ffi::{stack_privilege, thread_set_own_priority};
use crate::spin::Mutex;
use crate::vm::types::VmProt;
use crate::vm::vm_kern_ffi::{kmem_io_map_deallocate, kmem_submap};
use crate::vm::vm_map::{
    VM_MAP_WAIT_FOR_SPACE, VmMap, VmMapCopy, round_page, trunc_page,
};
use crate::vm::vm_user_ffi::vm_deallocate;
use core::ffi::{
    c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void,
};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};

/// `DEV_STATE_INIT` of <device/dev_hdr.h>.
const DEV_STATE_INIT: c_short = 0;
/// `DEV_STATE_OPENING`.
const DEV_STATE_OPENING: c_short = 1;
/// `DEV_STATE_OPEN`.
const DEV_STATE_OPEN: c_short = 2;
/// `DEV_STATE_CLOSING`.
const DEV_STATE_CLOSING: c_short = 3;

/// `D_EXCL_OPEN` of <device/dev_hdr.h>.
const D_EXCL_OPEN: c_short = 0x0001;

/// `IO_WRITE` of <device/io_req.h>.
const IO_WRITE: c_int = 0x0000_0000;
/// `IO_READ`.
const IO_READ: c_int = 0x0000_0001;
/// `IO_OPEN`.
const IO_OPEN: c_int = 0x0000_0002;
/// `IO_DONE`.
const IO_DONE: c_int = 0x0000_0100;
/// `IO_WANTED`.
const IO_WANTED: c_int = 0x0000_0800;
/// `IO_CALL`.
const IO_CALL: c_int = 0x0000_2000;
/// `IO_INBAND` of <device/io_req.h>.
const IO_INBAND: c_int = 0x0000_4000;
/// `IO_LOANED`.
const IO_LOANED: c_int = 0x0001_0000;

/// `D_SUCCESS` of <device/device_types.h>.
const D_SUCCESS: c_int = 0;
/// `D_IO_QUEUED` of <device/device_types.h>.
const D_IO_QUEUED: c_int = -1;
/// `MIG_NO_REPLY` of <mach/mig_errors.h>.
pub(crate) const MIG_NO_REPLY: c_int = -305;
/// `KERN_SUCCESS`.
pub(crate) const KERN_SUCCESS: c_int = 0;
/// `KERN_INVALID_ARGUMENT`.
const KERN_INVALID_ARGUMENT: c_int = 4;
/// `KERN_FAILURE`.
const KERN_FAILURE: c_int = 5;
/// `KERN_RESOURCE_SHORTAGE`.
const KERN_RESOURCE_SHORTAGE: c_int = 6;
/// `KERN_INVALID_VALUE`.
const KERN_INVALID_VALUE: c_int = 18;
/// `D_INFO_BLOCK_SIZE` of <device/conf.h>.
const D_INFO_BLOCK_SIZE: c_int = 1;
/// `IO_INBAND_MAX` of <device/device_types.h>.
const IO_INBAND_MAX: usize = 128;
/// `MACH_NOTIFY_NO_SENDERS` of <mach/notify.h>.
const MACH_NOTIFY_NO_SENDERS: c_int = 0o106;
/// `DEVICE_IO_MAP_SIZE` of `device/ds_routines.c`.
const DEVICE_IO_MAP_SIZE: VmSize = 16 * 1024 * 1024;
/// `IOTRAP_REQSIZE` of `device/ds_routines.c`.
const IOTRAP_REQSIZE: usize = 2048;
/// The `stack_iovec[16]` bound of `device_writev_trap()`.
const MAX_IOVECS: usize = 16;

/// `struct device` of <device/dev_hdr.h>: the emulation handle embedded at
/// the end of a [`MachDevice`].
#[repr(C)]
pub struct Device {
    pub emul_ops: *mut DeviceEmulationOps,
    pub emul_data: *mut c_void,
}

const _: () = {
    assert!(size_of::<Device>() == 2 * size_of::<*mut c_void>());
    assert!(align_of::<Device>() == align_of::<*mut c_void>());
    assert!(offset_of!(Device, emul_ops) == 0);
    assert!(offset_of!(Device, emul_data) == size_of::<*mut c_void>());
};

/// `struct mach_device` of <device/dev_hdr.h>: one open device record.
///
/// The mirror keeps every field of the C record; the device-lookup paths in
/// C still read `ref_count` and `number_chain`, and this module reads only the
/// fields the open/close/IO paths touch.
#[repr(C)]
pub struct MachDevice {
    pub ref_lock: SimpleLock,
    pub ref_count: c_int,
    pub lock: SimpleLock,
    pub state: c_short,
    pub flag: c_short,
    pub open_count: c_short,
    pub io_in_progress: c_short,
    pub io_wait: c_int,
    pub port: *mut c_void,
    pub number_chain: QueueEntry,
    pub dev_number: c_int,
    pub bsize: c_int,
    pub dev_ops: *mut DevOps,
    pub dev: Device,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<MachDevice>() == 80);
    assert!(align_of::<MachDevice>() == 8);
    assert!(offset_of!(MachDevice, ref_lock) == 0);
    assert!(offset_of!(MachDevice, ref_count) == 4);
    assert!(offset_of!(MachDevice, lock) == 8);
    assert!(offset_of!(MachDevice, state) == 12);
    assert!(offset_of!(MachDevice, flag) == 14);
    assert!(offset_of!(MachDevice, open_count) == 16);
    assert!(offset_of!(MachDevice, io_in_progress) == 18);
    assert!(offset_of!(MachDevice, io_wait) == 20);
    assert!(offset_of!(MachDevice, port) == 24);
    assert!(offset_of!(MachDevice, number_chain) == 32);
    assert!(offset_of!(MachDevice, dev_number) == 48);
    assert!(offset_of!(MachDevice, bsize) == 52);
    assert!(offset_of!(MachDevice, dev_ops) == 56);
    assert!(offset_of!(MachDevice, dev) == 64);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<MachDevice>() == 56);
    assert!(align_of::<MachDevice>() == 4);
    assert!(offset_of!(MachDevice, ref_lock) == 0);
    assert!(offset_of!(MachDevice, ref_count) == 4);
    assert!(offset_of!(MachDevice, lock) == 8);
    assert!(offset_of!(MachDevice, state) == 12);
    assert!(offset_of!(MachDevice, flag) == 14);
    assert!(offset_of!(MachDevice, open_count) == 16);
    assert!(offset_of!(MachDevice, io_in_progress) == 18);
    assert!(offset_of!(MachDevice, io_wait) == 20);
    assert!(offset_of!(MachDevice, port) == 24);
    assert!(offset_of!(MachDevice, number_chain) == 28);
    assert!(offset_of!(MachDevice, dev_number) == 36);
    assert!(offset_of!(MachDevice, bsize) == 40);
    assert!(offset_of!(MachDevice, dev_ops) == 44);
    assert!(offset_of!(MachDevice, dev) == 48);
};

/// `struct dev_ops` of <device/conf.h>: one driver's entry points.
///
/// The mirror keeps every field; this module dispatches through `d_open`,
/// `d_close`, `d_read`, `d_write`, `d_getstat`, `d_setstat`, `d_async_in` and
/// `d_dev_info` only.
#[repr(C)]
pub struct DevOps {
    pub d_name: *mut c_char,
    pub d_open: Option<unsafe extern "C" fn(DevT, c_int, *mut IoReq) -> c_int>,
    pub d_close: Option<unsafe extern "C" fn(DevT, c_int)>,
    pub d_read: Option<unsafe extern "C" fn(DevT, *mut IoReq) -> c_int>,
    pub d_write: Option<unsafe extern "C" fn(DevT, *mut IoReq) -> c_int>,
    pub d_getstat: Option<
        unsafe extern "C" fn(DevT, c_uint, *mut c_int, *mut c_uint) -> c_int,
    >,
    pub d_setstat: Option<
        unsafe extern "C" fn(DevT, c_uint, *mut c_int, c_uint) -> c_int,
    >,
    pub d_mmap:
        Option<unsafe extern "C" fn(DevT, VmOffset, c_int) -> VmOffset>,
    pub d_async_in: Option<
        unsafe extern "C" fn(
            DevT,
            *mut c_void,
            c_int,
            *mut c_ushort,
            c_uint,
        ) -> c_int,
    >,
    pub d_reset: Option<unsafe extern "C" fn(DevT) -> c_int>,
    pub d_port_death: Option<unsafe extern "C" fn(DevT, VmOffset) -> c_int>,
    pub d_subdev: c_int,
    pub d_dev_info:
        Option<unsafe extern "C" fn(DevT, c_int, *mut c_int) -> c_int>,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<DevOps>() == 104);
    assert!(align_of::<DevOps>() == 8);
    assert!(offset_of!(DevOps, d_name) == 0);
    assert!(offset_of!(DevOps, d_open) == 8);
    assert!(offset_of!(DevOps, d_close) == 16);
    assert!(offset_of!(DevOps, d_read) == 24);
    assert!(offset_of!(DevOps, d_write) == 32);
    assert!(offset_of!(DevOps, d_getstat) == 40);
    assert!(offset_of!(DevOps, d_setstat) == 48);
    assert!(offset_of!(DevOps, d_mmap) == 56);
    assert!(offset_of!(DevOps, d_async_in) == 64);
    assert!(offset_of!(DevOps, d_reset) == 72);
    assert!(offset_of!(DevOps, d_port_death) == 80);
    assert!(offset_of!(DevOps, d_subdev) == 88);
    assert!(offset_of!(DevOps, d_dev_info) == 96);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<DevOps>() == 52);
    assert!(align_of::<DevOps>() == 4);
    assert!(offset_of!(DevOps, d_name) == 0);
    assert!(offset_of!(DevOps, d_open) == 4);
    assert!(offset_of!(DevOps, d_close) == 8);
    assert!(offset_of!(DevOps, d_read) == 12);
    assert!(offset_of!(DevOps, d_write) == 16);
    assert!(offset_of!(DevOps, d_getstat) == 20);
    assert!(offset_of!(DevOps, d_setstat) == 24);
    assert!(offset_of!(DevOps, d_mmap) == 28);
    assert!(offset_of!(DevOps, d_async_in) == 32);
    assert!(offset_of!(DevOps, d_reset) == 36);
    assert!(offset_of!(DevOps, d_port_death) == 40);
    assert!(offset_of!(DevOps, d_subdev) == 44);
    assert!(offset_of!(DevOps, d_dev_info) == 48);
};

/// `struct device_emulation_ops` of <device/device_emul.h>: the operations
/// one emulation layer provides.
#[repr(C)]
pub struct DeviceEmulationOps {
    pub reference: Option<unsafe extern "C" fn(*mut c_void)>,
    pub dealloc: Option<unsafe extern "C" fn(*mut c_void)>,
    pub dev_to_port: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub open: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_uint,
            c_uint,
            *const c_char,
            *mut *mut c_void,
        ) -> c_int,
    >,
    pub close: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    pub write: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_uint,
            c_uint,
            c_ulong,
            *mut c_char,
            c_uint,
            *mut c_int,
        ) -> c_int,
    >,
    pub write_inband: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_uint,
            c_uint,
            c_ulong,
            *const c_char,
            c_uint,
            *mut c_int,
        ) -> c_int,
    >,
    pub read: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_uint,
            c_uint,
            c_ulong,
            c_int,
            *mut *mut c_char,
            *mut c_uint,
        ) -> c_int,
    >,
    pub read_inband: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_uint,
            c_uint,
            c_ulong,
            c_int,
            *mut c_char,
            *mut c_uint,
        ) -> c_int,
    >,
    pub set_status: Option<
        unsafe extern "C" fn(*mut c_void, c_uint, *mut c_int, c_uint) -> c_int,
    >,
    pub get_status: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_uint,
            *mut c_int,
            *mut c_uint,
        ) -> c_int,
    >,
    pub set_filter: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_int,
            *mut c_ushort,
            c_uint,
        ) -> c_int,
    >,
    pub map: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_int,
            VmOffset,
            VmSize,
            *mut *mut c_void,
            c_int,
        ) -> c_int,
    >,
    pub no_senders: Option<unsafe extern "C" fn(*mut c_void)>,
    pub write_trap: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_uint,
            c_ulong,
            c_ulong,
            c_ulong,
        ) -> c_int,
    >,
    pub writev_trap: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_uint,
            c_ulong,
            *mut RpcIoBufVec,
            c_ulong,
        ) -> c_int,
    >,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<DeviceEmulationOps>() == 128);
    assert!(align_of::<DeviceEmulationOps>() == 8);
    assert!(offset_of!(DeviceEmulationOps, reference) == 0);
    assert!(offset_of!(DeviceEmulationOps, dealloc) == 8);
    assert!(offset_of!(DeviceEmulationOps, dev_to_port) == 16);
    assert!(offset_of!(DeviceEmulationOps, open) == 24);
    assert!(offset_of!(DeviceEmulationOps, close) == 32);
    assert!(offset_of!(DeviceEmulationOps, write) == 40);
    assert!(offset_of!(DeviceEmulationOps, write_inband) == 48);
    assert!(offset_of!(DeviceEmulationOps, read) == 56);
    assert!(offset_of!(DeviceEmulationOps, read_inband) == 64);
    assert!(offset_of!(DeviceEmulationOps, set_status) == 72);
    assert!(offset_of!(DeviceEmulationOps, get_status) == 80);
    assert!(offset_of!(DeviceEmulationOps, set_filter) == 88);
    assert!(offset_of!(DeviceEmulationOps, map) == 96);
    assert!(offset_of!(DeviceEmulationOps, no_senders) == 104);
    assert!(offset_of!(DeviceEmulationOps, write_trap) == 112);
    assert!(offset_of!(DeviceEmulationOps, writev_trap) == 120);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<DeviceEmulationOps>() == 64);
    assert!(align_of::<DeviceEmulationOps>() == 4);
    assert!(offset_of!(DeviceEmulationOps, reference) == 0);
    assert!(offset_of!(DeviceEmulationOps, dealloc) == 4);
    assert!(offset_of!(DeviceEmulationOps, dev_to_port) == 8);
    assert!(offset_of!(DeviceEmulationOps, open) == 12);
    assert!(offset_of!(DeviceEmulationOps, close) == 16);
    assert!(offset_of!(DeviceEmulationOps, write) == 20);
    assert!(offset_of!(DeviceEmulationOps, write_inband) == 24);
    assert!(offset_of!(DeviceEmulationOps, read) == 28);
    assert!(offset_of!(DeviceEmulationOps, read_inband) == 32);
    assert!(offset_of!(DeviceEmulationOps, set_status) == 36);
    assert!(offset_of!(DeviceEmulationOps, get_status) == 40);
    assert!(offset_of!(DeviceEmulationOps, set_filter) == 44);
    assert!(offset_of!(DeviceEmulationOps, map) == 48);
    assert!(offset_of!(DeviceEmulationOps, no_senders) == 52);
    assert!(offset_of!(DeviceEmulationOps, write_trap) == 56);
    assert!(offset_of!(DeviceEmulationOps, writev_trap) == 60);
};

/// `mach_no_senders_notification_t` of <mach/notify.h>: the notification
/// `ds_notify()` handles.
#[repr(C)]
struct NoSendersNotification {
    header: MachMsgHeader,
    /// The C's `mach_msg_type_t` word, present for the layout only.
    not_type: usize,
    not_count: c_uint,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NoSendersNotification>() == 48);
    assert!(align_of::<NoSendersNotification>() == 8);
    assert!(offset_of!(NoSendersNotification, header) == 0);
    assert!(offset_of!(NoSendersNotification, not_type) == 32);
    assert!(offset_of!(NoSendersNotification, not_count) == 40);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NoSendersNotification>() == 32);
    assert!(align_of::<NoSendersNotification>() == 4);
    assert!(offset_of!(NoSendersNotification, header) == 0);
    assert!(offset_of!(NoSendersNotification, not_type) == 24);
    assert!(offset_of!(NoSendersNotification, not_count) == 28);
};

/// `io_buf_vec_t` of <device/device_types.h>: one scatter/gather segment of
/// kernel addresses.
#[repr(C)]
#[derive(Clone, Copy)]
struct IoBufVec {
    data: VmOffset,
    count: VmSize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IoBufVec>() == 16);
    assert!(align_of::<IoBufVec>() == 8);
    assert!(offset_of!(IoBufVec, data) == 0);
    assert!(offset_of!(IoBufVec, count) == 8);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IoBufVec>() == 8);
    assert!(align_of::<IoBufVec>() == 4);
    assert!(offset_of!(IoBufVec, data) == 0);
    assert!(offset_of!(IoBufVec, count) == 4);
};

/// `rpc_io_buf_vec_t` of <device/device_types.h>: one scatter/gather segment
/// of user addresses, as MIG passes them.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RpcIoBufVec {
    pub data: c_ulong,
    pub count: c_ulong,
}

const _: () = {
    assert!(size_of::<RpcIoBufVec>() == 2 * size_of::<c_ulong>());
    assert!(align_of::<RpcIoBufVec>() == align_of::<c_ulong>());
    assert!(offset_of!(RpcIoBufVec, data) == 0);
    assert!(offset_of!(RpcIoBufVec, count) == size_of::<c_ulong>());
};

/// `emulation_list[]` of `device/ds_routines.c`: the emulations
/// [`ds_device_open()`] tries in order.
static mut EMULATION_LIST: [*mut DeviceEmulationOps; 1] =
    [&raw mut MACH_DEVICE_EMULATION_OPS];

/// `device_io_map_store` of `device/ds_routines.c`: the storage of the map
/// every device IO buffer is mapped through.
static mut DEVICE_IO_MAP_STORE: VmMap = VmMap::zeroed();

/// `device_io_map` of <device/ds_routines.h>.
#[unsafe(no_mangle)]
pub static mut device_io_map: *mut VmMap = &raw mut DEVICE_IO_MAP_STORE;

/// `io_inband_cache` of <device/io_req.h>: the cache for inband read
/// buffers.
#[unsafe(export_name = "io_inband_cache")]
static mut IO_INBAND_CACHE: KmemCache = KmemCache::zeroed();

/// `io_trap_cache` of `device/ds_routines.c`: the cache for the trap path's
/// `io_req` blocks.
#[unsafe(export_name = "io_trap_cache")]
static mut IO_TRAP_CACHE: KmemCache = KmemCache::zeroed();

/// `io_done_list` of <device/ds_routines.h>: the requests the io-done thread
/// still has to complete.
#[unsafe(export_name = "io_done_list")]
static mut IO_DONE_LIST: QueueEntry = QueueEntry::unlinked();

/// `io_done_list_lock` of `device/ds_routines.c`.  The C held it at
/// `splhigh()`; the callers keep that interrupt level.
static IO_DONE_LIST_LOCK: Mutex<()> = Mutex::new(());

/// `mach_device_emulation_ops` of `device/ds_routines.c`: the native Mach
/// device emulation every device lookup installs.
#[unsafe(export_name = "mach_device_emulation_ops")]
pub(crate) static mut MACH_DEVICE_EMULATION_OPS: DeviceEmulationOps =
    DeviceEmulationOps {
        reference: Some(dev_lookup_ffi::mach_device_reference),
        dealloc: Some(dev_lookup_ffi::mach_device_deallocate),
        dev_to_port: Some(mach_convert_device_to_port),
        open: Some(device_open),
        close: Some(device_close),
        write: Some(device_write),
        write_inband: Some(device_write_inband),
        read: Some(device_read),
        read_inband: Some(device_read_inband),
        set_status: Some(device_set_status),
        get_status: Some(mach_device_get_status),
        set_filter: Some(device_set_filter),
        map: Some(device_map),
        no_senders: Some(ds_no_senders),
        write_trap: Some(device_write_trap),
        writev_trap: Some(device_writev_trap),
    };

/// The C's implicit `int` to `dev_t` truncation at every driver call; a
/// device number comes from the device table and fits in sixteen bits.
pub(crate) fn driver_unit(dev_number: c_int) -> DevT {
    dev_number as DevT
}

/// The C's implicit `unsigned int` to `long` conversion of an IO byte count.
fn io_count(count: c_uint) -> c_long {
    count as c_long
}

/// `io_req_alloc` of <device/io_req.h>.
///
/// # Safety
///
/// The allocator must be initialized.
unsafe fn io_req_alloc() -> *mut IoReq {
    // SAFETY: the caller promises the initialized allocator, and the size is
    // the C `sizeof(struct io_req)`.
    let addr = unsafe { kalloc(size_of::<IoReq>()) };
    let ior = ptr::with_exposed_provenance_mut::<IoReq>(addr);
    // SAFETY: a successful allocation is aligned for the request, and the C
    // initializes the embedded lock before handing it to a driver.
    unsafe { (*ior).lock.init() };
    ior
}

/// `io_req_free` of <device/io_req.h>.
///
/// # Safety
///
/// `ior` must be a live request from [`io_req_alloc()`] that nothing uses.
unsafe fn io_req_free(ior: *mut IoReq) {
    // SAFETY: the caller promises the live allocation.
    unsafe { kfree(ior.addr(), size_of::<IoReq>()) };
}

/// `ds_device_open()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `open_port` is the master device port, `reply_port` is a valid port or
/// `IP_NULL`, `name` is a NUL-terminated device name, and `devp` is writable.
pub(crate) unsafe extern "C" fn ds_device_open(
    open_port: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    name: *const c_char,
    devp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the boot path wrote `master_device_port` before any open could
    // arrive.
    if open_port != unsafe { glue::master_device_port } {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }

    if IpcPort::valid(reply_port).is_none() {
        // SAFETY: both C routines take the literal arguments below.
        unsafe {
            glue::printf(c"ds_* invalid reply port\n".as_ptr());
            glue::SoftDebugger(c"ds_* reply_port".as_ptr());
        }
        return MIG_NO_REPLY;
    }

    // SAFETY: the list is this module's one-entry static, and its entry is the
    // address of the live ops table.
    let ops = unsafe {
        *ptr::addr_of!(EMULATION_LIST).cast::<*mut DeviceEmulationOps>()
    };
    // SAFETY: the emulation registered a real open with the C signature.
    match unsafe { (*ops).open } {
        Some(open) => {
            // SAFETY: the caller's contract meets the emulation's.
            unsafe { open(reply_port, reply_port_type, mode, name, devp) }
        }
        None => D_SUCCESS,
    }
}

/// `ds_device_close()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is [`DEVICE_NULL`](core::ptr::null_mut) or a live `struct device`.
pub(crate) unsafe extern "C" fn ds_device_close(dev: *mut c_void) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).close {
            Some(close) => close((*dev).emul_data),
            None => D_SUCCESS,
        }
    }
}

/// `ds_device_write()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, `data` readable for `count` bytes
/// when non-null, and `bytes_written` writable.
pub(crate) unsafe extern "C" fn ds_device_write(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *mut c_char,
    count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    if data.is_null() {
        return Err(DeviceError::InvalidSize).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).write {
            Some(write) => write(
                (*dev).emul_data,
                reply_port,
                reply_port_type,
                mode,
                recnum,
                data,
                count,
                bytes_written,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_write_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, `data` readable for `count` bytes
/// when non-null, and `bytes_written` writable.
pub(crate) unsafe extern "C" fn ds_device_write_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *const c_char,
    count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    if data.is_null() {
        return Err(DeviceError::InvalidSize).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).write_inband {
            Some(write) => write(
                (*dev).emul_data,
                reply_port,
                reply_port_type,
                mode,
                recnum,
                data,
                count,
                bytes_written,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_read()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, and `data` and `bytes_read` must be
/// writable.
pub(crate) unsafe extern "C" fn ds_device_read(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    count: c_int,
    data: *mut *mut c_char,
    bytes_read: *mut c_uint,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).read {
            Some(read) => read(
                (*dev).emul_data,
                reply_port,
                reply_port_type,
                mode,
                recnum,
                count,
                data,
                bytes_read,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_read_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, `data` writable for the reply, and
/// `bytes_read` writable.
pub(crate) unsafe extern "C" fn ds_device_read_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    count: c_int,
    data: *mut c_char,
    bytes_read: *mut c_uint,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).read_inband {
            Some(read) => read(
                (*dev).emul_data,
                reply_port,
                reply_port_type,
                mode,
                recnum,
                count,
                data,
                bytes_read,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_set_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, and `status` readable for
/// `status_count` integers when the emulation reads it.
pub(crate) unsafe extern "C" fn ds_device_set_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: c_uint,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).set_status {
            Some(set_status) => {
                set_status((*dev).emul_data, flavor, status, status_count)
            }
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_get_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, `status` writable for
/// `*status_count` integers, and `status_count` writable.
pub(crate) unsafe extern "C" fn ds_device_get_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: *mut c_uint,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).get_status {
            Some(get_status) => {
                get_status((*dev).emul_data, flavor, status, status_count)
            }
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_set_filter()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, `receive_port` a valid port, and
/// `filter` readable for `filter_count` entries.
pub(crate) unsafe extern "C" fn ds_device_set_filter(
    dev: *mut c_void,
    receive_port: *mut c_void,
    priority: c_int,
    filter: *mut c_ushort,
    filter_count: c_uint,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).set_filter {
            Some(set_filter) => set_filter(
                (*dev).emul_data,
                receive_port,
                priority,
                filter,
                filter_count,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_map()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, and `pager` writable.
pub(crate) unsafe extern "C" fn ds_device_map(
    dev: *mut c_void,
    protection: c_int,
    offset: VmOffset,
    size: VmSize,
    pager: *mut *mut c_void,
    unmap: c_int,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).map {
            Some(map) => {
                map((*dev).emul_data, protection, offset, size, pager, unmap)
            }
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_intr_register()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device` for a mach device, and `receive_port`
/// a valid port.
pub(crate) unsafe extern "C" fn ds_device_intr_register(
    dev: *mut c_void,
    id: c_int,
    flags: c_int,
    receive_port: *mut c_void,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record, whose emulation
    // data is the owning `mach_device`.
    let mdev = unsafe { (*dev).emul_data.cast::<MachDevice>() };

    if flags != 0 {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }

    // SAFETY: the caller promises a live mach device, whose driver table and
    // name are live for its lifetime.
    let same_name = unsafe {
        crate::device::dev_name::name_equal(
            (*(*mdev).dev_ops).d_name,
            3,
            c"irq".as_ptr(),
        )
    };
    if same_name == 0 {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }

    if id < 0 {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }
    // The C compared the non-negative id against the NINTR-sized table.
    if id as usize >= NINTR {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }

    // SAFETY: `irqtab` is the live interrupt table, and the id is inside its
    // NINTR entries.
    let entry = unsafe {
        glue::insert_intr_entry(
            ptr::addr_of_mut!(irq::irqtab),
            id,
            receive_port,
        )
    };
    if entry.is_null() {
        return Err(DeviceError::NoMemory).as_io_return();
    }

    // SAFETY: the entry belongs to the table, which serializes its use.
    let err = unsafe {
        glue::install_user_intr_handler(
            ptr::addr_of_mut!(irq::irqtab),
            id,
            flags as c_ulong,
            entry,
        )
    };
    if err == D_SUCCESS {
        // SAFETY: the handler holds a reference to the live port from here
        // on, as the C's `ip_reference()` recorded.
        unsafe { ipc_object::reference(receive_port) };
    }
    err
}

/// `ds_device_intr_ack()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device` for a mach device, and `receive_port`
/// a valid port.
pub(crate) unsafe extern "C" fn ds_device_intr_ack(
    dev: *mut c_void,
    receive_port: *mut c_void,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record, whose emulation
    // data is the owning `mach_device`.
    let mdev = unsafe { (*dev).emul_data.cast::<MachDevice>() };

    // SAFETY: the caller promises a live mach device, whose driver table and
    // name are live for its lifetime.
    let same_name = unsafe {
        crate::device::dev_name::name_equal(
            (*(*mdev).dev_ops).d_name,
            3,
            c"irq".as_ptr(),
        )
    };
    if same_name == 0 {
        return Err(DeviceError::InvalidOperation).as_io_return();
    }

    // SAFETY: the caller promises a live irq port.
    let ret = unsafe { glue::irq_acknowledge(receive_port) };
    if ret == D_SUCCESS {
        // SAFETY: the acknowledge consumed the send right the registration
        // held.
        unsafe { ipc_port_ffi::ipc_port_release_send(receive_port) };
    }
    ret
}

/// `ds_notify()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `msg` must be a live message whose header and no-senders body are
/// readable.
pub(crate) unsafe extern "C" fn ds_notify(msg: *mut c_void) -> c_int {
    let msg = msg.cast::<NoSendersNotification>();
    // SAFETY: the caller promises the live message.
    unsafe {
        let header = ptr::addr_of!((*msg).header);
        if (*header).id() == MACH_NOTIFY_NO_SENDERS {
            let port =
                ptr::with_exposed_provenance_mut::<c_void>((*header).remote());
            let dev = dev_lookup::port_lookup(port);
            let ops = (*dev).emul_ops;
            if let Some(no_senders) = (*ops).no_senders {
                no_senders(msg.cast::<c_void>());
            }
            return c_int::from(true);
        }

        glue::printf(
            c"ds_notify: strange notification %d\n".as_ptr(),
            (*header).id(),
        );
    }
    c_int::from(false)
}

/// `ds_device_write_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`.
pub(crate) unsafe extern "C" fn ds_device_write_trap(
    dev: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    data: c_ulong,
    count: c_ulong,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).write_trap {
            Some(write_trap) => {
                write_trap((*dev).emul_data, mode, recnum, data, count)
            }
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `ds_device_writev_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be a live `struct device`, and `iovec` readable for `count`
/// user-space entries.
pub(crate) unsafe extern "C" fn ds_device_writev_trap(
    dev: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    iovec: *mut RpcIoBufVec,
    count: c_ulong,
) -> c_int {
    if dev.is_null() {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        match (*ops).writev_trap {
            Some(writev_trap) => {
                writev_trap((*dev).emul_data, mode, recnum, iovec, count)
            }
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `device_reference()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is null or a live `struct device`.
pub(crate) unsafe extern "C" fn device_reference(dev: *mut c_void) {
    if dev.is_null() {
        return;
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        if let Some(reference) = (*ops).reference {
            reference((*dev).emul_data);
        }
    }
}

/// `device_deallocate()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is null or a live `struct device`.
pub(crate) unsafe extern "C" fn device_deallocate(dev: *mut c_void) {
    if dev.is_null() {
        return;
    }
    let dev = dev.cast::<Device>();
    // SAFETY: the caller promises the live device record.
    unsafe {
        let ops = (*dev).emul_ops;
        if let Some(dealloc) = (*ops).dealloc {
            dealloc((*dev).emul_data);
        }
    }
}

/// `mach_convert_device_to_port()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `device` is null or a live `mach_device`.
unsafe extern "C" fn mach_convert_device_to_port(
    device: *mut c_void,
) -> *mut c_void {
    if device.is_null() {
        return ptr::null_mut();
    }
    let device = device.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        (*device).lock.lock();
        let port = if (*device).state == DEV_STATE_OPEN {
            ipc_port_ffi::ipc_port_make_send((*device).port)
        } else {
            ptr::null_mut()
        };
        (*device).lock.unlock();

        dev_lookup::deallocate(device);

        port
    }
}

/// `device_open()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `name` must be a NUL-terminated device name, and `device_p` writable.
unsafe extern "C" fn device_open(
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    name: *const c_char,
    device_p: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the NUL-terminated name; the lookup returns
    // a live device or null.
    let Some(device) = (unsafe { dev_lookup::lookup(name) }) else {
        return Err(DeviceError::NoSuchDevice).as_io_return();
    };
    let device = device.as_ptr();

    // SAFETY: a live mach device owns its lock, and the caller promises the
    // writable handle slot.
    unsafe {
        (*device).lock.lock();
        while (*device).state == DEV_STATE_OPENING
            || (*device).state == DEV_STATE_CLOSING
        {
            (*device).io_wait = c_int::from(true);
            thread_sleep(
                device.cast::<c_void>(),
                ptr::addr_of_mut!((*device).lock),
                c_int::from(true),
            );
            (*device).lock.lock();
        }

        if (*device).state == DEV_STATE_OPEN {
            if (*device).flag & D_EXCL_OPEN != 0 {
                (*device).lock.unlock();
                dev_lookup::deallocate(device);
                return Err(DeviceError::AlreadyOpen).as_io_return();
            }

            (*device).open_count += 1;
            (*device).lock.unlock();
            device_p.write(ptr::addr_of_mut!((*device).dev).cast::<c_void>());
            return D_SUCCESS;
        }

        (*device).state = DEV_STATE_OPENING;
        (*device).lock.unlock();

        (*device).port = ipc_port::alloc_special(ipc_space::kernel())
            .map_or(ptr::null_mut(), IpcPort::as_ptr);
        if (*device).port.is_null() {
            (*device).lock.lock();
            (*device).state = DEV_STATE_INIT;
            (*device).port = ptr::null_mut();
            if (*device).io_wait != 0 {
                (*device).io_wait = c_int::from(false);
                thread_wakeup_prim(
                    device.cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            (*device).lock.unlock();
            dev_lookup::deallocate(device);
            return KERN_RESOURCE_SHORTAGE;
        }

        dev_lookup::port_enter(device);

        let notify = ipc_port_ffi::ipc_port_make_sonce((*device).port);
        let port = IpcPort::from_raw((*device).port);
        port.lock();
        ipc_port::nsrequest(port, 1, notify);

        let ior = io_req_alloc();
        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_OPEN | IO_CALL;
        (*ior).mode = mode;
        (*ior).error = 0;
        (*ior).done = Some(ds_open_done);
        (*ior).reply_port = reply_port;
        (*ior).reply_port_type = reply_port_type;

        let d_open = (*(*device).dev_ops).d_open;
        let result = match d_open {
            Some(d_open) => {
                d_open(driver_unit((*device).dev_number), mode as c_int, ior)
            }
            None => D_SUCCESS,
        };
        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        (*ior).error = result;
        ds_open_done(ior);
        io_req_free(ior);
    }

    MIG_NO_REPLY
}

/// `ds_open_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be the live open request [`device_open()`] built.
pub(crate) unsafe extern "C" fn ds_open_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        let mut device = (*ior).device.cast::<MachDevice>();
        let result = (*ior).error;

        if result != D_SUCCESS {
            dev_lookup::port_remove(device);
            ipc_port::dealloc_special(IpcPort::from_raw((*device).port));
            (*device).port = ptr::null_mut();

            (*device).lock.lock();
            (*device).state = DEV_STATE_INIT;
            if (*device).io_wait != 0 {
                (*device).io_wait = c_int::from(false);
                thread_wakeup_prim(
                    device.cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            (*device).lock.unlock();

            dev_lookup::deallocate(device);
            device = ptr::null_mut();
        } else {
            (*device).lock.lock();
            (*device).state = DEV_STATE_OPEN;
            (*device).open_count = 1;
            if (*device).io_wait != 0 {
                (*device).io_wait = c_int::from(false);
                thread_wakeup_prim(
                    device.cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            (*device).lock.unlock();
        }

        if IpcPort::valid((*ior).reply_port).is_some() {
            glue::ds_device_open_reply(
                (*ior).reply_port,
                (*ior).reply_port_type,
                result,
                mach_convert_device_to_port(device.cast::<c_void>()),
            );
        } else if !device.is_null() {
            dev_lookup::deallocate(device);
        }
    }

    c_int::from(true)
}

/// `device_close()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device.
unsafe extern "C" fn device_close(dev: *mut c_void) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        (*device).lock.lock();

        (*device).open_count -= 1;
        if (*device).open_count > 0 {
            (*device).lock.unlock();
            return D_SUCCESS;
        }

        if (*device).state == DEV_STATE_CLOSING {
            (*device).lock.unlock();
            return D_SUCCESS;
        }

        (*device).state = DEV_STATE_CLOSING;
        (*device).lock.unlock();

        dev_lookup::port_remove(device);
        ipc_port::dealloc_special(IpcPort::from_raw((*device).port));

        if let Some(d_close) = (*(*device).dev_ops).d_close {
            d_close(driver_unit((*device).dev_number), 0);
        }

        (*device).lock.lock();
        (*device).state = DEV_STATE_INIT;
        if (*device).io_wait != 0 {
            (*device).io_wait = c_int::from(false);
            thread_wakeup_prim(device.cast::<c_void>(), 0, THREAD_AWAKENED);
        }
        (*device).lock.unlock();
    }

    D_SUCCESS
}

/// `device_write()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, `data` readable
/// for `data_count` bytes, and `bytes_written` writable.
unsafe extern "C" fn device_write(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *mut c_char,
    data_count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        let ior = io_req_alloc();
        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_WRITE | IO_CALL;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data = data;
        (*ior).count = io_count(data_count);
        (*ior).total = io_count(data_count);
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_write_done);
        (*ior).reply_port = reply_port;
        (*ior).reply_port_type = reply_port_type;
        (*ior).copy = ptr::null_mut();

        dev_lookup::reference(device);

        let result = loop {
            let d_write = (*(*device).dev_ops).d_write;
            let result = match d_write {
                Some(d_write) => {
                    d_write(driver_unit((*device).dev_number), ior)
                }
                None => D_SUCCESS,
            };

            if result == D_IO_QUEUED {
                return MIG_NO_REPLY;
            }

            if device_write_dealloc(ior) != 0 {
                break result;
            }
        };

        bytes_written.write(((*ior).total - (*ior).residual) as c_int);

        dev_lookup::deallocate(device);

        io_req_free(ior);
        result
    }
}

/// `device_write_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, `data` readable
/// for `data_count` bytes, and `bytes_written` writable.
unsafe extern "C" fn device_write_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *const c_char,
    data_count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        let ior = io_req_alloc();
        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_WRITE | IO_CALL | IO_INBAND;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data = data.cast_mut();
        (*ior).count = io_count(data_count);
        (*ior).total = io_count(data_count);
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_write_done);
        (*ior).reply_port = reply_port;
        (*ior).reply_port_type = reply_port_type;

        dev_lookup::reference(device);

        let d_write = (*(*device).dev_ops).d_write;
        let result = match d_write {
            Some(d_write) => d_write(driver_unit((*device).dev_number), ior),
            None => D_SUCCESS,
        };

        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        bytes_written.write(((*ior).total - (*ior).residual) as c_int);

        dev_lookup::deallocate(device);

        io_req_free(ior);
        result
    }
}

/// `device_write_get()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be a live request whose `io_data` is readable for `io_count`
/// bytes, and `wait` writable.
pub(crate) unsafe extern "C" fn device_write_get(
    ior: *mut IoReq,
    wait: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises the live request and writable slot.
    unsafe {
        wait.write(c_int::from(false));

        if (*ior).count == 0 {
            return KERN_SUCCESS;
        }

        if (*ior).op & IO_LOANED != 0 {
            return KERN_SUCCESS;
        }

        if (*ior).op & IO_INBAND != 0 {
            let new_addr =
                kmem_cache_alloc(ptr::addr_of_mut!(IO_INBAND_CACHE));
            ptr::copy_nonoverlapping(
                (*ior).data,
                ptr::with_exposed_provenance_mut::<c_char>(new_addr),
                (*ior).count as usize,
            );
            (*ior).data = ptr::with_exposed_provenance_mut::<c_char>(new_addr);
            (*ior).alloc_size = IO_INBAND_MAX;

            return KERN_SUCCESS;
        }

        let device = (*ior).device.cast::<MachDevice>();
        let mut bsize: c_int = 0;
        let d_dev_info = (*(*device).dev_ops).d_dev_info;
        let result = match d_dev_info {
            Some(d_dev_info) => d_dev_info(
                driver_unit((*device).dev_number),
                D_INFO_BLOCK_SIZE,
                &mut bsize,
            ),
            None => KERN_FAILURE,
        };

        let min_size = if result != KERN_SUCCESS
            || ((*ior).count as VmSize) < (bsize as VmSize)
        {
            (*ior).count as VmSize
        } else {
            bsize as VmSize
        };

        let io_copy = (*ior).data.cast::<VmMapCopy>();
        let mut new_addr: VmOffset = 0;
        let result = glue::kmem_io_map_copyout(
            device_io_map,
            ptr::addr_of_mut!((*ior).data).cast::<VmOffset>(),
            &mut new_addr,
            ptr::addr_of_mut!((*ior).alloc_size),
            io_copy.cast::<c_void>(),
            min_size,
        );
        if result != KERN_SUCCESS {
            return result;
        }

        let data_addr = (*ior).data.addr();
        let data_end = data_addr.wrapping_add((*ior).count as usize);
        let alloc_end = new_addr.wrapping_add((*ior).alloc_size);
        if data_end > alloc_end {
            (*ior).count = (*ior)
                .alloc_size
                .wrapping_sub(data_addr.wrapping_sub(new_addr))
                as c_long;
            (*ior).op &= !IO_CALL;
            wait.write(c_int::from(true));
        }

        (*ior).copy = io_copy;
        KERN_SUCCESS
    }
}

/// `device_write_dealloc()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be a live request from [`io_req_alloc()`].
pub(crate) unsafe extern "C" fn device_write_dealloc(
    ior: *mut IoReq,
) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        if (*ior).alloc_size == 0 {
            return c_int::from(true);
        }

        if (*ior).op & IO_INBAND != 0 {
            kmem_cache_free(
                ptr::addr_of_mut!(IO_INBAND_CACHE),
                (*ior).data.addr(),
            );
            return c_int::from(true);
        }

        let io_copy = (*ior).copy;
        if io_copy.is_null() {
            return c_int::from(true);
        }

        kmem_io_map_deallocate(
            device_io_map,
            trunc_page((*ior).data.addr()),
            (*ior).alloc_size,
        );

        let mut new_copy: *mut VmMapCopy = ptr::null_mut();
        if VmMapCopy::has_cont(NonNull::new_unchecked(io_copy)) {
            let size_to_do =
                (*io_copy).size.wrapping_sub((*ior).count as VmSize);
            let result;
            if (*ior).error == 0 {
                let invoked =
                    VmMapCopy::invoke_cont(NonNull::new_unchecked(io_copy));
                result = invoked.0;
                new_copy = invoked.1;
            } else {
                VmMapCopy::abort_cont(NonNull::new_unchecked(io_copy));
                result = KERN_FAILURE;
            }

            if result == KERN_SUCCESS && !new_copy.is_null() {
                (*ior).op &= !IO_DONE;
                (*ior).op |= IO_CALL;

                let device = (*ior).device.cast::<MachDevice>();
                let mut bsize: c_int = 0;
                let d_dev_info = (*(*device).dev_ops).d_dev_info;
                let res = match d_dev_info {
                    Some(d_dev_info) => d_dev_info(
                        driver_unit((*device).dev_number),
                        D_INFO_BLOCK_SIZE,
                        &mut bsize,
                    ),
                    None => KERN_FAILURE,
                };
                if res != D_SUCCESS {
                    // SAFETY: `Panic` does not return; the tags are the C
                    // `panic()` string.
                    glue::Panic(
                        c"device/ds_routines.c".as_ptr(),
                        line!() as c_int,
                        c"device_write_dealloc".as_ptr(),
                        c"device_write_dealloc: No block size".as_ptr(),
                    );
                }

                (*ior).recnum = (*ior).recnum.wrapping_add(
                    ((*ior).count / c_long::from(bsize)) as c_ulong,
                );
                (*ior).count = (*new_copy).size as c_long;
            } else {
                (*ior).residual =
                    (*ior).residual.wrapping_add(size_to_do as c_long);
            }
        }

        VmMapCopy::discard(NonNull::new_unchecked(io_copy));
        (*ior).copy = ptr::null_mut();
        (*ior).data = new_copy.cast::<c_char>();

        c_int::from(new_copy.is_null())
    }
}

/// `ds_write_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be the live write request [`device_write()`] queued.
pub(crate) unsafe extern "C" fn ds_write_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        loop {
            if device_write_dealloc(ior) != 0 {
                break;
            }

            let device = (*ior).device.cast::<MachDevice>();
            let d_write = (*(*device).dev_ops).d_write;
            let result = match d_write {
                Some(d_write) => {
                    d_write(driver_unit((*device).dev_number), ior)
                }
                None => D_SUCCESS,
            };

            if result == D_IO_QUEUED {
                return c_int::from(false);
            }
        }

        if IpcPort::valid((*ior).reply_port).is_some() {
            let bytes = ((*ior).total - (*ior).residual) as c_int;
            if (*ior).op & IO_INBAND != 0 {
                glue::ds_device_write_reply_inband(
                    (*ior).reply_port,
                    (*ior).reply_port_type,
                    (*ior).error,
                    bytes,
                );
            } else {
                glue::ds_device_write_reply(
                    (*ior).reply_port,
                    (*ior).reply_port_type,
                    (*ior).error,
                    bytes,
                );
            }
        }
        dev_lookup::deallocate((*ior).device.cast::<MachDevice>());
    }

    c_int::from(true)
}

/// `device_read()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, and `data` and
/// `data_count` writable.
unsafe extern "C" fn device_read(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    bytes_wanted: c_int,
    _data: *mut *mut c_char,
    _data_count: *mut c_uint,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        if IpcPort::valid(reply_port).is_none() {
            glue::printf(c"ds_* invalid reply port\n".as_ptr());
            glue::SoftDebugger(c"ds_* reply_port".as_ptr());
            return MIG_NO_REPLY;
        }

        let ior = io_req_alloc();
        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_READ | IO_CALL;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data = ptr::null_mut();
        (*ior).count = c_long::from(bytes_wanted);
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_read_done);
        (*ior).reply_port = reply_port;
        (*ior).reply_port_type = reply_port_type;

        dev_lookup::reference(device);

        let d_read = (*(*device).dev_ops).d_read;
        let result = match d_read {
            Some(d_read) => d_read(driver_unit((*device).dev_number), ior),
            None => D_SUCCESS,
        };

        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        (*ior).error = result;
        ds_read_done(ior);
        io_req_free(ior);
    }

    MIG_NO_REPLY
}

/// `device_read_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, and `data` and
/// `data_count` writable.
unsafe extern "C" fn device_read_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    bytes_wanted: c_int,
    _data: *mut c_char,
    _data_count: *mut c_uint,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        if IpcPort::valid(reply_port).is_none() {
            glue::printf(c"ds_* invalid reply port\n".as_ptr());
            glue::SoftDebugger(c"ds_* reply_port".as_ptr());
            return MIG_NO_REPLY;
        }

        let ior = io_req_alloc();
        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_READ | IO_CALL | IO_INBAND;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data = ptr::null_mut();
        // The C compared the int against the `size_t` array bound, so a
        // negative wish became a huge one and picked the bound.
        let wanted = bytes_wanted as usize;
        (*ior).count = if wanted < IO_INBAND_MAX {
            c_long::from(bytes_wanted)
        } else {
            IO_INBAND_MAX as c_long
        };
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_read_done);
        (*ior).reply_port = reply_port;
        (*ior).reply_port_type = reply_port_type;

        dev_lookup::reference(device);

        let d_read = (*(*device).dev_ops).d_read;
        let result = match d_read {
            Some(d_read) => d_read(driver_unit((*device).dev_number), ior),
            None => D_SUCCESS,
        };

        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        (*ior).error = result;
        ds_read_done(ior);
        io_req_free(ior);
    }

    MIG_NO_REPLY
}

/// `device_read_alloc()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be a live request with `io_count` bytes to allocate.
pub(crate) unsafe extern "C" fn device_read_alloc(
    ior: *mut IoReq,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        if (*ior).count == 0 {
            return KERN_SUCCESS;
        }

        if (*ior).op & IO_INBAND != 0 {
            let addr = kmem_cache_alloc(ptr::addr_of_mut!(IO_INBAND_CACHE));
            (*ior).data = ptr::with_exposed_provenance_mut::<c_char>(addr);
            (*ior).alloc_size = IO_INBAND_MAX;
        } else {
            let size = round_page(size);
            let mut addr: VmOffset = 0;
            let kr = glue::kmem_alloc(
                glue::kernel_map.cast::<VmMap>(),
                &mut addr,
                size,
            );
            if kr != KERN_SUCCESS {
                return kr;
            }

            (*ior).data = ptr::with_exposed_provenance_mut::<c_char>(addr);
            (*ior).alloc_size = size;
        }

        KERN_SUCCESS
    }
}

/// `ds_read_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be the live read request [`device_read()`] or
/// [`device_read_inband()`] queued.
pub(crate) unsafe extern "C" fn ds_read_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        let inband = (*ior).op & IO_INBAND != 0;
        let size_read = if (*ior).error != 0 {
            0
        } else {
            (*ior).count.wrapping_sub((*ior).residual) as VmSize
        };

        let start_data = (*ior).data.addr();
        let end_data = start_data.wrapping_add(size_read);
        let start_sent = if inband {
            start_data
        } else {
            trunc_page(start_data)
        };
        let end_sent = if inband {
            start_data.wrapping_add((*ior).alloc_size)
        } else {
            round_page(end_data)
        };

        if start_sent < start_data {
            ptr::write_bytes(
                start_sent as *mut u8,
                0,
                start_data - start_sent,
            );
        }
        if end_sent > end_data {
            ptr::write_bytes(end_data as *mut u8, 0, end_sent - end_data);
        }

        let mut touch = start_sent;
        while touch < end_sent {
            let byte = ptr::read_volatile(touch as *const u8);
            ptr::write_volatile(touch as *mut u8, byte);
            touch = touch.wrapping_add(PAGE_SIZE);
        }

        if inband {
            glue::ds_device_read_reply_inband(
                (*ior).reply_port,
                (*ior).reply_port_type,
                (*ior).error,
                (*ior).data,
                size_read as c_uint,
            );
        } else {
            let mut copy: *mut VmMapCopy = ptr::null_mut();
            let kr = crate::vm::vm_map_ffi::vm_map_copyin_page_list(
                glue::kernel_map.cast::<VmMap>(),
                start_data,
                size_read,
                1,
                1,
                &mut copy,
                0,
            );
            if kr != KERN_SUCCESS {
                // SAFETY: `Panic` does not return; the tags are the C
                // `panic()` string.
                glue::Panic(
                    c"device/ds_routines.c".as_ptr(),
                    line!() as c_int,
                    c"ds_read_done".as_ptr(),
                    c"read_done: vm_map_copyin_page_list failed".as_ptr(),
                );
            }

            glue::ds_device_read_reply(
                (*ior).reply_port,
                (*ior).reply_port_type,
                (*ior).error,
                copy.cast::<c_char>(),
                size_read as c_uint,
            );
        }

        if (*ior).count != 0 {
            if inband {
                if (*ior).alloc_size > 0 {
                    kmem_cache_free(
                        ptr::addr_of_mut!(IO_INBAND_CACHE),
                        (*ior).data.addr(),
                    );
                }
            } else {
                let end_alloc =
                    start_sent.wrapping_add(round_page((*ior).alloc_size));
                if end_alloc > end_sent {
                    vm_deallocate(
                        glue::kernel_map.cast::<VmMap>(),
                        end_sent,
                        end_alloc - end_sent,
                    );
                }
            }
        }

        dev_lookup::deallocate((*ior).device.cast::<MachDevice>());
    }

    c_int::from(true)
}

/// `device_set_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device.
unsafe extern "C" fn device_set_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: c_uint,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        let d_setstat = (*(*device).dev_ops).d_setstat;
        match d_setstat {
            Some(d_setstat) => d_setstat(
                driver_unit((*device).dev_number),
                flavor,
                status,
                status_count,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `mach_device_get_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device.
unsafe extern "C" fn mach_device_get_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: *mut c_uint,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        let d_getstat = (*(*device).dev_ops).d_getstat;
        match d_getstat {
            Some(d_getstat) => d_getstat(
                driver_unit((*device).dev_number),
                flavor,
                status,
                status_count,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `device_set_filter()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, and `filter`
/// readable for `filter_count` entries.
unsafe extern "C" fn device_set_filter(
    dev: *mut c_void,
    receive_port: *mut c_void,
    priority: c_int,
    filter: *mut c_ushort,
    filter_count: c_uint,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        if IpcPort::valid(receive_port).is_none() {
            return Err(DeviceError::InvalidOperation).as_io_return();
        }

        let d_async_in = (*(*device).dev_ops).d_async_in;
        match d_async_in {
            Some(d_async_in) => d_async_in(
                driver_unit((*device).dev_number),
                receive_port,
                priority,
                filter,
                filter_count,
            ),
            None => Err(DeviceError::InvalidOperation).as_io_return(),
        }
    }
}

/// `device_map()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` must be the emulation data of a live mach device, and `pager`
/// writable.
unsafe extern "C" fn device_map(
    dev: *mut c_void,
    protection: c_int,
    offset: VmOffset,
    // The C's `device_pager_setup()` stored the size and never read it
    // back, so the core does not take it.
    _size: VmSize,
    pager: *mut *mut c_void,
    _unmap: c_int,
) -> c_int {
    let device = dev.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if protection & !VmProt::ALL.bits() != 0 {
            return KERN_INVALID_ARGUMENT;
        }

        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        match crate::device::dev_pager::setup(device, protection, offset) {
            Ok(port) => {
                *pager = port.as_ptr();
                KERN_SUCCESS
            }
            Err(error) => error.code(),
        }
    }
}

/// `ds_no_senders()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `notification` must be a live no-senders notification.
unsafe extern "C" fn ds_no_senders(notification: *mut c_void) {
    let notification = notification.cast::<NoSendersNotification>();
    // SAFETY: the caller promises the live notification.
    unsafe {
        glue::printf(
            c"ds_no_senders called! device_port=0x%zx count=%d\n".as_ptr(),
            (*notification).header.remote(),
            (*notification).not_count,
        );
    }
}

/// `iodone()` of <device/io_req.h>.
///
/// # Safety
///
/// `ior` must be a live request whose completion path is not already running.
pub(crate) unsafe extern "C" fn iodone(ior: *mut IoReq) {
    // SAFETY: the caller promises the live request.
    unsafe {
        if (*ior).op & IO_LOANED != 0 {
            if let Some(done) = (*ior).done {
                done(ior);
            }
            return;
        }

        let s = glue::splsched();
        if (*ior).op & IO_CALL == 0 {
            (*ior).lock.lock();
            (*ior).op |= IO_DONE;
            (*ior).op &= !IO_WANTED;
            (*ior).lock.unlock();
            thread_wakeup_prim(ior.cast::<c_void>(), 0, THREAD_AWAKENED);
        } else {
            (*ior).op |= IO_DONE;
            {
                let _guard = IO_DONE_LIST_LOCK.lock();
                let mut head = QueueEntry::pin_in_place(
                    NonNull::new_unchecked(ptr::addr_of_mut!(IO_DONE_LIST)),
                );
                let entry = QueueEntry::pin_in_place(NonNull::new_unchecked(
                    ior.cast::<QueueEntry>(),
                ));
                head.as_mut().push_back(entry);
                thread_wakeup_prim(
                    ptr::addr_of_mut!(IO_DONE_LIST).cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
        }
        glue::splx(s);
    }
}

/// `io_done_thread_continue()` of `device/ds_routines.c`.
unsafe extern "C" fn io_done_thread_continue() {
    loop {
        // SAFETY: the interrupt level and the list lock serialize the list
        // against `iodone()`.
        let mut s = unsafe { glue::splhigh() };
        loop {
            let guard = IO_DONE_LIST_LOCK.lock();
            // SAFETY: the list is this module's static and the lock is held.
            let popped = unsafe {
                QueueEntry::pin_in_place(NonNull::new_unchecked(
                    ptr::addr_of_mut!(IO_DONE_LIST),
                ))
                .as_mut()
                .pop_front()
            };
            match popped {
                None => {
                    // SAFETY: the event is the list head every wakeup names,
                    // and the lock is held.
                    unsafe {
                        assert_wait(
                            ptr::addr_of_mut!(IO_DONE_LIST).cast::<c_void>(),
                            0,
                        );
                    }
                    drop(guard);
                    // SAFETY: `s` is this iteration's `splhigh()`.
                    unsafe { glue::splx(s) };
                    break;
                }
                Some(entry) => {
                    drop(guard);
                    // SAFETY: `s` is this iteration's `splhigh()`.
                    unsafe { glue::splx(s) };
                    let ior = entry.as_ptr().cast::<IoReq>();
                    // SAFETY: every list entry is a live request.
                    let finished = unsafe {
                        match (*ior).done {
                            Some(done) => done(ior),
                            None => c_int::from(true),
                        }
                    };
                    if finished != 0 {
                        // SAFETY: the completion released the request.
                        unsafe { io_req_free(ior) };
                    }
                    s = unsafe { glue::splhigh() };
                }
            }
        }
        // SAFETY: the caller runs this as the io-done kernel thread; the
        // continuation is this routine.
        unsafe { thread_block(Some(io_done_thread_continue)) };
    }
}

/// `io_done_thread()` of `device/ds_routines.c`.
///
/// # Safety
///
/// Runs only as the io-done kernel thread.
pub(crate) unsafe extern "C" fn io_done_thread() {
    // SAFETY: the running thread is the io-done thread.
    unsafe {
        let thread = current_thread();
        (*thread).vm_privilege = 1;
        stack_privilege(thread);
        thread_set_own_priority(0);

        io_done_thread_continue();
    }
}

/// `mach_device_init()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during
/// boot before any device can be opened.
pub(crate) unsafe extern "C" fn mach_device_init() {
    let mut device_io_min: VmOffset = 0;
    let mut device_io_max: VmOffset = 0;

    // SAFETY: the list is not linked yet, and this boot step is its only
    // initializer.
    unsafe {
        QueueEntry::pin_in_place(NonNull::new_unchecked(ptr::addr_of_mut!(
            IO_DONE_LIST
        )))
        .as_mut()
        .init_head();
    }

    // SAFETY: the map storage is this module's static, the kernel map is the
    // live boot map, and both out-pointers are live locals.
    unsafe {
        kmem_submap(
            device_io_map,
            glue::kernel_map.cast::<VmMap>(),
            &mut device_io_min,
            &mut device_io_max,
            DEVICE_IO_MAP_SIZE,
        );
    }

    // SAFETY: this boot step is the map's only writer.
    unsafe {
        (*device_io_map).flags |= VM_MAP_WAIT_FOR_SPACE;
    }

    // SAFETY: the caches are this module's statics, unshared during boot.
    unsafe {
        kmem_cache_init(
            ptr::addr_of_mut!(IO_INBAND_CACHE),
            c"io_buf_ptr_inband".as_ptr(),
            IO_INBAND_MAX,
            0,
            None,
            0,
        );
    }
    mach_device_trap_init();
}

/// `iowait()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be a live request whose `io_done` callback was not `IO_CALL`.
pub(crate) unsafe extern "C" fn iowait(ior: *mut IoReq) {
    // SAFETY: the caller promises the live request, and the interrupt level
    // plus the request lock serialize its completion.
    unsafe {
        let s = glue::splsched();
        (*ior).lock.lock();
        loop {
            if (*ior).op & IO_DONE != 0 {
                break;
            }
            assert_wait(ior.cast::<c_void>(), 0);
            (*ior).lock.unlock();
            thread_block(None);
            (*ior).lock.lock();
        }
        (*ior).lock.unlock();
        glue::splx(s);
    }
}

/// `mach_device_trap_init()` of `device/ds_routines.c`.
fn mach_device_trap_init() {
    // SAFETY: the cache is this module's static, unshared during boot.
    unsafe {
        kmem_cache_init(
            ptr::addr_of_mut!(IO_TRAP_CACHE),
            c"io_req".as_ptr(),
            IOTRAP_REQSIZE,
            0,
            None,
            0,
        );
    }
}

/// `ds_trap_req_alloc()` of `device/ds_routines.c`.
///
/// # Safety
///
/// The `io_trap_cache` must be initialized.
unsafe fn ds_trap_req_alloc(
    _device: *mut MachDevice,
    _data_size: VmSize,
) -> *mut IoReq {
    // SAFETY: the caller promises the initialized cache.
    let addr = unsafe { kmem_cache_alloc(ptr::addr_of_mut!(IO_TRAP_CACHE)) };
    ptr::with_exposed_provenance_mut::<IoReq>(addr)
}

/// `ds_trap_write_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` must be the live trap request [`device_write_trap()`] built.
unsafe extern "C" fn ds_trap_write_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    unsafe {
        let dev = (*ior).device;

        kmem_cache_free(ptr::addr_of_mut!(IO_TRAP_CACHE), ior.addr());
        dev_lookup::deallocate(dev.cast::<MachDevice>());
    }

    c_int::from(true)
}

/// `device_write_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `device` must be the emulation data of a live mach device, and `data`
/// readable for `data_count` bytes of user memory.
unsafe extern "C" fn device_write_trap(
    device: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    data: c_ulong,
    data_count: c_ulong,
) -> c_int {
    let device = device.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        let ior = ds_trap_req_alloc(device, data_count as VmSize);

        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_WRITE | IO_CALL | IO_LOANED;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data =
            ior.cast::<u8>().add(size_of::<IoReq>()).cast::<c_char>();
        (*ior).count = data_count as c_long;
        (*ior).total = data_count as c_long;
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_trap_write_done);
        (*ior).reply_port = ptr::null_mut();
        (*ior).reply_port_type = 0;

        if data_count > 0 {
            glue::copyin(
                ptr::with_exposed_provenance::<c_void>(data as usize),
                (*ior).data.cast::<c_void>(),
                data_count as usize,
            );
        }

        dev_lookup::reference(device);

        let d_write = (*(*device).dev_ops).d_write;
        let result = match d_write {
            Some(d_write) => d_write(driver_unit((*device).dev_number), ior),
            None => D_SUCCESS,
        };

        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        dev_lookup::deallocate(device);

        kmem_cache_free(ptr::addr_of_mut!(IO_TRAP_CACHE), ior.addr());
        result
    }
}

/// `device_writev_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `device` must be the emulation data of a live mach device, and `iovec`
/// readable for `iocount` user-space entries.
unsafe extern "C" fn device_writev_trap(
    device: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    iovec: *mut RpcIoBufVec,
    iocount: c_ulong,
) -> c_int {
    let device = device.cast::<MachDevice>();
    // SAFETY: the caller promises the live mach device.
    unsafe {
        if (*device).state != DEV_STATE_OPEN {
            return Err(DeviceError::NoSuchDevice).as_io_return();
        }

        if iocount > MAX_IOVECS as c_ulong {
            return KERN_INVALID_VALUE;
        }
        let iocount = iocount as usize;

        let mut stack_iovec = [IoBufVec { data: 0, count: 0 }; MAX_IOVECS];
        let mut data_count: VmSize = 0;
        for (i, slot) in stack_iovec.iter_mut().enumerate().take(iocount) {
            let mut riov = RpcIoBufVec { data: 0, count: 0 };
            let kr = glue::copyin(
                iovec.add(i).cast::<c_void>(),
                ptr::addr_of_mut!(riov).cast::<c_void>(),
                size_of::<RpcIoBufVec>(),
            );
            if kr != 0 {
                return KERN_INVALID_ARGUMENT;
            }
            *slot = IoBufVec {
                data: riov.data as VmOffset,
                count: riov.count as VmSize,
            };
            data_count = data_count.wrapping_add(slot.count);
        }

        let ior = ds_trap_req_alloc(device, data_count);

        (*ior).device = device.cast::<c_void>();
        (*ior).unit = (*device).dev_number;
        (*ior).op = IO_WRITE | IO_CALL | IO_LOANED;
        (*ior).mode = mode;
        (*ior).recnum = recnum;
        (*ior).data =
            ior.cast::<u8>().add(size_of::<IoReq>()).cast::<c_char>();
        (*ior).count = data_count as c_long;
        (*ior).total = data_count as c_long;
        (*ior).alloc_size = 0;
        (*ior).residual = 0;
        (*ior).error = 0;
        (*ior).done = Some(ds_trap_write_done);
        (*ior).reply_port = ptr::null_mut();
        (*ior).reply_port_type = 0;

        if data_count > 0 {
            let mut p = (*ior).data;
            for iovec in stack_iovec.iter().take(iocount) {
                glue::copyin(
                    ptr::with_exposed_provenance::<c_void>(iovec.data),
                    p.cast::<c_void>(),
                    iovec.count,
                );
                p = p.add(iovec.count);
            }
        }

        dev_lookup::reference(device);

        let d_write = (*(*device).dev_ops).d_write;
        let result = match d_write {
            Some(d_write) => d_write(driver_unit((*device).dev_number), ior),
            None => D_SUCCESS,
        };

        if result == D_IO_QUEUED {
            return MIG_NO_REPLY;
        }

        dev_lookup::deallocate(device);

        kmem_cache_free(ptr::addr_of_mut!(IO_TRAP_CACHE), ior.addr());
        result
    }
}
