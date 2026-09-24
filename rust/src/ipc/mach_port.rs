// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/mach_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The port calls, which `ipc/mach_port.c` used to define and
//! `ipc/mach_port.h` belongs to.

use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue;
use crate::ipc::ipc_init;
use crate::ipc::ipc_object::{self, copyin_type};
use crate::ipc::ipc_port;
use crate::ipc::{IE_BITS_TYPE_MASK, IpcEntry, IpcPort, IpcSpace, IpcTarget};
use crate::kern::rdxtree::RdxtreeIter;
use crate::kern::task::current_task;
use crate::kern::types::KernError;
use crate::vm::error::Error;
use crate::vm::types::VmProt;
use crate::vm::vm_kern_ffi::kmem_free;
use crate::vm::vm_map::{VmMapCopy, round_page};
use crate::vm::vm_user;
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of, size_of_val};
use core::ptr::{self, NonNull};
use core::slice;
use core::sync::atomic::{AtomicBool, Ordering};

/// `MACH_PORT_NAME_NULL` of <mach/port.h>: the name no entry holds.
const MACH_PORT_NAME_NULL: c_uint = 0;
/// `MACH_PORT_RIGHT_RECEIVE` of <mach/port.h>: the right
/// `ipc_port_translate_receive` asks `ipc_object_translate` for.
const MACH_PORT_RIGHT_RECEIVE: c_uint = 1;
/// `MACH_PORT_QLIMIT_MAX` of <mach/port.h>: the largest queue limit
/// `mach_port_set_qlimit()` accepts.
const MACH_PORT_QLIMIT_MAX: c_uint = 16;
/// `MACH_PORT_KTYPE_NONE` of <mach/port.h>.
const MACH_PORT_KTYPE_NONE: c_uint = 0;
/// `MACH_PORT_KTYPE_USER_DEVICE` of <mach/port.h>: the only other kobject
/// type `mach_port_set_ktype()` accepts.
const MACH_PORT_KTYPE_USER_DEVICE: c_uint = 28;

/// `MACH_PORT_TYPE_SEND_RIGHTS` of <mach/port.h>: send and send-once
/// rights.
const MACH_PORT_TYPE_SEND_RIGHTS: u32 = (1 << 16) | (1 << 18);
/// `MACH_PORT_TYPE_RECEIVE` of <mach/port.h>.
const MACH_PORT_TYPE_RECEIVE: u32 = 1 << 17;
/// `MACH_PORT_TYPE_PORT_SET` of <mach/port.h>.
const MACH_PORT_TYPE_PORT_SET: u32 = 1 << 19;
/// `MACH_PORT_TYPE_DEAD_NAME` of <mach/port.h>.
const MACH_PORT_TYPE_DEAD_NAME: u32 = 1 << 20;
/// `MACH_PORT_TYPE_DNREQUEST` of <mach/port.h>: a dead-name request is
/// outstanding.
const MACH_PORT_TYPE_DNREQUEST: u32 = 0x8000_0000;
/// `MACH_PORT_TYPE_MAREQUEST` of <mach/port.h>: a msg-accepted request is
/// outstanding.
const MACH_PORT_TYPE_MAREQUEST: u32 = 0x4000_0000;
/// `IE_BITS_MAREQUEST` of <ipc/ipc_entry.h>: the msg-accepted bit of an
/// entry.
const IE_BITS_MAREQUEST: u32 = 0x0020_0000;

/// `IKOT_NONE` of <kern/ipc_kobject.h>: a port bound to no kernel object.
const IKOT_NONE: c_uint = 0;
/// `IKOT_USER_DEVICE` of <kern/ipc_kobject.h>.
const IKOT_USER_DEVICE: c_uint = 28;

/// `MACH_MSG_TYPE_MOVE_RECEIVE`: the first port type name.
const MOVE_RECEIVE: c_uint = 16;
/// `MACH_MSG_TYPE_MOVE_SEND_ONCE`: the last name
/// `MACH_MSG_TYPE_PORT_ANY_RIGHT` accepts.
const MOVE_SEND_ONCE: c_uint = 18;
/// `MACH_MSG_TYPE_MAKE_SEND_ONCE`: the last name `MACH_MSG_TYPE_PORT_ANY`
/// accepts.
const MAKE_SEND_ONCE: c_uint = 21;

/// `MACH_NOTIFY_PORT_DESTROYED` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 5`, a
/// receive right was deallocated.
const MACH_NOTIFY_PORT_DESTROYED: c_int = 0o100 + 5;
/// `MACH_NOTIFY_NO_SENDERS` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 6`, a
/// receive right has no extant send rights.
const MACH_NOTIFY_NO_SENDERS: c_int = 0o100 + 6;
/// `MACH_NOTIFY_DEAD_NAME` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 010`, a
/// send or send-once right died, leaving a dead name.
const MACH_NOTIFY_DEAD_NAME: c_int = 0o100 + 0o10;

/// `IO_DEAD` of <ipc/ipc_object.h>: the dead-object pointer, all bits set.
const IO_DEAD: *mut c_void = usize::MAX as *mut c_void;

/// `mach_port_deallocate_debug` of <ipc/ipc_space.h>: the debug switch the
/// bogus-name diagnostics check, which `ipc/mach_port.c` defined.
#[unsafe(no_mangle)]
pub(crate) static mut mach_port_deallocate_debug: c_int = 0;

/// `mach_port_status_t` of <mach/port.h>: the record
/// `mach_port_get_receive_status()` fills in.
#[repr(C)]
pub struct MachPortStatus {
    pub mps_pset: c_uint,
    pub mps_seqno: c_uint,
    pub mps_mscount: c_uint,
    pub mps_qlimit: c_uint,
    pub mps_msgcount: c_uint,
    pub mps_sorights: c_uint,
    pub mps_srights: c_int,
    pub mps_pdrequest: c_int,
    pub mps_nsrequest: c_int,
}

