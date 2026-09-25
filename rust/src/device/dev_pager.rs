// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_pager.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device pager, which `device/dev_pager.c` used to define and
//! <device/dev_pager.h> used to declare.
//!
//! The C record's `client_count`, `pager_name` and `size` fields were
//! written and never read, so the Rust record does not carry them; its
//! reference count is an atomic where the C held a per-record lock.

use crate::arch::i386::io_req::DevT;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SHIFT;
use crate::device::dev_lookup;
use crate::device::dev_name_ffi::nomap;
use crate::device::ds_routines::{MachDevice, driver_unit};
use crate::device::r#return::DeviceError;
use crate::glue;
use crate::ipc::{IpcPort, ipc_port, ipc_space};
use crate::kern::queue::{
    QueueEntry, queue_end, queue_enter_tail, queue_first, queue_init,
    queue_next, queue_remove_generic,
};
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::spin::Mutex;
use crate::vm::error::Error;
use crate::vm::vm_object;
use core::ffi::{CStr, c_int, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicI32, Ordering};

/// `DEV_HASH_COUNT` of `device/dev_pager.c`: the number of buckets in both
/// tables.
const DEV_HASH_COUNT: usize = 127;

/// `KERN_RESOURCE_SHORTAGE` of <mach/kern_return.h>.
const KERN_RESOURCE_SHORTAGE: c_int = 6;

/// `MEMORY_OBJECT_COPY_NONE` of <mach/memory_object.h>.
const MEMORY_OBJECT_COPY_NONE: c_int = 0;

/// `device_pager_debug` of `device/dev_pager.c`: the switch the C checked
/// before its two trace prints, kept for a debugger to set.
#[unsafe(no_mangle)]
pub static device_pager_debug: AtomicI32 = AtomicI32::new(0);

const _: () = assert!(size_of::<AtomicI32>() == size_of::<c_int>());
const _: () = assert!(align_of::<AtomicI32>() == align_of::<c_int>());

/// One device pager record, the C `struct dev_pager`.
struct DevPager {
    ref_count: AtomicI32,
    pager: IpcPort,
    pager_request: Option<IpcPort>,
    device: *mut MachDevice,
    offset: VmOffset,
    prot: c_int,
}

/// The failure `device_pager_setup()` reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupError {
    /// The `D_INVALID_OPERATION` of a device whose driver cannot map.
    InvalidOperation,
    /// The `KERN_RESOURCE_SHORTAGE` of a failed record or port allocation.
    ResourceShortage,
}

impl SetupError {
    /// The code the C returned for this failure: the `D_*` code goes to the
    /// device map caller, the `kern_return_t` to `device_pager_setup()`.
    pub(crate) const fn code(self) -> c_int {
        match self {
            Self::InvalidOperation => DeviceError::InvalidOperation as i32,
            Self::ResourceShortage => KERN_RESOURCE_SHORTAGE,
        }
    }
}

/// `dev_pager_cache` of `device/dev_pager.c`: the `struct dev_pager` slab
/// cache.
static mut DEV_PAGER_CACHE: KmemCache = KmemCache::zeroed();

/// `dev_pager_hashtable` of `device/dev_pager.c`: one bucket per
/// `dev_hash()` result, keyed by the pager port.
static mut DEV_PAGER_HASHTABLE: [QueueEntry; DEV_HASH_COUNT] =
    [const { QueueEntry::unlinked() }; DEV_HASH_COUNT];

/// `dev_pager_hash_lock`: serializes the port-name table.
static DEV_PAGER_HASH_LOCK: Mutex<()> = Mutex::new(());

/// `dev_pager_hash_cache`: the `struct dev_pager_entry` slab cache.
static mut DEV_PAGER_HASH_CACHE: KmemCache = KmemCache::zeroed();

/// One entry of `dev_pager_hashtable`, the C `struct dev_pager_entry`.
struct DevPagerEntry {
    links: QueueEntry,
    name: Option<IpcPort>,
    pager: NonNull<DevPager>,
}

/// `dev_device_hashtable` of `device/dev_pager.c`: one bucket per
/// `dev_hash()` result, keyed by device and offset.
static mut DEV_DEVICE_HASHTABLE: [QueueEntry; DEV_HASH_COUNT] =
    [const { QueueEntry::unlinked() }; DEV_HASH_COUNT];