const _: () = {
    assert!(size_of::<MachPortStatus>() == 36);
    assert!(align_of::<MachPortStatus>() == 4);
    assert!(offset_of!(MachPortStatus, mps_pset) == 0);
    assert!(offset_of!(MachPortStatus, mps_seqno) == 4);
    assert!(offset_of!(MachPortStatus, mps_mscount) == 8);
    assert!(offset_of!(MachPortStatus, mps_qlimit) == 12);
    assert!(offset_of!(MachPortStatus, mps_msgcount) == 16);
    assert!(offset_of!(MachPortStatus, mps_sorights) == 20);
    assert!(offset_of!(MachPortStatus, mps_srights) == 24);
    assert!(offset_of!(MachPortStatus, mps_pdrequest) == 28);
    assert!(offset_of!(MachPortStatus, mps_nsrequest) == 32);
};

/// The `MACH_PORT_RIGHT_*` values the port calls accept.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortRight {
    Send = 0,
    Receive = 1,
    SendOnce = 2,
    PortSet = 3,
    DeadName = 4,
}

impl PortRight {
    /// The right a wire value names, or `None` for the values the C's
    /// `right >= MACH_PORT_RIGHT_NUMBER` checks rejected.
    const fn from_u32(right: c_uint) -> Option<Self> {
        match right {
            0 => Some(Self::Send),
            1 => Some(Self::Receive),
            2 => Some(Self::SendOnce),
            3 => Some(Self::PortSet),
            4 => Some(Self::DeadName),
            _ => None,
        }
    }
}

/// The `mach_port_names()` result: the two map copies and the count both
/// arrays hold.
pub(crate) struct PortNames {
    pub(crate) names: Option<NonNull<VmMapCopy>>,
    pub(crate) types: Option<NonNull<VmMapCopy>>,
    pub(crate) count: c_uint,
}

/// The `mach_port_get_set_status()` result.
pub(crate) struct SetStatus {
    pub(crate) members: Option<NonNull<VmMapCopy>>,
    pub(crate) count: c_uint,
}

/// `MACH_PORT_NAME_VALID(name)` of <mach/port.h>: a name is valid when it is
/// neither null nor dead.
const fn port_name_valid(name: c_uint) -> bool {
    name != 0 && name != c_uint::MAX
}

/// `MACH_PORT_TYPE(right)` of <mach/port.h>: `1 << (right + 16)`.
const fn mach_port_type(right: c_uint) -> c_uint {
    // The C shifts by `right + 16`; the target masks the count the same way.
    1u32.wrapping_shl(right.wrapping_add(16))
}

/// `IO_VALID(io)` of <ipc/ipc_object.h>: an object is valid when it is
/// neither null nor dead.
fn io_valid(object: *mut c_void) -> bool {
    !object.is_null() && !core::ptr::eq(object, IO_DEAD)
}

/// An `ipc_entry_num_t` as an index.  `usize` is at least as wide as the
/// C's `unsigned int` on both targets, so the widening cannot lose anything.
const fn as_index(count: c_uint) -> usize {
    count as usize
}

/// `IP_TIMESTAMP_ORDER(one, two)` of <ipc/ipc_port.h>: whether `one`
/// happened before `two` across the counter's 32-bit wrap.
fn timestamp_order(one: c_uint, two: c_uint) -> bool {
    // The C casts the wrapping difference to a signed int, so only the sign
    // bit matters; this is the same reinterpretation.
    (one.wrapping_sub(two) as c_int) < 0
}

/// The [`KernError`] a C `kern_return_t` stands for.
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// The [`KernError`] a VM map error stands for.
fn map_error(error: Error) -> KernError {
    match error {
        Error::InvalidAddress => KernError::InvalidAddress,
        Error::NoSpace => KernError::NoSpace,
        Error::InvalidArgument => KernError::InvalidArgument,
        Error::ResourceShortage => KernError::ResourceShortage,
        _ => KernError::Failure,
    }
}

/// The C `printf_once` macro: print one literal once, as the per-site static
/// did.
fn printf_once(printed: &AtomicBool, message: &CStr) {
    if !printed.load(Ordering::Relaxed) {
        printed.store(true, Ordering::Relaxed);

        // SAFETY: `message` is a NUL-terminated literal with no arguments.
        unsafe { glue::printf(message.as_ptr()) };
    }
}

/// The C's "task ... a bogus port ..." diagnostic, for the two routines that
/// share its wording.
///
/// # Safety
///
/// `space` must be live and the caller must run in a thread context.
unsafe fn report_bogus_port(space: IpcSpace, name: c_uint, action: &CStr) {
    // SAFETY: the caller promises the thread context.
    let task = unsafe { current_task() };
    if !port_name_valid(name) || space.as_ptr() != unsafe { (*task).itk_space }
    {
        return;
    }

    // SAFETY: the task is live and its name array has the size the format's
    // precision reads; `action` is NUL-terminated.
    unsafe {
        glue::printf(
            c"task %.*s %s a bogus port %lu, most probably a bug.\n".as_ptr(),
            // The C's `(int) sizeof`.
            size_of_val(&(*task).name) as c_int,
            ptr::addr_of!((*task).name).cast::<c_char>(),
            action.as_ptr(),
            c_ulong::from(name),
        );

        if mach_port_deallocate_debug != 0 {
            glue::SoftDebugger(c"mach_port_deallocate".as_ptr());
        }
    }
}

/// The `ipc_right_lookup_write()` call of the routines that report a bogus
/// name.
///
/// # Safety
///
/// `space` must be live and unlocked, and `name` is a plain value.
unsafe fn lookup_write(
    space: IpcSpace,
    name: c_uint,
) -> Result<*mut IpcEntry, KernError> {
    let mut found: *mut c_void = ptr::null_mut();

    // SAFETY: the caller promises the live space; `found` is this live local,
    // written only on success.
    kern_error(unsafe {
        glue::ipc_right_lookup_write(space.as_ptr(), name, &mut found)
    })?;

    Ok(found.cast())
}

/// `mach_port_names_helper()` in C.
///
/// # Safety
///
/// `entry` must be a live entry of the read-locked space, and `names` and
/// `types` must have room for `*actual` plus one.
unsafe fn names_push(
    timestamp: c_uint,
    entry: *mut IpcEntry,
    names: &mut [c_uint],
    types: &mut [c_uint],
    actual: &mut c_uint,
) {
    // SAFETY: the caller promises the live entry.
    let mut bits = unsafe { (*entry).bits() };
    // SAFETY: as above; `ie_request` shares the entry's index word.
    let mut request = unsafe { (*entry).request() };

    if bits & MACH_PORT_TYPE_SEND_RIGHTS != 0 {
        // SAFETY: a send-rights entry names a live port.
        let port = IpcPort::valid(unsafe { (*entry).object() });

        if let Some(port) = port {
            // SAFETY: the space is read-locked and the entry's right keeps
            // the port alive, so this is the only lock taken.
            let died = unsafe {
                port.lock();
                let died = !port.is_active()
                    && timestamp_order(port.timestamp(), timestamp);
                port.unlock();
                died
            };

            if died {
                bits &= !(IE_BITS_TYPE_MASK | IE_BITS_MAREQUEST);
                bits |= MACH_PORT_TYPE_DEAD_NAME;
                if request != 0 {
                    bits = bits.wrapping_add(1);
                }
                request = 0;
            }
        }
    }

    let mut type_ = bits & IE_BITS_TYPE_MASK;
    if request != 0 {
        type_ |= MACH_PORT_TYPE_DNREQUEST;
    }
    if bits & IE_BITS_MAREQUEST != 0 {
        type_ |= MACH_PORT_TYPE_MAREQUEST;
    }

    let index = *actual;
    if let (Some(names), Some(types)) = (
        names.get_mut(as_index(index)),
        types.get_mut(as_index(index)),
    ) {
        // SAFETY: the caller promises the live entry.
        *names = unsafe { (*entry).name() };
        *types = type_;
    }
    *actual = index.wrapping_add(1);
}