/// `dev_device_hash_lock`: serializes the device-and-offset table.
static DEV_DEVICE_HASH_LOCK: Mutex<()> = Mutex::new(());

/// `dev_device_hash_cache`: the `struct dev_device_entry` slab cache.
static mut DEV_DEVICE_HASH_CACHE: KmemCache = KmemCache::zeroed();

/// One entry of `dev_device_hashtable`, the C `struct dev_device_entry`.
struct DevDeviceEntry {
    links: QueueEntry,
    device: *mut MachDevice,
    offset: VmOffset,
    pager: NonNull<DevPager>,
}

/// Halt the way the C `panic()` of `device/dev_pager.c` did.
#[track_caller]
fn die(fun: &'static CStr, message: &'static CStr) -> ! {
    let location = core::panic::Location::caller();
    // SAFETY: `Panic` does not return; the file, function and message are
    // this module's, and the caller's line fits the `c_int` the format
    // takes.
    unsafe {
        glue::Panic(
            c"device/dev_pager.c".as_ptr(),
            location.line() as c_int,
            fun.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// `dev_hash()` of `device/dev_pager.c`: the C masked the low 24 bits of the
/// whole value, a pointer or a pointer plus offset, and reduced it modulo the
/// bucket count.
const fn dev_hash(value: usize) -> usize {
    (value & 0xffffff) % DEV_HASH_COUNT
}

/// The port-name bucket the C's `dev_hash()` selected.
fn pager_bucket(name: Option<IpcPort>) -> *mut QueueEntry {
    let value = name.map_or(0, |port| port.as_ptr().addr());
    // SAFETY: `dev_hash()` is below the array bound, so the element is in
    // bounds.
    unsafe {
        ptr::addr_of_mut!(DEV_PAGER_HASHTABLE)
            .cast::<QueueEntry>()
            .add(dev_hash(value))
    }
}

/// The device-and-offset bucket the C's `dev_hash()` selected.
fn device_bucket(
    device: *mut MachDevice,
    offset: VmOffset,
) -> *mut QueueEntry {
    let value = device.addr().wrapping_add(offset);
    // SAFETY: `dev_hash()` is below the array bound, so the element is in
    // bounds.
    unsafe {
        ptr::addr_of_mut!(DEV_DEVICE_HASHTABLE)
            .cast::<QueueEntry>()
            .add(dev_hash(value))
    }
}

/// The first entry of `head` whose name is `name`.
///
/// # Safety
///
/// `head` must be an initialized bucket of the port-name table, and the
/// caller must hold that table's lock.
unsafe fn find_pager_entry(
    head: *mut QueueEntry,
    name: Option<IpcPort>,
) -> Option<NonNull<DevPagerEntry>> {
    // SAFETY: the caller promises the initialized bucket, and the queue
    // invariant keeps `entry` on a live entry.
    unsafe {
        let mut entry = queue_first(head).cast::<DevPagerEntry>();
        while queue_end(head, entry.cast()) == 0 {
            if (*entry).name == name {
                return NonNull::new(entry);
            }
            entry = queue_next(ptr::addr_of_mut!((*entry).links))
                .cast::<DevPagerEntry>();
        }
        None
    }
}

/// The first entry of `head` for `device` and `offset`.
///
/// # Safety
///
/// `head` must be an initialized bucket of the device table, and the caller
/// must hold that table's lock.
unsafe fn find_device_entry(
    head: *mut QueueEntry,
    device: *mut MachDevice,
    offset: VmOffset,
) -> Option<NonNull<DevDeviceEntry>> {
    // SAFETY: the caller promises the initialized bucket, and the queue
    // invariant keeps `entry` on a live entry.
    unsafe {
        let mut entry = queue_first(head).cast::<DevDeviceEntry>();
        while queue_end(head, entry.cast()) == 0 {
            if (*entry).device == device && (*entry).offset == offset {
                return NonNull::new(entry);
            }
            entry = queue_next(ptr::addr_of_mut!((*entry).links))
                .cast::<DevDeviceEntry>();
        }
        None
    }
}

/// Take a reference on `rec`.
///
/// # Safety
///
/// `rec` must be a live record whose count is nonzero.
unsafe fn reference(rec: NonNull<DevPager>) -> NonNull<DevPager> {
    // The increment is `Relaxed`: the caller already holds a reference, so
    // no other thread can free the record, and the count needs atomicity
    // rather than publication.
    // SAFETY: the caller promises the live record.
    unsafe { (*rec.as_ptr()).ref_count.fetch_add(1, Ordering::Relaxed) };
    rec
}

/// Drop a reference on `rec`, freeing the record when it was the last.
///
/// # Safety
///
/// `rec` must be a live record this call owns a reference on, and nothing
/// else may access it once the count reaches zero.
unsafe fn deallocate(rec: NonNull<DevPager>) {
    // `AcqRel` on the decrement releases this thread's writes and acquires
    // the other releases, so the free below sees a complete record.
    // SAFETY: the caller promises the live, owned record.
    let last =
        unsafe { (*rec.as_ptr()).ref_count.fetch_sub(1, Ordering::AcqRel) }
            == 1;
    if last {
        // SAFETY: no reference remains, so nothing else can reach the
        // record's cache object.
        unsafe {
            (*ptr::addr_of_mut!(DEV_PAGER_CACHE)).free(rec.cast::<u8>());
        }
    }
}

/// `dev_pager_hash_insert()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The package must be initialized, `rec` must be a live record, and nothing
/// may hold the port-name lock.
unsafe fn pager_hash_insert(name: Option<IpcPort>, rec: NonNull<DevPager>) {
    // SAFETY: the cache is live after `init()`, and the fresh buffer is
    // linked nowhere.
    let Some(buf) =
        (unsafe { (*ptr::addr_of_mut!(DEV_PAGER_HASH_CACHE)).alloc() })
    else {
        die(
            c"dev_pager_hash_insert",
            c"dev_pager_hash_insert: no memory",
        );
    };
    let entry = buf.as_ptr().cast::<DevPagerEntry>();
    // SAFETY: the cache object holds a whole entry; the queue links are
    // written by the insertion below.
    unsafe {
        (*entry).name = name;
        (*entry).pager = rec;
    }

    let _guard = DEV_PAGER_HASH_LOCK.lock();
    // SAFETY: the bucket is an initialized head, and the entry is a stable,
    // unlinked cache object.
    unsafe {
        queue_enter_tail(
            pager_bucket(name),
            entry.cast::<c_void>(),
            offset_of!(DevPagerEntry, links),
        );
    }
}

/// `dev_pager_hash_delete()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The package must be initialized, and only the C protocol's own entry for
/// `name` may exist.
unsafe fn pager_hash_delete(name: Option<IpcPort>) {
    let head = pager_bucket(name);
    let found = {
        let _guard = DEV_PAGER_HASH_LOCK.lock();
        // SAFETY: the bucket is initialized and the lock is held.
        let entry = unsafe { find_pager_entry(head, name) };
        if let Some(entry) = entry {
            // SAFETY: `entry` is linked into `head`, and the lock is held.
            unsafe {
                queue_remove_generic(
                    head,
                    entry.as_ptr().cast::<c_void>(),
                    offset_of!(DevPagerEntry, links),
                );
            }
        }
        entry
    };

    let Some(entry) = found else {
        return;
    };
    // SAFETY: the entry was just unlinked and owns its cache object.
    unsafe {
        (*ptr::addr_of_mut!(DEV_PAGER_HASH_CACHE)).free(entry.cast::<u8>());
    }
}

/// `dev_pager_hash_lookup()` of `device/dev_pager.c`: the record an entry
/// names, with a reference taken on it.
///
/// # Safety
///
/// The package must be initialized.
unsafe fn pager_hash_lookup(
    name: Option<IpcPort>,
) -> Option<NonNull<DevPager>> {
    let head = pager_bucket(name);
    let _guard = DEV_PAGER_HASH_LOCK.lock();
    // SAFETY: the bucket is initialized and the lock is held.
    let entry = unsafe { find_pager_entry(head, name) }?;
    // SAFETY: the table holds the record's initial reference, and the entry
    // keeps the record alive while the lock is held.
    Some(unsafe { reference((*entry.as_ptr()).pager) })
}

/// `dev_device_hash_insert()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The package must be initialized, `rec` must be a live record, and nothing
/// may hold the device lock.
unsafe fn device_hash_insert(
    device: *mut MachDevice,
    offset: VmOffset,
    rec: NonNull<DevPager>,
) {
    // SAFETY: the cache is live after `init()`, and the fresh buffer is
    // linked nowhere.
    let Some(buf) =
        (unsafe { (*ptr::addr_of_mut!(DEV_DEVICE_HASH_CACHE)).alloc() })
    else {
        die(
            c"dev_device_hash_insert",
            c"dev_device_hash_insert: no memory",
        );
    };
    let entry = buf.as_ptr().cast::<DevDeviceEntry>();
    // SAFETY: the cache object holds a whole entry; the queue links are
    // written by the insertion below.
    unsafe {
        (*entry).device = device;
        (*entry).offset = offset;
        (*entry).pager = rec;
    }

    let _guard = DEV_DEVICE_HASH_LOCK.lock();
    // SAFETY: the bucket is an initialized head, and the entry is a stable,
    // unlinked cache object.
    unsafe {
        queue_enter_tail(
            device_bucket(device, offset),
            entry.cast::<c_void>(),
            offset_of!(DevDeviceEntry, links),
        );
    }
}

/// `dev_device_hash_delete()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The package must be initialized, and only the C protocol's own entry for
/// `device` and `offset` may exist.
unsafe fn device_hash_delete(device: *mut MachDevice, offset: VmOffset) {
    let head = device_bucket(device, offset);
    let found = {
        let _guard = DEV_DEVICE_HASH_LOCK.lock();
        // SAFETY: the bucket is initialized and the lock is held.
        let entry = unsafe { find_device_entry(head, device, offset) };
        if let Some(entry) = entry {
            // SAFETY: `entry` is linked into `head`, and the lock is held.
            unsafe {
                queue_remove_generic(
                    head,
                    entry.as_ptr().cast::<c_void>(),
                    offset_of!(DevDeviceEntry, links),
                );
            }
        }
        entry
    };

    let Some(entry) = found else {
        return;
    };
    // SAFETY: the entry was just unlinked and owns its cache object.
    unsafe {
        (*ptr::addr_of_mut!(DEV_DEVICE_HASH_CACHE)).free(entry.cast::<u8>());
    }
}

/// `dev_device_hash_lookup()` of `device/dev_pager.c`: the record an entry
/// names, with a reference taken on it.
///
/// # Safety
///
/// The package must be initialized.
unsafe fn device_hash_lookup(
    device: *mut MachDevice,
    offset: VmOffset,
) -> Option<NonNull<DevPager>> {
    let head = device_bucket(device, offset);
    let _guard = DEV_DEVICE_HASH_LOCK.lock();
    // SAFETY: the bucket is initialized and the lock is held.
    let entry = unsafe { find_device_entry(head, device, offset) }?;
    // SAFETY: the table holds the record's initial reference, and the entry
    // keeps the record alive while the lock is held.
    Some(unsafe { reference((*entry.as_ptr()).pager) })
}

/// `device_map_page()` of `device/dev_pager.c`, the callback
/// `vm_object_page_map()` calls.
///
/// # Safety
///
/// `dsp` must be the live record [`data_request()`] passes.
pub(crate) unsafe extern "C" fn device_map_page(
    dsp: *mut c_void,
    offset: VmOffset,
) -> VmOffset {
    let Some(rec) = NonNull::new(dsp.cast::<DevPager>()) else {
        // SAFETY: the fictitious address is a read-only C global.
        return unsafe { glue::vm_page_fictitious_addr };
    };

    // SAFETY: the caller promises the live record, whose device reference
    // keeps `dev_ops` alive.
    unsafe {
        let record = &*rec.as_ptr();
        let device = &*record.device;
        let Some(d_mmap) = (*device.dev_ops).d_mmap else {
            return glue::vm_page_fictitious_addr;
        };
        let pagenum = d_mmap(
            driver_unit(device.dev_number),
            record.offset.wrapping_add(offset),
            record.prot,
        );
        if pagenum == VmOffset::MAX {
            return glue::vm_page_fictitious_addr;
        }
        // `pmap_phys_address(frame)` of <i386/intel/pmap.h> is the
        // `intel_ptob()` shift of the frame cast to `phys_addr_t`; the
        // shift drops what does not fit, as the C's did.
        pagenum.wrapping_shl(PAGE_SHIFT)
    }
}

/// `device_pager_setup()` of `device/dev_pager.c`.
///
/// # Safety
///
/// `device` must be a live, referenced mach device whose `dev_ops` is live,
/// and the package must be initialized.
pub(crate) unsafe fn setup(
    device: *mut MachDevice,
    prot: c_int,
    offset: VmOffset,
) -> Result<IpcPort, SetupError> {
    // SAFETY: the caller promises the live device.
    let ops = unsafe { (*device).dev_ops };
    if ops.is_null() {
        return Err(SetupError::InvalidOperation);
    }
    // SAFETY: `ops` belongs to the live device.
    let Some(d_mmap) = (unsafe { (*ops).d_mmap }) else {
        return Err(SetupError::InvalidOperation);
    };
    if ptr::fn_addr_eq(
        d_mmap,
        nomap as unsafe extern "C" fn(DevT, VmOffset, c_int) -> VmOffset,
    ) {
        return Err(SetupError::InvalidOperation);
    }

    // SAFETY: the package is initialized, and a found record is referenced
    // by the lookup.
    if let Some(rec) = unsafe { device_hash_lookup(device, offset) } {
        // SAFETY: the record's pager port stays live until termination.
        let port = unsafe { ipc_port::make_send((*rec.as_ptr()).pager) };
        // SAFETY: this drops the reference the lookup took.
        unsafe { deallocate(rec) };
        return Ok(port);
    }

    // SAFETY: the cache is live after `init()`, and the fresh buffer is
    // unshared.
    let Some(buf) = (unsafe { (*ptr::addr_of_mut!(DEV_PAGER_CACHE)).alloc() })
    else {
        return Err(SetupError::ResourceShortage);
    };
    let rec = buf.as_ptr().cast::<DevPager>();
    // SAFETY: the kernel space is live and the port cache is initialized,
    // as the C's `ipc_port_alloc_kernel()` required.
    let Some(pager) =
        (unsafe { ipc_port::alloc_special(ipc_space::kernel()) })
    else {
        // SAFETY: the fresh cache object owns nothing yet.
        unsafe {
            (*ptr::addr_of_mut!(DEV_PAGER_CACHE))
                .free(NonNull::new_unchecked(rec.cast::<u8>()));
        }
        return Err(SetupError::ResourceShortage);
    };

    // SAFETY: the cache object is uninitialized and owned here; the device
    // reference is taken before the record becomes reachable.
    unsafe {
        ptr::write(
            rec,
            DevPager {
                ref_count: AtomicI32::new(1),
                pager,
                pager_request: None,
                device,
                offset,
                prot,
            },
        );
        dev_lookup::reference(device);
        let rec = NonNull::new_unchecked(rec);
        pager_hash_insert(Some(pager), rec);
        device_hash_insert(device, offset, rec);
    }
    Ok(pager)
}

/// `device_pager_data_request()` of `device/dev_pager.c`.
///
/// # Safety
///
/// `pager` must be the live port of a set-up pager record, `pager_request`
/// the live control port the kernel bound to it, and the package must be
/// initialized.
pub(crate) unsafe fn data_request(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    offset: VmOffset,
    length: VmSize,
) {
    if device_pager_debug.load(Ordering::Relaxed) != 0 {
        // The C printed `vm_offset_t` through `%lx`; the type has the same
        // width as `c_ulong` on both builds.
        // SAFETY: `printf` is the kernel's, and the format matches the
        // arguments.
        unsafe {
            glue::printf(
                c"(device_pager)data_request: pager=%p, offset=0x%lx, length=0x%lx\n"
                    .as_ptr(),
                pager.map_or(ptr::null_mut(), IpcPort::as_ptr),
                offset as c_ulong,
                length as c_ulong,
            );
        }
    }

    // SAFETY: the package is initialized.
    let Some(rec) = (unsafe { pager_hash_lookup(pager) }) else {
        die(
            c"device_pager_data_request",
            c"(device_pager)data_request: lookup failed",
        );
    };

    // SAFETY: the lookup referenced the record, and the device reference it
    // holds keeps the record's fields live.
    unsafe {
        let record = rec.as_ptr();
        if (*record).pager_request != pager_request {
            die(
                c"device_pager_data_request",
                c"(device_pager)data_request: bad pager_request",
            );
        }

        let control = pager_request.map_or(ptr::null_mut(), IpcPort::as_ptr);
        let Some(object) = vm_object::lookup(control) else {
            let _ = glue::r_memory_object_data_error(
                control,
                offset,
                length,
                Error::Failure.as_kern_return(),
            );
            deallocate(rec);
            return;
        };

        // SAFETY: the object is the live object of the request, and the
        // callback takes the record the lookup referenced.
        let result = vm_object::page_map(
            object.as_ptr(),
            offset,
            length,
            Some(device_map_page),
            record.cast::<c_void>(),
        );
        if let Err(error) = result {
            let _ = glue::r_memory_object_data_error(
                control,
                offset,
                length,
                error.as_kern_return(),
            );
        }
        vm_object::deallocate(object.as_ptr());
        deallocate(rec);
    }
}

/// `device_pager_init_pager()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The MIG server calls this once for the pager `pager` denotes, before any
/// data request, and the package must be initialized.
pub(crate) unsafe fn init_pager(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    pager_name: Option<IpcPort>,
) {
    if device_pager_debug.load(Ordering::Relaxed) != 0 {
        // SAFETY: `printf` is the kernel's, and the format matches the three
        // port pointers.
        unsafe {
            glue::printf(
                c"(device_pager)init: pager=%p, request=%p, name=%p\n"
                    .as_ptr(),
                pager.map_or(ptr::null_mut(), IpcPort::as_ptr),
                pager_request.map_or(ptr::null_mut(), IpcPort::as_ptr),
                pager_name.map_or(ptr::null_mut(), IpcPort::as_ptr),
            );
        }
    }

    // SAFETY: the package is initialized.
    let Some(rec) = (unsafe { pager_hash_lookup(pager) }) else {
        die(
            c"device_pager_init_pager",
            c"(device_pager)init: lookup failed",
        );
    };

    // SAFETY: the lookup referenced the record, and the MIG protocol runs
    // this once before any data request.
    unsafe {
        (*rec.as_ptr()).pager_request = pager_request;
        let _ = glue::r_memory_object_ready(
            pager_request.map_or(ptr::null_mut(), IpcPort::as_ptr),
            c_int::from(false),
            MEMORY_OBJECT_COPY_NONE,
        );
        deallocate(rec);
    }
}

/// `device_pager_terminate()` of `device/dev_pager.c`.
///
/// # Safety
///
/// The MIG server calls this once after a completed init, with the ports of
/// that init, and the package must be initialized.
pub(crate) unsafe fn terminate(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    pager_name: Option<IpcPort>,
) {
    // SAFETY: the package is initialized.
    let Some(rec) = (unsafe { pager_hash_lookup(pager) }) else {
        die(
            c"device_pager_terminate",
            c"(device_pager)terminate: lookup failed",
        );
    };

    let (Some(pager), Some(pager_request), Some(pager_name)) =
        (pager, pager_request, pager_name)
    else {
        die(
            c"device_pager_terminate",
            c"(device_pager)terminate: null port",
        );
    };

    // SAFETY: the lookup's reference keeps the record and its device alive,
    // and the protocol paired this with one `init_pager()`, so the saved
    // send rights and the naked receive rights are live.
    unsafe {
        let record = rec.as_ptr();
        pager_hash_delete(Some((*record).pager));
        device_hash_delete((*record).device, (*record).offset);
        dev_lookup::deallocate((*record).device);

        ipc_port::release_send(pager_request);
        ipc_port::release_send(pager_name);
        ipc_port::release_receive(pager_request);
        ipc_port::release_receive(pager_name);
        ipc_port::dealloc_special(pager);

        deallocate(rec);
        deallocate(rec);
    }
}

/// `device_pager_init()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Runs once from the boot sequence, before any pager exists.
pub(crate) unsafe fn init() {
    // SAFETY: this call builds the caches and the queue heads before any
    // pager is created.
    unsafe {
        (*ptr::addr_of_mut!(DEV_PAGER_CACHE)).init(
            b"dev_pager",
            size_of::<DevPager>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        (*ptr::addr_of_mut!(DEV_PAGER_HASH_CACHE)).init(
            b"dev_pager_entry",
            size_of::<DevPagerEntry>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        (*ptr::addr_of_mut!(DEV_DEVICE_HASH_CACHE)).init(
            b"dev_device_entry",
            size_of::<DevDeviceEntry>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        for i in 0..DEV_HASH_COUNT {
            // SAFETY: `i` is below the array bounds.
            queue_init(
                ptr::addr_of_mut!(DEV_PAGER_HASHTABLE)
                    .cast::<QueueEntry>()
                    .add(i),
            );
            // SAFETY: as above for the device table.
            queue_init(
                ptr::addr_of_mut!(DEV_DEVICE_HASHTABLE)
                    .cast::<QueueEntry>()
                    .add(i),
            );
        }
    }
}