/// `mach_port_names()` in C: the names and types of a space's rights, as two
/// map copies.
///
/// # Safety
///
/// `space` must be null or live and unlocked; may allocate memory.  The
/// returned copies hold their own references.
pub(crate) unsafe fn names(
    space: Option<IpcSpace>,
) -> Result<PortNames, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    let map = ipc_init::kernel_map();
    let mut size: VmSize = 0;
    let mut addr1: VmOffset = 0;
    let mut addr2: VmOffset = 0;
    let bound: c_uint;

    loop {
        // SAFETY: the caller promises a live space with nothing locked.
        unsafe { space.lock_read() };

        // SAFETY: the space is live and read-locked.
        if !unsafe { space.is_active() } {
            // SAFETY: the space is live and read-locked.
            unsafe { space.lock_done() };
            if size != 0 {
                // SAFETY: the two regions came from the allocations below.
                unsafe {
                    kmem_free(map, addr1, size);
                    kmem_free(map, addr2, size);
                }
            }
            return Err(KernError::InvalidTask);
        }

        // The C's `bound` is a 32-bit `ipc_entry_num_t`, so `is_size`
        // truncates the same way.
        // SAFETY: the space is live and read-locked.
        let bound_now = unsafe { (*space.record()).size as c_uint };
        let size_needed =
            round_page(as_index(bound_now) * size_of::<c_uint>());

        if size_needed <= size {
            bound = bound_now;
            break;
        }

        // SAFETY: the space is live and read-locked.
        unsafe { space.lock_done() };

        if size != 0 {
            // SAFETY: the two regions came from the allocations below.
            unsafe {
                kmem_free(map, addr1, size);
                kmem_free(map, addr2, size);
            }
        }
        size = size_needed;

        static FIRST_NO_ROOM: AtomicBool = AtomicBool::new(false);
        static SECOND_NO_ROOM: AtomicBool = AtomicBool::new(false);

        // SAFETY: `ipc_kernel_map` is the live kernel map, nothing is
        // locked, and `addr1` is this call's live local.
        if unsafe { vm_user::allocate(&mut *map, &mut addr1, size, true) }
            .is_err()
        {
            printf_once(&FIRST_NO_ROOM, c"no more room in ipc_kernel_map\n");
            return Err(KernError::ResourceShortage);
        }

        // SAFETY: as above, with `addr2`.
        if unsafe { vm_user::allocate(&mut *map, &mut addr2, size, true) }
            .is_err()
        {
            printf_once(&SECOND_NO_ROOM, c"no more room in ipc_kernel_map\n");
            // SAFETY: `addr1` came from the allocation above.
            unsafe { kmem_free(map, addr1, size) };
            return Err(KernError::ResourceShortage);
        }

        // The C ignored both statuses; the regions were just allocated from
        // the kernel map.
        // SAFETY: the regions are live in the live kernel map.
        unsafe {
            let _ = (*map).pageable(
                addr1,
                addr1.wrapping_add(size),
                VmProt::READ | VmProt::WRITE,
                true,
                true,
            );
            let _ = (*map).pageable(
                addr2,
                addr2.wrapping_add(size),
                VmProt::READ | VmProt::WRITE,
                true,
                true,
            );
        }
    }
    // The space is read-locked and active, and `bound` bounds its entries.

    let mut actual: c_uint = 0;
    let timestamp = ipc_port::timestamp();
    // SAFETY: each region holds `bound` names, and the space is read-locked.
    let names = unsafe {
        slice::from_raw_parts_mut(addr1 as *mut c_uint, as_index(bound))
    };
    // SAFETY: as above, with the types region.
    let types = unsafe {
        slice::from_raw_parts_mut(addr2 as *mut c_uint, as_index(bound))
    };

    // SAFETY: the space is live and read-locked; the map address is formed
    // without reading.
    let map_ptr = unsafe { ptr::addr_of_mut!((*space.record()).map) };
    let mut iter = RdxtreeIter::new();
    // SAFETY: the space is read-locked, so its map is stable; each walk
    // returns a live entry once.
    while let Some(found) = unsafe { (*map_ptr).walk(&mut iter) } {
        let entry = found.as_ptr().cast::<IpcEntry>();

        // SAFETY: a walked pointer is a live entry.
        if unsafe { (*entry).bits() } & IE_BITS_TYPE_MASK != 0 {
            // SAFETY: `bound` bounds the entries the walk can yield, so the
            // slices have room, and the entry is live under the space lock.
            unsafe { names_push(timestamp, entry, names, types, &mut actual) };
        }
    }

    // SAFETY: the space is live and read-locked.
    unsafe { space.lock_done() };

    if actual == 0 {
        if size != 0 {
            // SAFETY: the two regions came from the allocations above.
            unsafe {
                kmem_free(map, addr1, size);
                kmem_free(map, addr2, size);
            }
        }
        return Ok(PortNames {
            names: None,
            types: None,
            count: 0,
        });
    }

    let size_used = round_page(as_index(actual) * size_of::<c_uint>());

    // The C ignored both statuses; the copies below consume the regions.
    // SAFETY: the regions are live in the live kernel map.
    unsafe {
        let _ = (*map).pageable(
            addr1,
            addr1.wrapping_add(size_used),
            VmProt::NONE,
            true,
            true,
        );
        let _ = (*map).pageable(
            addr2,
            addr2.wrapping_add(size_used),
            VmProt::NONE,
            true,
            true,
        );
    }

    // SAFETY: each region holds `actual` names; the C left the copies
    // uninitialized when one of these failed, which cannot be expressed.
    let names_copy =
        unsafe { (*map).copyin(addr1, size_used, true) }.map_err(map_error)?;
    // SAFETY: as above, with the types region.
    let types_copy =
        unsafe { (*map).copyin(addr2, size_used, true) }.map_err(map_error)?;

    if size_used != size {
        // SAFETY: the tails of the allocations are unused.
        unsafe {
            kmem_free(map, addr1.wrapping_add(size_used), size - size_used);
            kmem_free(map, addr2.wrapping_add(size_used), size - size_used);
        }
    }

    Ok(PortNames {
        names: Some(names_copy),
        types: Some(types_copy),
        count: actual,
    })
}

/// `mach_port_type()` in C: the type bits of the named right.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn port_type(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<c_uint, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let entry = unsafe { lookup_write(space, name) }?;

    let mut type_: c_uint = 0;
    let mut ignored_urefs: c_uint = 0;
    // SAFETY: the lookup returned the live entry with the space
    // write-locked; `ipc_right_info` leaves the space locked on success.
    let result = kern_error(unsafe {
        glue::ipc_right_info(
            space.as_ptr(),
            name,
            entry.cast(),
            &mut type_,
            &mut ignored_urefs,
        )
    });

    if result.is_ok() {
        // SAFETY: the space is live and write-locked.
        unsafe { space.lock_done() };
    }

    result.map(|()| type_)
}

/// `mach_port_allocate_name()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked; may allocate memory.
pub(crate) unsafe fn allocate_name(
    space: Option<IpcSpace>,
    right: c_uint,
    name: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !port_name_valid(name) {
        return Err(KernError::InvalidValue);
    }

    match PortRight::from_u32(right) {
        Some(PortRight::Receive) => {
            // SAFETY: the caller promises a live space; a successful
            // allocation returns the port locked.
            let port = ipc_port::alloc_name(space, name)?;

            // SAFETY: the port is locked and the caller holds no reference,
            // as the C's `ip_unlock`.
            unsafe { port.unlock() };
            Ok(())
        }
        Some(PortRight::PortSet) => {
            let mut pset: *mut c_void = ptr::null_mut();

            // SAFETY: the caller promises the live space; `pset` is this
            // live local, written only on success.
            kern_error(unsafe {
                glue::ipc_pset_alloc_name(space.as_ptr(), name, &mut pset)
            })?;

            // SAFETY: a successful allocation returned the port set locked.
            unsafe { (*pset.cast::<IpcTarget>()).unlock() };
            Ok(())
        }
        Some(PortRight::DeadName) => {
            // SAFETY: the caller promises the live space and nothing locked.
            unsafe { ipc_object::alloc_dead_name(space, name) }
        }
        Some(PortRight::Send) | Some(PortRight::SendOnce) | None => {
            Err(KernError::InvalidValue)
        }
    }
}

/// `mach_port_allocate()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked; may allocate memory.
pub(crate) unsafe fn allocate(
    space: Option<IpcSpace>,
    right: c_uint,
) -> Result<c_uint, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    match PortRight::from_u32(right) {
        Some(PortRight::Receive) => {
            // SAFETY: the caller promises a live space; a successful
            // allocation returns the port locked.
            let (name, port) = ipc_port::alloc(space)?;

            // SAFETY: the port is locked and the caller holds no reference,
            // as the C's `ip_unlock`.
            unsafe { port.unlock() };
            Ok(name)
        }
        Some(PortRight::PortSet) => {
            let mut name: c_uint = 0;
            let mut pset: *mut c_void = ptr::null_mut();

            // SAFETY: the caller promises the live space; the two
            // out-pointers are this call's live locals, written only on
            // success.
            kern_error(unsafe {
                glue::ipc_pset_alloc(space.as_ptr(), &mut name, &mut pset)
            })?;

            // SAFETY: a successful allocation returned the port set locked.
            unsafe { (*pset.cast::<IpcTarget>()).unlock() };
            Ok(name)
        }
        Some(PortRight::DeadName) => {
            // SAFETY: the caller promises the live space and nothing locked.
            unsafe { ipc_object::alloc_dead(space) }
        }
        Some(PortRight::Send) | Some(PortRight::SendOnce) | None => {
            Err(KernError::InvalidValue)
        }
    }
}

/// `mach_port_destroy()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn destroy(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space with nothing locked.
    let entry = match unsafe { lookup_write(space, name) } {
        Ok(entry) => entry,
        Err(error) => {
            // SAFETY: the space is live and this call runs in a thread
            // context.
            unsafe { report_bogus_port(space, name, c"destroying") };
            return Err(error);
        }
    };

    // SAFETY: the lookup returned the live entry with the space
    // write-locked; `ipc_right_destroy` unlocks the space.
    kern_error(unsafe {
        glue::ipc_right_destroy(space.as_ptr(), name, entry.cast())
    })
}

/// `mach_port_deallocate()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn deallocate(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space with nothing locked.
    let entry = match unsafe { lookup_write(space, name) } {
        Ok(entry) => entry,
        Err(error) => {
            // SAFETY: the space is live and this call runs in a thread
            // context.
            unsafe { report_bogus_port(space, name, c"deallocating") };
            return Err(error);
        }
    };

    // SAFETY: the lookup returned the live entry with the space
    // write-locked; `ipc_right_dealloc` unlocks the space.
    kern_error(unsafe {
        glue::ipc_right_dealloc(space.as_ptr(), name, entry.cast())
    })
}

/// `mach_port_get_refs()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn get_refs(
    space: Option<IpcSpace>,
    name: c_uint,
    right: c_uint,
) -> Result<c_uint, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };
    let Some(right) = PortRight::from_u32(right) else {
        return Err(KernError::InvalidValue);
    };

    // SAFETY: the caller promises a live space.
    let entry = unsafe { lookup_write(space, name) }?;

    let mut type_: c_uint = 0;
    let mut urefs: c_uint = 0;
    // SAFETY: the lookup returned the live entry with the space
    // write-locked; `ipc_right_info` leaves the space locked on success.
    kern_error(unsafe {
        glue::ipc_right_info(
            space.as_ptr(),
            name,
            entry.cast(),
            &mut type_,
            &mut urefs,
        )
    })?;

    // SAFETY: the space is live and write-locked.
    unsafe { space.lock_done() };

    if type_ & mach_port_type(right as c_uint) == 0 {
        return Ok(0);
    }

    match right {
        PortRight::SendOnce | PortRight::PortSet | PortRight::Receive => Ok(1),
        PortRight::DeadName | PortRight::Send => Ok(urefs),
    }
}

/// `mach_port_mod_refs()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn mod_refs(
    space: Option<IpcSpace>,
    name: c_uint,
    right: c_uint,
    delta: c_int,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };
    if PortRight::from_u32(right).is_none() {
        return Err(KernError::InvalidValue);
    }

    // SAFETY: the caller promises a live space with nothing locked.
    let entry = match unsafe { lookup_write(space, name) } {
        Ok(entry) => entry,
        Err(error) => {
            // SAFETY: the task is live in this thread context and the space
            // is live.
            let task = unsafe { current_task() };
            if port_name_valid(name)
                && space.as_ptr() == unsafe { (*task).itk_space }
            {
                // SAFETY: the format is the C's and the arguments are the
                // values it reads.
                unsafe {
                    glue::printf(
                        c"task %.*s %screasing a bogus port %u by %d, \
                          most probably a bug.\n"
                            .as_ptr(),
                        // The C's `(int) sizeof`.
                        size_of_val(&(*task).name) as c_int,
                        ptr::addr_of!((*task).name).cast::<c_char>(),
                        if delta < 0 {
                            c"de".as_ptr()
                        } else {
                            c"in".as_ptr()
                        },
                        name,
                        if delta < 0 {
                            delta.wrapping_neg()
                        } else {
                            delta
                        },
                    );

                    if mach_port_deallocate_debug != 0 {
                        glue::SoftDebugger(c"mach_port_mod_refs".as_ptr());
                    }
                }
            }
            return Err(error);
        }
    };

    // SAFETY: the lookup returned the live entry with the space
    // write-locked; `ipc_right_delta` unlocks the space.
    kern_error(unsafe {
        glue::ipc_right_delta(space.as_ptr(), name, entry.cast(), right, delta)
    })
}

/// `mach_port_set_qlimit()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn set_qlimit(
    space: Option<IpcSpace>,
    name: c_uint,
    qlimit: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };
    if qlimit > MACH_PORT_QLIMIT_MAX {
        return Err(KernError::InvalidValue);
    }

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked; `ipc_port_set_qlimit`
    // leaves it locked and the C's `ip_unlock` follows.
    unsafe {
        ipc_port::set_qlimit(port, qlimit);
        port.unlock();
    }

    Ok(())
}

/// `mach_port_set_mscount()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn set_mscount(
    space: Option<IpcSpace>,
    name: c_uint,
    mscount: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked; the C's assignment and
    // `ip_unlock`.
    unsafe {
        port.set_mscount(mscount);
        port.unlock();
    }

    Ok(())
}

/// `mach_port_set_seqno()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn set_seqno(
    space: Option<IpcSpace>,
    name: c_uint,
    seqno: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked; `ipc_port_set_seqno`
    // leaves it locked and the C's `ip_unlock` follows.
    unsafe {
        ipc_port::set_seqno(port, seqno);
        port.unlock();
    }

    Ok(())
}

/// `mach_port_gst_helper()` in C.
///
/// # Safety
///
/// `pset` must be a live port set, `port` a live receive port, and `names`
/// must have room for `*actual` plus one.
unsafe fn get_set_status_helper(
    pset: *mut IpcTarget,
    port: *mut c_void,
    maxnames: usize,
    names: &mut [c_uint],
    actual: &mut c_uint,
) {
    // SAFETY: the caller promises the live port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller promises the live port, whose lock is the only one
    // taken here.
    let (name, ip_pset) = unsafe {
        port.lock();
        let pair = (port.receiver_name(), port.pset());
        port.unlock();
        pair
    };

    if pset.cast::<c_void>() == ip_pset {
        let index = *actual;
        if as_index(index) < maxnames
            && let Some(slot) = names.get_mut(as_index(index))
        {
            *slot = name;
        }
        *actual = index.wrapping_add(1);
    }
}

/// `mach_port_get_set_status()` in C: the members of a port set, as a map
/// copy.
///
/// # Safety
///
/// `space` must be null or live and unlocked; may allocate memory.  The
/// returned copy holds its own reference.
pub(crate) unsafe fn get_set_status(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<SetStatus, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    let map = ipc_init::kernel_map();
    let mut size: VmSize = PAGE_SIZE;

    let (addr, actual) = loop {
        let mut addr: VmOffset = 0;

        // SAFETY: `ipc_kernel_map` is the live kernel map, nothing is
        // locked, and `addr` is this call's live local.
        if unsafe { vm_user::allocate(&mut *map, &mut addr, size, true) }
            .is_err()
        {
            static NO_ROOM: AtomicBool = AtomicBool::new(false);
            printf_once(&NO_ROOM, c"no more room in ipc_kernel_map\n");
            return Err(KernError::ResourceShortage);
        }

        // The C ignored the status; the region was just allocated from the
        // kernel map.
        // SAFETY: the region is live in the live kernel map.
        unsafe {
            let _ = (*map).pageable(
                addr,
                addr.wrapping_add(size),
                VmProt::READ | VmProt::WRITE,
                true,
                true,
            );
        }

        // SAFETY: the caller promises a live space; a successful lookup
        // returns the space read-locked and active.
        let entry = match unsafe { lookup_write(space, name) } {
            Ok(entry) => entry,
            Err(error) => {
                // SAFETY: the region came from the allocation above.
                unsafe { kmem_free(map, addr, size) };
                return Err(error);
            }
        };

        // SAFETY: the lookup returned the live entry.
        if unsafe { (*entry).bits() } & IE_BITS_TYPE_MASK
            != MACH_PORT_TYPE_PORT_SET
        {
            // SAFETY: the space is live and read-locked.
            unsafe {
                space.lock_done();
                kmem_free(map, addr, size);
            }
            return Err(KernError::InvalidRight);
        }

        // SAFETY: a port-set entry names a live port set.
        let pset = unsafe { (*entry).object() };
        let maxnames = size / size_of::<c_uint>();
        // SAFETY: the allocation holds `maxnames` names.
        let names = unsafe {
            slice::from_raw_parts_mut(addr as *mut c_uint, maxnames)
        };
        let mut actual: c_uint = 0;

        // SAFETY: the space is live and read-locked; the map address is
        // formed without reading.
        let map_ptr = unsafe { ptr::addr_of_mut!((*space.record()).map) };
        let mut iter = RdxtreeIter::new();
        // SAFETY: the space is read-locked, so its map is stable; each walk
        // returns a live entry once.
        while let Some(found) = unsafe { (*map_ptr).walk(&mut iter) } {
            let entry = found.as_ptr().cast::<IpcEntry>();

            // SAFETY: a walked pointer is a live entry.
            if unsafe { (*entry).bits() } & MACH_PORT_TYPE_RECEIVE != 0 {
                // SAFETY: a receive entry names a live port.
                let port = unsafe { (*entry).object() };

                // SAFETY: the entry's receive right keeps the port alive for
                // the walk.
                unsafe {
                    get_set_status_helper(
                        pset.cast::<IpcTarget>(),
                        port,
                        maxnames,
                        names,
                        &mut actual,
                    );
                }
            }
        }

        // SAFETY: the space is live and read-locked.
        unsafe { space.lock_done() };

        if as_index(actual) <= maxnames {
            break (addr, actual);
        }

        // SAFETY: the region came from the allocation above.
        unsafe { kmem_free(map, addr, size) };
        size = round_page(as_index(actual) * size_of::<c_uint>()) + PAGE_SIZE;
    };

    if actual == 0 {
        // SAFETY: the region came from the allocation above.
        unsafe { kmem_free(map, addr, size) };
        return Ok(SetStatus {
            members: None,
            count: 0,
        });
    }

    let size_used = round_page(as_index(actual) * size_of::<c_uint>());

    // The C ignored the status; the copy below consumes the region.
    // SAFETY: the region is live in the live kernel map.
    unsafe {
        let _ = (*map).pageable(
            addr,
            addr.wrapping_add(size_used),
            VmProt::NONE,
            true,
            true,
        );
    }

    // SAFETY: the region holds `actual` names; the C left the copy
    // uninitialized when this failed, which cannot be expressed.
    let memory =
        unsafe { (*map).copyin(addr, size_used, true) }.map_err(map_error)?;

    if size_used != size {
        // SAFETY: the tail of the allocation is unused.
        unsafe {
            kmem_free(map, addr.wrapping_add(size_used), size - size_used);
        }
    }

    Ok(SetStatus {
        members: Some(memory),
        count: actual,
    })
}

/// `mach_port_move_member()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn move_member(
    space: Option<IpcSpace>,
    member: c_uint,
    after: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let entry = unsafe { lookup_write(space, member) }?;

    // SAFETY: the lookup returned the live entry.
    if unsafe { (*entry).bits() } & MACH_PORT_TYPE_RECEIVE == 0 {
        // SAFETY: the space is live and write-locked.
        unsafe { space.lock_done() };
        return Err(KernError::InvalidRight);
    }

    // SAFETY: as above; a receive entry names a live port.
    let port = unsafe { (*entry).object() };

    let nset = if after == MACH_PORT_NAME_NULL {
        ptr::null_mut()
    } else {
        // SAFETY: the space is live and write-locked.
        let Some(entry) = (unsafe { space.entry_lookup(after) }) else {
            // SAFETY: the space is live and write-locked.
            unsafe { space.lock_done() };
            return Err(KernError::InvalidName);
        };

        // SAFETY: a looked-up entry is live.
        if unsafe { (*entry).bits() } & MACH_PORT_TYPE_PORT_SET == 0 {
            // SAFETY: the space is live and write-locked.
            unsafe { space.lock_done() };
            return Err(KernError::InvalidRight);
        }

        // SAFETY: a port-set entry names a live port set.
        unsafe { (*entry).object() }
    };

    // SAFETY: the lookup left the space write-locked and active;
    // `ipc_pset_move` unlocks the space.
    kern_error(unsafe { glue::ipc_pset_move(space.as_ptr(), port, nset) })
}

/// `mach_port_get_receive_status()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn get_receive_status(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<MachPortStatus, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked; the port-set and message
    // queue locks come and go under it, as the C's do.
    let (mps_pset, mps_seqno) = unsafe {
        let pset = port.pset();
        if pset.is_null() {
            let mqueue = port.messages();
            (*mqueue).lock();
            let seqno = port.seqno();
            (*mqueue).unlock();
            (MACH_PORT_NAME_NULL, seqno)
        } else {
            let target = pset.cast::<IpcTarget>();
            (*target).lock();
            if (*target).is_active() {
                let name = (*target).local_name();
                let mqueue = (*target).messages();
                (*mqueue).lock();
                let seqno = port.seqno();
                (*mqueue).unlock();
                (*target).unlock();
                (name, seqno)
            } else {
                glue::ipc_pset_remove(pset, port.as_ptr());
                IpcTarget::check_unlock(target);
                let mqueue = port.messages();
                (*mqueue).lock();
                let seqno = port.seqno();
                (*mqueue).unlock();
                (MACH_PORT_NAME_NULL, seqno)
            }
        }
    };

    let status = MachPortStatus {
        mps_pset,
        mps_seqno,
        // SAFETY: the port is live and locked.
        mps_mscount: unsafe { port.mscount() },
        // SAFETY: as above.
        mps_qlimit: unsafe { port.qlimit() },
        // SAFETY: as above.
        mps_msgcount: unsafe { port.msgcount() },
        // SAFETY: as above.
        mps_sorights: unsafe { port.sorights() },
        // SAFETY: as above.
        mps_srights: c_int::from(unsafe { port.srights() } > 0),
        // SAFETY: as above.
        mps_pdrequest: c_int::from(!unsafe { port.pdrequest() }.is_null()),
        // SAFETY: as above.
        mps_nsrequest: c_int::from(!unsafe { port.nsrequest() }.is_null()),
    };

    // SAFETY: the port is live, active, and locked; the C's `ip_unlock`.
    unsafe { port.unlock() };

    Ok(status)
}

/// `mach_port_set_protected_payload()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn set_protected_payload(
    space: Option<IpcSpace>,
    name: c_uint,
    payload: usize,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked;
    // `ipc_port_set_protected_payload` leaves it locked and the C's
    // `ip_unlock` follows.
    unsafe {
        ipc_port::set_protected_payload(port, payload);
        port.unlock();
    }

    Ok(())
}

/// `mach_port_clear_protected_payload()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn clear_protected_payload(
    space: Option<IpcSpace>,
    name: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // SAFETY: the caller promises a live space.
    let port = unsafe { translate_receive(space, name) }?;

    // SAFETY: `translate_receive` returned the live, locked, active port.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the port is live, active, and locked;
    // `ipc_port_clear_protected_payload` leaves it locked and the C's
    // `ip_unlock` follows.
    unsafe {
        ipc_port::clear_protected_payload(port);
        port.unlock();
    }

    Ok(())
}

/// `mach_port_set_ktype()` in C.
///
/// # Safety
///
/// `host` must be `HOST_NULL` or a live host, and `space` must be null or
/// live and unlocked.
pub(crate) unsafe fn set_ktype(
    host: *mut c_void,
    space: Option<IpcSpace>,
    name: c_uint,
    right: c_uint,
    ktype: c_uint,
) -> Result<(), KernError> {
    if host.is_null() {
        return Err(KernError::InvalidHost);
    }

    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };
    if ktype != MACH_PORT_KTYPE_NONE && ktype != MACH_PORT_KTYPE_USER_DEVICE {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live space; a successful translate
    // returns the object locked and active.
    let object = unsafe { ipc_object::translate(space, name, right) }?;

    // SAFETY: the translate returned the live, locked object.
    let port = unsafe { IpcPort::from_raw(object) };

    // SAFETY: the port is live and locked.
    let kotype = unsafe { port.kotype() };

    let result = if kotype == IKOT_NONE || kotype == IKOT_USER_DEVICE {
        // SAFETY: the port is live and locked, and `ktype` is one of the two
        // values the check above accepted.
        unsafe {
            glue::ipc_kobject_set_locked(
                port.as_ptr(),
                0,
                if ktype == MACH_PORT_KTYPE_USER_DEVICE {
                    IKOT_USER_DEVICE
                } else {
                    IKOT_NONE
                },
            );
        }
        Ok(())
    } else {
        Err(KernError::InvalidArgument)
    };

    // SAFETY: the port is live, active, and locked; the C's `ip_unlock`.
    unsafe { port.unlock() };

    result
}

/// `mach_port_rename()` in C.
///
/// # Safety
///
/// `space` must be null or live and unlocked.
pub(crate) unsafe fn rename(
    space: Option<IpcSpace>,
    oname: c_uint,
    nname: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !port_name_valid(nname) {
        return Err(KernError::InvalidValue);
    }

    // SAFETY: the caller promises a live space; `ipc_object_rename` takes no
    // reference.
    unsafe { ipc_object::rename(space, oname, nname) }
}

/// `mach_port_insert_right()` in C.
///
/// # Safety
///
/// `space` must be null or live; `poly` must be `IO_NULL`, `IO_DEAD` or a
/// live `ipc_object` the caller holds one reference to, which the call
/// consumes on success.
pub(crate) unsafe fn insert_right(
    space: Option<IpcSpace>,
    name: c_uint,
    poly: *mut c_void,
    poly_poly: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !port_name_valid(name)
        || !(MOVE_RECEIVE..=MOVE_SEND_ONCE).contains(&poly_poly)
    {
        return Err(KernError::InvalidValue);
    }

    if !io_valid(poly) {
        return Err(KernError::InvalidCapability);
    }

    // SAFETY: the caller promises a live space; `poly` is valid by the check
    // above, and on success `ipc_object_copyout_name` consumes one reference
    // to it, which the caller owns.
    unsafe { ipc_object::copyout_name(space, poly, poly_poly, false, name) }
}

/// `mach_port_extract_right()` in C.
///
/// # Safety
///
/// `space` must be null or live.
pub(crate) unsafe fn extract_right(
    space: Option<IpcSpace>,
    name: c_uint,
    msgt_name: c_uint,
) -> Result<(*mut c_void, c_uint), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !(MOVE_RECEIVE..=MAKE_SEND_ONCE).contains(&msgt_name) {
        return Err(KernError::InvalidValue);
    }

    // SAFETY: the caller promises a live space; the name is a plain value,
    // and a successful copyin returns an object holding one reference for
    // the caller.
    let object = unsafe { ipc_object::copyin(space, name, msgt_name) }?;

    Ok((object, copyin_type(msgt_name)))
}

/// `ipc_port_translate_receive()` of <ipc/ipc_port.h> in C.
///
/// # Safety
///
/// `space` must be live and unlocked.
unsafe fn translate_receive(
    space: IpcSpace,
    name: c_uint,
) -> Result<*mut c_void, KernError> {
    // SAFETY: the caller promises a live space; the name is a plain value,
    // and the call returns the live, locked port.
    unsafe { ipc_object::translate(space, name, MACH_PORT_RIGHT_RECEIVE) }
}

/// `mach_port_request_notification()` in C.
///
/// # Safety
///
/// `space` must be null or live; `notify` must be `IP_NULL`, `IP_DEAD` or a
/// live `ipc_port`, and on success the call consumes it.
pub(crate) unsafe fn request_notification(
    space: Option<IpcSpace>,
    name: c_uint,
    id: c_int,
    sync: c_uint,
    notify: *mut c_void,
) -> Result<Option<IpcPort>, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if core::ptr::eq(notify, IO_DEAD) {
        return Err(KernError::InvalidCapability);
    }

    match id {
        MACH_NOTIFY_PORT_DESTROYED => {
            if sync != 0 {
                return Err(KernError::InvalidValue);
            }

            // SAFETY: the caller promises a live space.
            let port = unsafe { translate_receive(space, name) }?;

            // SAFETY: `translate_receive` returned the live, locked port,
            // and `pdrequest` owns the unlock; `notify` is the caller's
            // send-once right.
            let previous = unsafe {
                ipc_port::pdrequest(IpcPort::from_raw(port), notify)
            };

            Ok(IpcPort::new(previous))
        }
        MACH_NOTIFY_NO_SENDERS => {
            // SAFETY: the caller promises a live space.
            let port = unsafe { translate_receive(space, name) }?;

            // SAFETY: `translate_receive` returned the live, locked port,
            // and `nsrequest` owns the unlock; `notify` is the caller's
            // send-once right.
            let previous = unsafe {
                ipc_port::nsrequest(IpcPort::from_raw(port), sync, notify)
            };

            Ok(IpcPort::new(previous))
        }
        MACH_NOTIFY_DEAD_NAME => {
            let mut previous: *mut c_void = ptr::null_mut();
            // SAFETY: the caller promises a live space; the name, flag and
            // notify port are plain values; this live local is the out-slot
            // `ipc_right_dnrequest` writes on success only.
            kern_error(unsafe {
                glue::ipc_right_dnrequest(
                    space.as_ptr(),
                    name,
                    c_int::from(sync != 0),
                    notify,
                    &mut previous,
                )
            })?;

            Ok(IpcPort::new(previous))
        }
        _ => Err(KernError::InvalidValue),
    }
}
