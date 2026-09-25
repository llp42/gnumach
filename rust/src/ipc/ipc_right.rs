// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_right.c and ipc/ipc_right.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The capability-manipulation routines, which `ipc/ipc_right.c` used to
//! define and `ipc/ipc_right.h` declares.

use crate::glue;
use crate::ipc::ipc_entry;
use crate::ipc::ipc_object;
use crate::ipc::ipc_port;
use crate::ipc::{
    IE_BITS_TYPE_MASK, IO_DEAD, IpcEntry, IpcPort, IpcSpace, IpcTarget,
};
use crate::kern::types::KernError;
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::ptr;

/// `MACH_PORT_NULL` and `MACH_PORT_NAME_NULL` of <mach/port.h>.
const MACH_PORT_NULL: c_uint = 0;
/// `MACH_PORT_TYPE_NONE` of <mach/port.h>.
const MACH_PORT_TYPE_NONE: u32 = 0;
/// `MACH_PORT_TYPE_SEND` of <mach/port.h>: `1 << (right + 16)` for the send
/// right.
const MACH_PORT_TYPE_SEND: u32 = 1 << 16;
/// `MACH_PORT_TYPE_RECEIVE` of <mach/port.h>.
const MACH_PORT_TYPE_RECEIVE: u32 = 1 << 17;
/// `MACH_PORT_TYPE_SEND_ONCE` of <mach/port.h>.
const MACH_PORT_TYPE_SEND_ONCE: u32 = 1 << 18;
/// `MACH_PORT_TYPE_PORT_SET` of <mach/port.h>.
const MACH_PORT_TYPE_PORT_SET: u32 = 1 << 19;
/// `MACH_PORT_TYPE_DEAD_NAME` of <mach/port.h>.
const MACH_PORT_TYPE_DEAD_NAME: u32 = 1 << 20;
/// `MACH_PORT_TYPE_SEND_RECEIVE` of <mach/port.h>.
const MACH_PORT_TYPE_SEND_RECEIVE: u32 =
    MACH_PORT_TYPE_SEND | MACH_PORT_TYPE_RECEIVE;
/// `MACH_PORT_TYPE_SEND_RIGHTS` of <mach/port.h>.
const MACH_PORT_TYPE_SEND_RIGHTS: u32 =
    MACH_PORT_TYPE_SEND | MACH_PORT_TYPE_SEND_ONCE;
/// `MACH_PORT_TYPE_PORT_RIGHTS` of <mach/port.h>.
const MACH_PORT_TYPE_PORT_RIGHTS: u32 =
    MACH_PORT_TYPE_SEND_RIGHTS | MACH_PORT_TYPE_RECEIVE;
/// `MACH_PORT_TYPE_PORT_OR_DEAD` of <mach/port.h>.
const MACH_PORT_TYPE_PORT_OR_DEAD: u32 =
    MACH_PORT_TYPE_PORT_RIGHTS | MACH_PORT_TYPE_DEAD_NAME;
/// `MACH_PORT_TYPE_DNREQUEST` of <mach/port.h>: the dummy type bit
/// `ipc_right_info()` reports for a dead-name request.
const MACH_PORT_TYPE_DNREQUEST: u32 = 0x8000_0000;
/// `MACH_PORT_TYPE_MAREQUEST` of <mach/port.h>: the dummy type bit for a
/// msg-accepted request.
const MACH_PORT_TYPE_MAREQUEST: u32 = 0x4000_0000;

/// `IE_BITS_UREFS_MASK` of <ipc/ipc_entry.h>.
const IE_BITS_UREFS_MASK: u32 = 0x0000_ffff;
/// `IE_BITS_MAREQUEST` of <ipc/ipc_entry.h>.
const IE_BITS_MAREQUEST: u32 = 0x0020_0000;
/// `IE_BITS_RIGHT_MASK` of <ipc/ipc_entry.h>.
const IE_BITS_RIGHT_MASK: u32 = 0x003f_ffff;
/// `MACH_PORT_UREFS_MAX` of <ipc/port.h>.
const MACH_PORT_UREFS_MAX: u32 = (1 << 16) - 1;

/// `MACH_PORT_RIGHT_SEND` of <mach/port.h>.
const MACH_PORT_RIGHT_SEND: c_uint = 0;
/// `MACH_PORT_RIGHT_RECEIVE` of <mach/port.h>.
const MACH_PORT_RIGHT_RECEIVE: c_uint = 1;
/// `MACH_PORT_RIGHT_SEND_ONCE` of <mach/port.h>.
const MACH_PORT_RIGHT_SEND_ONCE: c_uint = 2;
/// `MACH_PORT_RIGHT_PORT_SET` of <mach/port.h>.
const MACH_PORT_RIGHT_PORT_SET: c_uint = 3;
/// `MACH_PORT_RIGHT_DEAD_NAME` of <mach/port.h>.
const MACH_PORT_RIGHT_DEAD_NAME: c_uint = 4;

/// `MACH_MSG_TYPE_MOVE_RECEIVE` of <mach/message.h>.
const MACH_MSG_TYPE_MOVE_RECEIVE: c_uint = 16;
/// `MACH_MSG_TYPE_MOVE_SEND` of <mach/message.h>.
const MACH_MSG_TYPE_MOVE_SEND: c_uint = 17;
/// `MACH_MSG_TYPE_MOVE_SEND_ONCE` of <mach/message.h>.
const MACH_MSG_TYPE_MOVE_SEND_ONCE: c_uint = 18;
/// `MACH_MSG_TYPE_COPY_SEND` of <mach/message.h>.
const MACH_MSG_TYPE_COPY_SEND: c_uint = 19;
/// `MACH_MSG_TYPE_MAKE_SEND` of <mach/message.h>.
const MACH_MSG_TYPE_MAKE_SEND: c_uint = 20;
/// `MACH_MSG_TYPE_MAKE_SEND_ONCE` of <mach/message.h>.
const MACH_MSG_TYPE_MAKE_SEND_ONCE: c_uint = 21;

/// The C `default: panic()` arm of a rights switch.
fn strange_rights(fun: &'static CStr, message: &'static CStr) -> ! {
    // SAFETY: `Panic` does not return; the file is the one the switch belongs
    // to, the line is this Rust file's, and `fun` and `message` are the C's
    // own tags.
    unsafe {
        glue::Panic(
            c"ipc/ipc_right.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            fun.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// `MACH_PORT_UREFS_OVERFLOW()` of <ipc/port.h>: the C adds the signed delta
/// to the unsigned count, so the same wrapping sum decides it here.
fn urefs_overflow(urefs: u32, delta: c_int) -> bool {
    if delta <= 0 {
        return false;
    }

    // The positive delta converts exactly.
    let sum = urefs.wrapping_add(delta as u32);
    sum <= urefs || sum > MACH_PORT_UREFS_MAX
}

/// `MACH_PORT_UREFS_UNDERFLOW()` of <ipc/port.h>.
fn urefs_underflow(urefs: u32, delta: c_int) -> bool {
    // The negated delta is positive except at `c_int::MIN`, whose bit pattern
    // still compares greater than any 16-bit count.
    delta < 0 && delta.wrapping_neg() as u32 > urefs
}

/// `ipc_right_lookup_write()` in C.
///
/// # Safety
///
/// `space` must be live and unlocked.  On success the space is write-locked
/// and the returned entry is live.
pub(crate) unsafe fn lookup_write(
    space: IpcSpace,
    name: c_uint,
) -> Result<*mut IpcEntry, KernError> {
    // SAFETY: the caller promises a live, unlocked space.
    unsafe { space.lock_write() };

    // SAFETY: the space lock is held.
    if !unsafe { space.is_active() } {
        // SAFETY: the space lock is held.
        unsafe { space.lock_done() };
        return Err(KernError::InvalidTask);
    }

    // SAFETY: the space is live and write-locked.
    let Some(entry) = (unsafe { space.entry_lookup(name) }) else {
        // SAFETY: the space lock is held.
        unsafe { space.lock_done() };
        return Err(KernError::InvalidName);
    };

    Ok(entry)
}

/// `ipc_right_reverse()` in C.
///
/// # Safety
///
/// The space must be live and locked for reading or writing, and `object`
/// must be a live port.
pub(crate) unsafe fn reverse(
    space: IpcSpace,
    object: *mut c_void,
) -> Option<(c_uint, *mut IpcEntry)> {
    // SAFETY: the caller promises a live port.
    let port = unsafe { IpcPort::from_raw(object) };

    // SAFETY: the caller promises a live port and holds no port lock.
    unsafe { port.lock() };
    // SAFETY: the port lock is held.
    if !unsafe { port.is_active() } {
        // SAFETY: the port lock is held.
        unsafe { port.unlock() };
        return None;
    }

    // SAFETY: the port lock is held.
    if unsafe { port.receiver() } == space.as_ptr() {
        // SAFETY: the port lock is held.
        let name = unsafe { port.receiver_name() };

        // SAFETY: the caller holds the space lock.
        let entry = match unsafe { space.entry_lookup(name) } {
            Some(entry) => entry,
            None => ptr::null_mut(),
        };

        return Some((name, entry));
    }

    // SAFETY: the caller holds the space lock.
    let Some(entry) = (unsafe { space.reverse_lookup(port.as_ptr()) }) else {
        // SAFETY: the port lock is held; nothing was found for it.
        unsafe { port.unlock() };
        return None;
    };
    // SAFETY: a reverse-map result is a live entry.
    let name = unsafe { (*entry).name() };

    Some((name, entry))
}

/// `ipc_right_dnrequest()` in C.
///
/// # Safety
///
/// `space` must be live and unlocked; `notify` must be `IP_NULL` or a live
/// send-once right the call consumes on success.
pub(crate) unsafe fn dnrequest(
    space: IpcSpace,
    name: c_uint,
    immediate: bool,
    notify: *mut c_void,
) -> Result<*mut c_void, KernError> {
    loop {
        // SAFETY: the caller promises a live space.
        let entry = unsafe { lookup_write(space, name) }?;
        // SAFETY: the lookup returned a live entry.
        let mut bits = unsafe { (*entry).bits() };

        if bits & MACH_PORT_TYPE_PORT_RIGHTS != 0 {
            // SAFETY: a port-rights entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live; the
            // port is unlocked.
            if !unsafe { check(space, port, name, entry) } {
                // The port is locked and active.

                if notify.is_null() {
                    // SAFETY: the port is live and locked.
                    let previous =
                        unsafe { dncancel_if_requested(port, entry) };
                    // SAFETY: the port lock is held.
                    unsafe { port.unlock() };
                    // SAFETY: the space lock is held.
                    unsafe { space.lock_done() };
                    return Ok(previous);
                }

                // SAFETY: the port is live and locked.
                let previous = unsafe { dncancel_if_requested(port, entry) };

                // SAFETY: the port is live, locked, and active.
                match unsafe { ipc_port::dnrequest(port, name, notify) } {
                    Ok(request) => {
                        // SAFETY: the port lock is held.
                        unsafe { port.unlock() };
                        // SAFETY: the entry is live.
                        unsafe { (*entry).set_request(request) };
                        // SAFETY: the space lock is held.
                        unsafe { space.lock_done() };
                        return Ok(previous);
                    }
                    Err(_) => {
                        // SAFETY: the space lock is held.
                        unsafe { space.lock_done() };

                        // SAFETY: the port is live and locked; `dngrow`
                        // unlocks it and reports why it could not grow.
                        unsafe { ipc_port::dngrow(port) }?;
                        continue;
                    }
                }
            }

            // SAFETY: the entry is live.
            bits = unsafe { (*entry).bits() };
        }

        if bits & MACH_PORT_TYPE_DEAD_NAME != 0
            && immediate
            && !notify.is_null()
        {
            if urefs_overflow(bits & IE_BITS_UREFS_MASK, 1) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::UrefsOverflow);
            }

            // SAFETY: the entry is live and the space lock is held.
            unsafe { (*entry).set_bits(bits.wrapping_add(1)) };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            // SAFETY: the caller promises the live send-once right.
            unsafe { glue::ipc_notify_dead_name(notify, name) };
            return Ok(ptr::null_mut());
        }

        // SAFETY: the space lock is held.
        unsafe { space.lock_done() };

        return if bits & MACH_PORT_TYPE_PORT_OR_DEAD != 0 {
            Err(KernError::InvalidArgument)
        } else {
            Err(KernError::InvalidRight)
        };
    }
}

/// `ipc_right_dncancel()` in C: cancel the entry's dead-name request and
/// return the registered send-once right.
///
/// # Safety
///
/// `port` must be live and locked, and `entry`'s `ie_request` must name a live
/// request in the port's table.
pub(crate) unsafe fn dncancel(
    port: IpcPort,
    entry: *mut IpcEntry,
) -> *mut c_void {
    // SAFETY: the caller promises the live entry.
    let request = unsafe { (*entry).request() };
    // SAFETY: the entry is live.
    unsafe { (*entry).set_request(0) };

    // SAFETY: the caller promises the live locked port and the live request.
    unsafe { ipc_port::dncancel(port, request) }
}

/// The `ipc_right_dncancel_macro()` of <ipc/ipc_right.h>: `IP_NULL` unless the
/// entry holds a dead-name request.
///
/// # Safety
///
/// `port` must be live and locked, and `entry` must be a live entry of the
/// write-locked space.
unsafe fn dncancel_if_requested(
    port: IpcPort,
    entry: *mut IpcEntry,
) -> *mut c_void {
    // SAFETY: the caller promises the live entry.
    if unsafe { (*entry).request() } == 0 {
        return ptr::null_mut();
    }

    // SAFETY: the caller promises the live locked port and a nonzero request.
    unsafe { dncancel(port, entry) }
}

/// `ipc_right_inuse()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.
pub(crate) unsafe fn inuse(space: IpcSpace, entry: *mut IpcEntry) -> bool {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };

    if bits & IE_BITS_TYPE_MASK != MACH_PORT_TYPE_NONE {
        // SAFETY: the caller holds the space lock.
        unsafe { space.lock_done() };
        return true;
    }

    false
}

/// `ipc_right_check()` in C.
///
/// # Safety
///
/// The space must be write-locked.  On success the port is dead and converted
/// to a dead name; otherwise the port is live and locked.
pub(crate) unsafe fn check(
    space: IpcSpace,
    port: IpcPort,
    name: c_uint,
    entry: *mut IpcEntry,
) -> bool {
    // SAFETY: the caller promises a live port.
    unsafe { port.lock() };
    // SAFETY: the port lock is held.
    if unsafe { port.is_active() } {
        return false;
    }
    // SAFETY: the port lock is held.
    unsafe { port.unlock() };

    // SAFETY: the entry is live.
    let mut bits = unsafe { (*entry).bits() };

    if bits & MACH_PORT_TYPE_SEND != 0 {
        if bits & IE_BITS_MAREQUEST != 0 {
            bits &= !IE_BITS_MAREQUEST;

            // SAFETY: the caller holds the space lock.
            unsafe { glue::ipc_marequest_cancel(space.as_ptr(), name) };
        }

        // SAFETY: the caller holds the space lock.
        let _ = unsafe { space.reverse_remove(port.as_ptr()) };
    }

    // SAFETY: the port is dead, so its lock is free; this is the C's
    // `ipc_port_release()`.
    unsafe { port.release() };

    bits = (bits & !IE_BITS_TYPE_MASK) | MACH_PORT_TYPE_DEAD_NAME;

    // SAFETY: the entry is live.
    if unsafe { (*entry).request() } != 0 {
        // SAFETY: the entry is live.
        unsafe {
            (*entry).set_request(0);
            (*entry).set_bits(bits.wrapping_add(1));
            (*entry).set_object(ptr::null_mut());
        }
    } else {
        // SAFETY: the entry is live.
        unsafe { (*entry).set_bits(bits) };
        // SAFETY: the entry is live.
        unsafe { (*entry).set_object(ptr::null_mut()) };
    }

    true
}

/// `ipc_right_clean()` in C: release a dead space's entry.
///
/// # Safety
///
/// `entry` must be a live entry of a dead, unlocked space.
pub(crate) unsafe fn clean(name: c_uint, entry: *mut IpcEntry) {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };
    let type_ = bits & IE_BITS_TYPE_MASK;

    match type_ {
        MACH_PORT_TYPE_DEAD_NAME => (),

        MACH_PORT_TYPE_PORT_SET => {
            // SAFETY: a typed port-set entry names a live port set.
            let pset = unsafe { (*entry).object() };
            // SAFETY: the pset is live.
            let target = pset.cast::<IpcTarget>();
            // SAFETY: the pset is live and unlocked.
            unsafe { (*target).lock() };

            // SAFETY: the port set is live and locked; the destroy consumes
            // the entry's reference and unlocks.
            unsafe { glue::ipc_pset_destroy(pset) };
        }

        MACH_PORT_TYPE_SEND
        | MACH_PORT_TYPE_RECEIVE
        | MACH_PORT_TYPE_SEND_RECEIVE
        | MACH_PORT_TYPE_SEND_ONCE => {
            // SAFETY: a port-rights entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };

            // SAFETY: the port lock is held.
            if !unsafe { port.is_active() } {
                // SAFETY: the port is dead and its lock is held; this is the
                // C's `ip_release()` and `ip_check_unlock()`.
                unsafe {
                    port.decrement_references();
                    port.check_unlock();
                }
                return;
            }

            // SAFETY: the port is live and locked.
            let dnrequest = unsafe { dncancel_if_requested(port, entry) };

            let mut nsrequest = ptr::null_mut();
            let mut mscount = 0;

            if type_ & MACH_PORT_TYPE_SEND != 0 {
                // SAFETY: the port is live and locked.
                unsafe { port.decrement_srights() };
                // SAFETY: the port lock is held.
                if unsafe { port.srights() } == 0 {
                    // SAFETY: the port lock is held.
                    nsrequest = unsafe { port.nsrequest() };
                    if !nsrequest.is_null() {
                        // SAFETY: the port is live and locked.
                        unsafe { port.set_nsrequest(ptr::null_mut()) };
                        // SAFETY: the port lock is held.
                        mscount = unsafe { port.mscount() };
                    }
                }
            }

            if type_ & MACH_PORT_TYPE_RECEIVE != 0 {
                // SAFETY: the port is live and locked.
                unsafe { ipc_port::clear_receiver(port) };
                // SAFETY: the port is live and locked; the destroy consumes
                // the entry's reference and unlocks.
                unsafe { ipc_port::destroy(port) };
            } else if type_ & MACH_PORT_TYPE_SEND_ONCE != 0 {
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };

                // SAFETY: the notifications consume the send-once right.
                unsafe { glue::ipc_notify_send_once(port.as_ptr()) };
            } else {
                // SAFETY: the port is live and its lock is held; the release
                // consumes the entry's reference.
                unsafe {
                    port.decrement_references();
                    port.unlock();
                }
            }

            if !nsrequest.is_null() {
                // SAFETY: a nonzero no-senders request is a live send-once
                // right.
                unsafe { glue::ipc_notify_no_senders(nsrequest, mscount) };
            }

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }
        }

        _ => strange_rights(
            c"ipc_right_clean",
            c"ipc_right_clean: strange type",
        ),
    }
}

/// `ipc_right_destroy()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
pub(crate) unsafe fn destroy(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
) {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };
    let type_ = bits & IE_BITS_TYPE_MASK;

    match type_ {
        MACH_PORT_TYPE_DEAD_NAME => {
            // SAFETY: the caller promises the live locked space and entry.
            unsafe { ipc_entry::dealloc(space, name, entry) };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };
        }

        MACH_PORT_TYPE_PORT_SET => {
            // SAFETY: a typed port-set entry names a live port set.
            let pset = unsafe { (*entry).object() };

            // SAFETY: the entry is live and the space lock is held.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                ipc_entry::dealloc(space, name, entry);
            }

            // SAFETY: the pset is live.
            let target = pset.cast::<IpcTarget>();
            // SAFETY: the pset is live and unlocked.
            unsafe { (*target).lock() };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            // SAFETY: the port set is live and locked; the destroy consumes
            // the entry's reference and unlocks.
            unsafe { glue::ipc_pset_destroy(pset) };
        }

        MACH_PORT_TYPE_SEND
        | MACH_PORT_TYPE_RECEIVE
        | MACH_PORT_TYPE_SEND_RECEIVE
        | MACH_PORT_TYPE_SEND_ONCE => {
            // SAFETY: a port-rights entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            if bits & IE_BITS_MAREQUEST != 0 {
                // SAFETY: the caller holds the space lock.
                unsafe { glue::ipc_marequest_cancel(space.as_ptr(), name) };
            }

            if type_ == MACH_PORT_TYPE_SEND {
                // SAFETY: the caller holds the space lock.
                let _ = unsafe { space.reverse_remove(port.as_ptr()) };
            }

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };

            // SAFETY: the port lock is held.
            if !unsafe { port.is_active() } {
                // SAFETY: the port is dead and its lock is held; this is the
                // C's `ip_release()` and `ip_check_unlock()`, and the entry
                // is freed under the space lock.
                unsafe {
                    port.decrement_references();
                    port.check_unlock();
                    (*entry).set_request(0);
                    (*entry).set_object(ptr::null_mut());
                    ipc_entry::dealloc(space, name, entry);
                    space.lock_done();
                }
                return;
            }

            // SAFETY: the port is live and locked.
            let dnrequest = unsafe { dncancel_if_requested(port, entry) };

            // SAFETY: the entry is live and the space lock is held.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                ipc_entry::dealloc(space, name, entry);
                space.lock_done();
            }

            let mut nsrequest = ptr::null_mut();
            let mut mscount = 0;

            if type_ & MACH_PORT_TYPE_SEND != 0 {
                // SAFETY: the port is live and locked.
                unsafe { port.decrement_srights() };
                // SAFETY: the port lock is held.
                if unsafe { port.srights() } == 0 {
                    // SAFETY: the port lock is held.
                    nsrequest = unsafe { port.nsrequest() };
                    if !nsrequest.is_null() {
                        // SAFETY: the port is live and locked.
                        unsafe { port.set_nsrequest(ptr::null_mut()) };
                        // SAFETY: the port lock is held.
                        mscount = unsafe { port.mscount() };
                    }
                }
            }

            if type_ & MACH_PORT_TYPE_RECEIVE != 0 {
                // SAFETY: the port is live and locked.
                unsafe { ipc_port::clear_receiver(port) };
                // SAFETY: the port is live and locked; the destroy consumes
                // the entry's reference and unlocks.
                unsafe { ipc_port::destroy(port) };
            } else if type_ & MACH_PORT_TYPE_SEND_ONCE != 0 {
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };

                // SAFETY: the notifications consume the send-once right.
                unsafe { glue::ipc_notify_send_once(port.as_ptr()) };
            } else {
                // SAFETY: the port is live and its lock is held; the release
                // consumes the entry's reference.
                unsafe {
                    port.decrement_references();
                    port.unlock();
                }
            }

            if !nsrequest.is_null() {
                // SAFETY: a nonzero no-senders request is a live send-once
                // right.
                unsafe { glue::ipc_notify_no_senders(nsrequest, mscount) };
            }

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }
        }

        _ => strange_rights(
            c"ipc_right_destroy",
            c"ipc_right_destroy: strange type",
        ),
    }
}

/// The `dead_name:` label of the C `ipc_right_dealloc()`.
///
/// # Safety
///
/// The space must be write-locked, active, and live, and `entry` a live entry
/// of it whose type is `MACH_PORT_TYPE_DEAD_NAME`.
unsafe fn dealloc_dead_name(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
    bits: u32,
) {
    if bits & IE_BITS_UREFS_MASK == 1 {
        // SAFETY: the caller promises the live locked space and entry.
        unsafe { ipc_entry::dealloc(space, name, entry) };
    } else {
        // SAFETY: the caller promises the live entry.
        unsafe { (*entry).set_bits(bits.wrapping_sub(1)) };
    }

    // SAFETY: the caller holds the space lock.
    unsafe { space.lock_done() };
}

/// `ipc_right_dealloc()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
pub(crate) unsafe fn dealloc(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
) -> Result<(), KernError> {
    // SAFETY: the caller promises the live entry.
    let mut bits = unsafe { (*entry).bits() };
    let type_ = bits & IE_BITS_TYPE_MASK;

    match type_ {
        MACH_PORT_TYPE_DEAD_NAME => {
            // SAFETY: the caller promises the live locked space and entry.
            unsafe { dealloc_dead_name(space, name, entry, bits) };
            Ok(())
        }

        MACH_PORT_TYPE_SEND_ONCE => {
            // SAFETY: a send-once entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live.
            if unsafe { check(space, port, name, entry) } {
                // SAFETY: the check converted the entry to a dead name.
                bits = unsafe { (*entry).bits() };
                // SAFETY: the entry is a live dead name.
                unsafe { dealloc_dead_name(space, name, entry, bits) };
                return Ok(());
            }

            // The port is locked and active.

            // SAFETY: the port is live and locked.
            let dnrequest = unsafe { dncancel_if_requested(port, entry) };
            // SAFETY: the port lock is held.
            unsafe { port.unlock() };

            // SAFETY: the entry is live and the space lock is held.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                ipc_entry::dealloc(space, name, entry);
                space.lock_done();
            }

            // SAFETY: the notification consumes the send-once right (or its
            // reference).
            unsafe { glue::ipc_notify_send_once(port.as_ptr()) };

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }

            Ok(())
        }

        MACH_PORT_TYPE_SEND => {
            // SAFETY: a send entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live.
            if unsafe { check(space, port, name, entry) } {
                // SAFETY: the check converted the entry to a dead name.
                bits = unsafe { (*entry).bits() };
                // SAFETY: the entry is a live dead name.
                unsafe { dealloc_dead_name(space, name, entry, bits) };
                return Ok(());
            }

            // The port is locked and active.

            let mut dnrequest = ptr::null_mut();
            let mut nsrequest = ptr::null_mut();
            let mut mscount = 0;

            if bits & IE_BITS_UREFS_MASK == 1 {
                // SAFETY: the port is live and locked.
                unsafe { port.decrement_srights() };
                // SAFETY: the port lock is held.
                if unsafe { port.srights() } == 0 {
                    // SAFETY: the port lock is held.
                    nsrequest = unsafe { port.nsrequest() };
                    if !nsrequest.is_null() {
                        // SAFETY: the port is live and locked.
                        unsafe { port.set_nsrequest(ptr::null_mut()) };
                        // SAFETY: the port lock is held.
                        mscount = unsafe { port.mscount() };
                    }
                }

                // SAFETY: the port is live and locked.
                dnrequest = unsafe { dncancel_if_requested(port, entry) };

                // SAFETY: the caller holds the space lock.
                let _ = unsafe { space.reverse_remove(port.as_ptr()) };

                if bits & IE_BITS_MAREQUEST != 0 {
                    // SAFETY: the caller holds the space lock.
                    unsafe {
                        glue::ipc_marequest_cancel(space.as_ptr(), name)
                    };
                }

                // SAFETY: the port is live and its lock is held; the release
                // consumes the entry's reference.
                unsafe { port.decrement_references() };
                // SAFETY: the entry is live and the space lock is held.
                unsafe {
                    (*entry).set_object(ptr::null_mut());
                    ipc_entry::dealloc(space, name, entry);
                }
            } else {
                // SAFETY: the entry is live.
                unsafe { (*entry).set_bits(bits.wrapping_sub(1)) };
            }

            // SAFETY: the port is live and its lock is held.
            unsafe { port.unlock() };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            if !nsrequest.is_null() {
                // SAFETY: a nonzero no-senders request is a live send-once
                // right.
                unsafe { glue::ipc_notify_no_senders(nsrequest, mscount) };
            }

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }

            Ok(())
        }

        MACH_PORT_TYPE_SEND_RECEIVE => {
            // SAFETY: a send-receive entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };

            let mut nsrequest = ptr::null_mut();
            let mut mscount = 0;

            if bits & IE_BITS_UREFS_MASK == 1 {
                // SAFETY: the port is live and locked.
                unsafe { port.decrement_srights() };
                // SAFETY: the port lock is held.
                if unsafe { port.srights() } == 0 {
                    // SAFETY: the port lock is held.
                    nsrequest = unsafe { port.nsrequest() };
                    if !nsrequest.is_null() {
                        // SAFETY: the port is live and locked.
                        unsafe { port.set_nsrequest(ptr::null_mut()) };
                        // SAFETY: the port lock is held.
                        mscount = unsafe { port.mscount() };
                    }
                }

                // SAFETY: the entry is live.
                unsafe {
                    (*entry).set_bits(
                        bits & !(IE_BITS_UREFS_MASK | MACH_PORT_TYPE_SEND),
                    );
                }
            } else {
                // SAFETY: the entry is live.
                unsafe { (*entry).set_bits(bits.wrapping_sub(1)) };
            }

            // SAFETY: the port lock is held.
            unsafe { port.unlock() };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            if !nsrequest.is_null() {
                // SAFETY: a nonzero no-senders request is a live send-once
                // right.
                unsafe { glue::ipc_notify_no_senders(nsrequest, mscount) };
            }

            Ok(())
        }

        _ => {
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };
            Err(KernError::InvalidRight)
        }
    }
}

/// `ipc_right_delta()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
pub(crate) unsafe fn delta(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
    right: c_uint,
    delta: c_int,
) -> Result<(), KernError> {
    // SAFETY: the caller promises the live entry.
    let mut bits = unsafe { (*entry).bits() };

    match right {
        MACH_PORT_RIGHT_PORT_SET => {
            if bits & MACH_PORT_TYPE_PORT_SET == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            if delta == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Ok(());
            }

            if delta != -1 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidValue);
            }

            // SAFETY: a typed port-set entry names a live port set.
            let pset = unsafe { (*entry).object() };

            // SAFETY: the entry is live and the space lock is held.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                ipc_entry::dealloc(space, name, entry);
            }

            // SAFETY: the pset is live.
            let target = pset.cast::<IpcTarget>();
            // SAFETY: the pset is live and unlocked.
            unsafe { (*target).lock() };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            // SAFETY: the port set is live and locked; the destroy consumes
            // the entry's reference and unlocks.
            unsafe { glue::ipc_pset_destroy(pset) };

            Ok(())
        }

        MACH_PORT_RIGHT_RECEIVE => {
            if bits & MACH_PORT_TYPE_RECEIVE == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            if delta == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Ok(());
            }

            if delta != -1 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidValue);
            }

            if bits & IE_BITS_MAREQUEST != 0 {
                bits &= !IE_BITS_MAREQUEST;

                // SAFETY: the caller holds the space lock.
                unsafe { glue::ipc_marequest_cancel(space.as_ptr(), name) };
            }

            // SAFETY: a receive entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };

            let mut dnrequest = ptr::null_mut();

            if bits & MACH_PORT_TYPE_SEND != 0 {
                bits &= !IE_BITS_TYPE_MASK;
                bits |= MACH_PORT_TYPE_DEAD_NAME;

                // SAFETY: the entry is live.
                if unsafe { (*entry).request() } != 0 {
                    // SAFETY: the entry is live.
                    unsafe { (*entry).set_request(0) };
                    bits = bits.wrapping_add(1);
                }

                // SAFETY: the entry is live and the port is locked.
                unsafe {
                    (*entry).set_bits(bits);
                    (*entry).set_object(ptr::null_mut());
                }
            } else {
                // SAFETY: the port is live and locked.
                dnrequest = unsafe { dncancel_if_requested(port, entry) };

                // SAFETY: the entry is live and the space lock is held.
                unsafe {
                    (*entry).set_object(ptr::null_mut());
                    ipc_entry::dealloc(space, name, entry);
                }
            }

            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            // SAFETY: the port is live and locked.
            unsafe { ipc_port::clear_receiver(port) };
            // SAFETY: the port is live and locked; the destroy consumes the
            // entry's reference and unlocks.
            unsafe { ipc_port::destroy(port) };

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }

            Ok(())
        }

        MACH_PORT_RIGHT_SEND_ONCE => {
            if bits & MACH_PORT_TYPE_SEND_ONCE == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            if !(-1..=0).contains(&delta) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidValue);
            }
            // SAFETY: a send-once entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live.
            if unsafe { check(space, port, name, entry) } {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            // The port is locked and active.

            if delta == 0 {
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Ok(());
            }

            // SAFETY: the port is live and locked.
            let dnrequest = unsafe { dncancel_if_requested(port, entry) };
            // SAFETY: the port lock is held.
            unsafe { port.unlock() };

            // SAFETY: the entry is live and the space lock is held.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                ipc_entry::dealloc(space, name, entry);
                space.lock_done();
            }

            // SAFETY: the send-once notification consumes the entry's
            // reference.
            unsafe { glue::ipc_notify_send_once(port.as_ptr()) };

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }

            Ok(())
        }

        MACH_PORT_RIGHT_DEAD_NAME => {
            if bits & MACH_PORT_TYPE_SEND_RIGHTS != 0 {
                // SAFETY: a send-rights entry names a live port.
                let port = unsafe { IpcPort::from_raw((*entry).object()) };

                // SAFETY: the space is write-locked and the entry is live.
                if !unsafe { check(space, port, name, entry) } {
                    // SAFETY: the port is live and locked.
                    unsafe { port.unlock() };
                    // SAFETY: the space lock is held.
                    unsafe { space.lock_done() };
                    return Err(KernError::InvalidRight);
                }

                // SAFETY: the check converted the entry to a dead name.
                bits = unsafe { (*entry).bits() };
            } else if bits & MACH_PORT_TYPE_DEAD_NAME == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            let urefs = bits & IE_BITS_UREFS_MASK;

            if urefs_underflow(urefs, delta) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidValue);
            }

            if urefs_overflow(urefs, delta) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::UrefsOverflow);
            }

            if urefs.wrapping_add(delta as u32) == 0 {
                // SAFETY: the caller promises the live locked space and
                // entry.
                unsafe { ipc_entry::dealloc(space, name, entry) };
            } else {
                // SAFETY: the entry is live.
                unsafe { (*entry).set_bits(bits.wrapping_add(delta as u32)) };
            }

            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };
            Ok(())
        }

        MACH_PORT_RIGHT_SEND => {
            if bits & MACH_PORT_TYPE_SEND == 0 {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            // The maximum user-reference count for a send right is one short
            // of `MACH_PORT_UREFS_MAX`.
            let urefs = bits & IE_BITS_UREFS_MASK;
            if urefs_underflow(urefs, delta) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidValue);
            }

            if urefs_overflow(urefs.wrapping_add(1), delta) {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::UrefsOverflow);
            }

            // SAFETY: a send entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live.
            if unsafe { check(space, port, name, entry) } {
                // SAFETY: the space lock is held.
                unsafe { space.lock_done() };
                return Err(KernError::InvalidRight);
            }

            // The port is locked and active.

            let mut dnrequest = ptr::null_mut();
            let mut nsrequest = ptr::null_mut();
            let mut mscount = 0;

            if urefs.wrapping_add(delta as u32) == 0 {
                // SAFETY: the port is live and locked.
                unsafe { port.decrement_srights() };
                // SAFETY: the port lock is held.
                if unsafe { port.srights() } == 0 {
                    // SAFETY: the port lock is held.
                    nsrequest = unsafe { port.nsrequest() };
                    if !nsrequest.is_null() {
                        // SAFETY: the port is live and locked.
                        unsafe { port.set_nsrequest(ptr::null_mut()) };
                        // SAFETY: the port lock is held.
                        mscount = unsafe { port.mscount() };
                    }
                }

                if bits & MACH_PORT_TYPE_RECEIVE != 0 {
                    // SAFETY: the entry is live.
                    unsafe {
                        (*entry).set_bits(
                            bits & !(IE_BITS_UREFS_MASK | MACH_PORT_TYPE_SEND),
                        );
                    }
                } else {
                    // SAFETY: the port is live and locked.
                    dnrequest = unsafe { dncancel_if_requested(port, entry) };

                    // SAFETY: the caller holds the space lock.
                    let _ = unsafe { space.reverse_remove(port.as_ptr()) };

                    if bits & IE_BITS_MAREQUEST != 0 {
                        // SAFETY: the caller holds the space lock.
                        unsafe {
                            glue::ipc_marequest_cancel(space.as_ptr(), name)
                        };
                    }

                    // SAFETY: the port is live and its lock is held; the
                    // release consumes the entry's reference.
                    unsafe { port.decrement_references() };
                    // SAFETY: the entry is live and the space lock is held.
                    unsafe {
                        (*entry).set_object(ptr::null_mut());
                        ipc_entry::dealloc(space, name, entry);
                    }
                }
            } else {
                // SAFETY: the entry is live.
                unsafe { (*entry).set_bits(bits.wrapping_add(delta as u32)) };
            }

            // SAFETY: the port is live and its lock is held.
            unsafe { port.unlock() };
            // SAFETY: the space lock is held.
            unsafe { space.lock_done() };

            if !nsrequest.is_null() {
                // SAFETY: a nonzero no-senders request is a live send-once
                // right.
                unsafe { glue::ipc_notify_no_senders(nsrequest, mscount) };
            }

            if !dnrequest.is_null() {
                // SAFETY: a nonzero dead-name request is a live send-once
                // right.
                unsafe { glue::ipc_notify_port_deleted(dnrequest, name) };
            }

            Ok(())
        }

        _ => strange_rights(
            c"ipc_right_delta",
            c"ipc_right_delta: strange right",
        ),
    }
}

/// `ipc_right_info()` in C: the entry's type bits and user-reference count.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space stays locked.
pub(crate) unsafe fn info(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
) -> (c_uint, c_uint) {
    // SAFETY: the caller promises the live entry.
    let mut bits = unsafe { (*entry).bits() };

    if bits & MACH_PORT_TYPE_SEND_RIGHTS != 0 {
        // SAFETY: a send-rights entry names a live port.
        let port = unsafe { IpcPort::from_raw((*entry).object()) };

        // SAFETY: the space is write-locked and the entry is live.
        if unsafe { check(space, port, name, entry) } {
            // SAFETY: the check converted the entry to a dead name.
            bits = unsafe { (*entry).bits() };
        } else {
            // SAFETY: the port is live and locked.
            unsafe { port.unlock() };
        }
    }

    let mut type_ = bits & IE_BITS_TYPE_MASK;

    // SAFETY: the entry is live.
    if unsafe { (*entry).request() } != 0 {
        type_ |= MACH_PORT_TYPE_DNREQUEST;
    }

    if bits & IE_BITS_MAREQUEST != 0 {
        type_ |= MACH_PORT_TYPE_MAREQUEST;
    }

    (type_, bits & IE_BITS_UREFS_MASK)
}

/// `ipc_right_copyin_check()` in C.
///
/// # Safety
///
/// The space must be live, active, and locked for reading or writing, and
/// `entry` a live entry of it.
pub(crate) unsafe fn copyin_check(
    entry: *mut IpcEntry,
    msgt_name: c_uint,
) -> bool {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };

    match msgt_name {
        MACH_MSG_TYPE_MAKE_SEND
        | MACH_MSG_TYPE_MAKE_SEND_ONCE
        | MACH_MSG_TYPE_MOVE_RECEIVE => bits & MACH_PORT_TYPE_RECEIVE != 0,

        MACH_MSG_TYPE_COPY_SEND
        | MACH_MSG_TYPE_MOVE_SEND
        | MACH_MSG_TYPE_MOVE_SEND_ONCE => {
            if bits & MACH_PORT_TYPE_DEAD_NAME != 0 {
                return true;
            }

            if bits & MACH_PORT_TYPE_SEND_RIGHTS == 0 {
                return false;
            }

            // SAFETY: a send-rights entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };
            // SAFETY: the port lock is held.
            let active = unsafe { port.is_active() };
            // SAFETY: the port lock is held.
            unsafe { port.unlock() };

            if !active {
                return true;
            }

            if msgt_name == MACH_MSG_TYPE_MOVE_SEND_ONCE {
                bits & MACH_PORT_TYPE_SEND_ONCE != 0
            } else {
                bits & MACH_PORT_TYPE_SEND != 0
            }
        }

        _ => strange_rights(
            c"ipc_right_copyin_check",
            c"ipc_right_copyin_check: strange rights",
        ),
    }
}

/// The `copy_dead:` label of the C `ipc_right_copyin()`.
fn copy_dead(deadok: bool) -> Result<(*mut c_void, *mut c_void), KernError> {
    if !deadok {
        return Err(KernError::InvalidRight);
    }

    Ok((IO_DEAD, ptr::null_mut()))
}

/// The `move_dead:` label of the C `ipc_right_copyin()`.
///
/// # Safety
///
/// The space must be write-locked and `entry` a live entry of it whose type is
/// `MACH_PORT_TYPE_DEAD_NAME`.
unsafe fn move_dead(
    entry: *mut IpcEntry,
    bits: u32,
    deadok: bool,
) -> Result<(*mut c_void, *mut c_void), KernError> {
    if !deadok {
        return Err(KernError::InvalidRight);
    }

    let bits = if bits & IE_BITS_UREFS_MASK == 1 {
        bits & !MACH_PORT_TYPE_DEAD_NAME
    } else {
        bits.wrapping_sub(1)
    };

    // SAFETY: the caller promises the live entry.
    unsafe { (*entry).set_bits(bits) };

    Ok((IO_DEAD, ptr::null_mut()))
}

/// `ipc_right_copyin()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  On success the caller gets a reference for the object, unless it is
/// `IO_DEAD`.
pub(crate) unsafe fn copyin(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
    msgt_name: c_uint,
    deadok: bool,
) -> Result<(*mut c_void, *mut c_void), KernError> {
    // SAFETY: the caller promises the live entry.
    let mut bits = unsafe { (*entry).bits() };

    match msgt_name {
        MACH_MSG_TYPE_MAKE_SEND => {
            if bits & MACH_PORT_TYPE_RECEIVE == 0 {
                return Err(KernError::InvalidRight);
            }

            // SAFETY: a receive entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe {
                port.lock();
                port.increment_mscount();
                port.increment_srights();
                port.increment_references();
                port.unlock();
            }

            Ok((port.as_ptr(), ptr::null_mut()))
        }

        MACH_MSG_TYPE_MAKE_SEND_ONCE => {
            if bits & MACH_PORT_TYPE_RECEIVE == 0 {
                return Err(KernError::InvalidRight);
            }

            // SAFETY: a receive entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe {
                port.lock();
                port.increment_sorights();
                port.increment_references();
                port.unlock();
            }

            Ok((port.as_ptr(), ptr::null_mut()))
        }

        MACH_MSG_TYPE_MOVE_RECEIVE => {
            if bits & MACH_PORT_TYPE_RECEIVE == 0 {
                return Err(KernError::InvalidRight);
            }

            // SAFETY: a receive entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the port is live and unlocked.
            unsafe { port.lock() };

            let dnrequest;

            if bits & MACH_PORT_TYPE_SEND != 0 {
                // SAFETY: the entry is live.
                unsafe { (*entry).set_name(name) };
                // SAFETY: the caller holds the space lock.
                let _ = unsafe { space.reverse_insert(port.as_ptr(), entry) };
                // SAFETY: the port is live and locked.
                unsafe { port.increment_references() };
                dnrequest = ptr::null_mut();
            } else {
                // SAFETY: the port is live and locked.
                dnrequest = unsafe { dncancel_if_requested(port, entry) };

                if bits & IE_BITS_MAREQUEST != 0 {
                    // SAFETY: the caller holds the space lock.
                    unsafe {
                        glue::ipc_marequest_cancel(space.as_ptr(), name)
                    };
                }

                // SAFETY: the entry is live.
                unsafe { (*entry).set_object(ptr::null_mut()) };
            }

            // SAFETY: the entry is live and the port is locked; the C clears
            // the receiver before unlocking.
            unsafe {
                (*entry).set_bits(bits & !MACH_PORT_TYPE_RECEIVE);
                ipc_port::clear_receiver(port);
                port.set_receiver_name(MACH_PORT_NULL);
                port.set_destination(ptr::null_mut());
                port.clear_protected_flag();
                port.unlock();
            }

            Ok((port.as_ptr(), dnrequest))
        }

        MACH_MSG_TYPE_COPY_SEND
        | MACH_MSG_TYPE_MOVE_SEND
        | MACH_MSG_TYPE_MOVE_SEND_ONCE => {
            let copy = msgt_name == MACH_MSG_TYPE_COPY_SEND;

            if bits & MACH_PORT_TYPE_DEAD_NAME != 0 {
                return if copy {
                    copy_dead(deadok)
                } else {
                    // SAFETY: the caller promises the live entry.
                    unsafe { move_dead(entry, bits, deadok) }
                };
            }

            // Allow for dead send-once rights.
            if bits & MACH_PORT_TYPE_SEND_RIGHTS == 0 {
                return Err(KernError::InvalidRight);
            }

            // SAFETY: a send-rights entry names a live port.
            let port = unsafe { IpcPort::from_raw((*entry).object()) };

            // SAFETY: the space is write-locked and the entry is live.
            if unsafe { check(space, port, name, entry) } {
                // SAFETY: the check converted the entry to a dead name.
                bits = unsafe { (*entry).bits() };

                return if copy {
                    copy_dead(deadok)
                } else {
                    // SAFETY: the entry is a live dead name.
                    unsafe { move_dead(entry, bits, deadok) }
                };
            }

            // The port is locked and active.

            if copy {
                if bits & MACH_PORT_TYPE_SEND == 0 {
                    // SAFETY: the port lock is held.
                    unsafe { port.unlock() };
                    return Err(KernError::InvalidRight);
                }

                // SAFETY: the port is live and locked.
                unsafe {
                    port.increment_srights();
                    port.increment_references();
                    port.unlock();
                }

                return Ok((port.as_ptr(), ptr::null_mut()));
            }

            if msgt_name == MACH_MSG_TYPE_MOVE_SEND {
                if bits & MACH_PORT_TYPE_SEND == 0 {
                    // SAFETY: the port lock is held.
                    unsafe { port.unlock() };
                    return Err(KernError::InvalidRight);
                }

                let dnrequest;

                if bits & IE_BITS_UREFS_MASK == 1 {
                    if bits & MACH_PORT_TYPE_RECEIVE != 0 {
                        // SAFETY: the port is live and locked.
                        unsafe { port.increment_references() };
                        dnrequest = ptr::null_mut();
                    } else {
                        // SAFETY: the port is live and locked.
                        dnrequest =
                            unsafe { dncancel_if_requested(port, entry) };

                        // SAFETY: the caller holds the space lock.
                        let _ = unsafe { space.reverse_remove(port.as_ptr()) };

                        if bits & IE_BITS_MAREQUEST != 0 {
                            // SAFETY: the caller holds the space lock.
                            unsafe {
                                glue::ipc_marequest_cancel(
                                    space.as_ptr(),
                                    name,
                                )
                            };
                        }

                        // SAFETY: the entry is live.
                        unsafe { (*entry).set_object(ptr::null_mut()) };
                    }

                    // SAFETY: the entry is live.
                    unsafe {
                        (*entry).set_bits(
                            bits & !(IE_BITS_UREFS_MASK | MACH_PORT_TYPE_SEND),
                        );
                    }
                } else {
                    // SAFETY: the port is live and locked.
                    unsafe {
                        port.increment_srights();
                        port.increment_references();
                        (*entry).set_bits(bits.wrapping_sub(1));
                    }
                    dnrequest = ptr::null_mut();
                }

                // SAFETY: the port lock is held.
                unsafe { port.unlock() };

                return Ok((port.as_ptr(), dnrequest));
            }

            if bits & MACH_PORT_TYPE_SEND_ONCE == 0 {
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };
                return Err(KernError::InvalidRight);
            }

            // SAFETY: the port is live and locked.
            let dnrequest = unsafe { dncancel_if_requested(port, entry) };
            // SAFETY: the port lock is held.
            unsafe { port.unlock() };

            // SAFETY: the entry is live.
            unsafe {
                (*entry).set_object(ptr::null_mut());
                (*entry).set_bits(bits & !MACH_PORT_TYPE_SEND_ONCE);
            }

            Ok((port.as_ptr(), dnrequest))
        }

        _ => strange_rights(
            c"ipc_right_copyin",
            c"ipc_right_copyin: strange rights",
        ),
    }
}

/// `ipc_right_copyin_undo()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked; `entry` a live entry of
/// it; and `object` either `IO_DEAD` or a dead port the entry refers to.
pub(crate) unsafe fn copyin_undo(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
    msgt_name: c_uint,
    object: *mut c_void,
    soright: *mut c_void,
) {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };

    if !soright.is_null() {
        // SAFETY: the entry is live.
        unsafe {
            (*entry).set_bits(
                (bits & !IE_BITS_RIGHT_MASK) | MACH_PORT_TYPE_DEAD_NAME | 2,
            );
        }
    } else if bits & IE_BITS_TYPE_MASK == MACH_PORT_TYPE_NONE {
        // SAFETY: the entry is live.
        unsafe {
            (*entry).set_bits(
                (bits & !IE_BITS_RIGHT_MASK) | MACH_PORT_TYPE_DEAD_NAME | 1,
            );
        }
    } else if bits & IE_BITS_TYPE_MASK == MACH_PORT_TYPE_DEAD_NAME {
        if msgt_name != MACH_MSG_TYPE_COPY_SEND {
            // SAFETY: the entry is live.
            unsafe { (*entry).set_bits(bits.wrapping_add(1)) };
        }
    } else {
        if msgt_name != MACH_MSG_TYPE_COPY_SEND {
            // SAFETY: the entry is live.
            unsafe { (*entry).set_bits(bits.wrapping_add(1)) };
        }

        // The object is dead, so the check leaves the entry a dead name and
        // returns without keeping a lock.
        // SAFETY: the caller promises a dead port.
        let port = unsafe { IpcPort::from_raw(object) };
        // SAFETY: the space is write-locked and the entry is live.
        let _ = unsafe { check(space, port, name, entry) };
    }

    // The reference copyin acquired does not exist for `IO_DEAD`.
    if object != IO_DEAD {
        // SAFETY: the caller promises the object the copyin referenced.
        unsafe { ipc_object::release(object) };
    }
}

/// `ipc_right_copyin_two()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  On success the object is returned with two references.
pub(crate) unsafe fn copyin_two(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
) -> Result<(*mut c_void, *mut c_void), KernError> {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };

    if bits & MACH_PORT_TYPE_SEND == 0 {
        return Err(KernError::InvalidRight);
    }

    let urefs = bits & IE_BITS_UREFS_MASK;
    if urefs < 2 {
        return Err(KernError::InvalidRight);
    }

    // SAFETY: a send entry names a live port.
    let port = unsafe { IpcPort::from_raw((*entry).object()) };

    // SAFETY: the space is write-locked and the entry is live.
    if unsafe { check(space, port, name, entry) } {
        return Err(KernError::InvalidRight);
    }

    // The port is locked and active.

    let dnrequest;

    if urefs == 2 {
        if bits & MACH_PORT_TYPE_RECEIVE != 0 {
            // SAFETY: the port is live and locked.
            unsafe {
                port.increment_srights();
                port.increment_references();
                port.increment_references();
            }
            dnrequest = ptr::null_mut();
        } else {
            // SAFETY: the port is live and locked.
            dnrequest = unsafe { dncancel_if_requested(port, entry) };

            // SAFETY: the caller holds the space lock.
            let _ = unsafe { space.reverse_remove(port.as_ptr()) };

            if bits & IE_BITS_MAREQUEST != 0 {
                // SAFETY: the caller holds the space lock.
                unsafe { glue::ipc_marequest_cancel(space.as_ptr(), name) };
            }

            // SAFETY: the port is live and locked.
            unsafe {
                port.increment_srights();
                port.increment_references();
                (*entry).set_object(ptr::null_mut());
            }
        }

        // SAFETY: the entry is live.
        unsafe {
            (*entry)
                .set_bits(bits & !(IE_BITS_UREFS_MASK | MACH_PORT_TYPE_SEND));
        }
    } else {
        // SAFETY: the port is live and locked.
        unsafe {
            port.increment_srights();
            port.increment_srights();
            port.increment_references();
            port.increment_references();
            (*entry).set_bits(bits.wrapping_sub(2));
        }
        dnrequest = ptr::null_mut();
    }

    // SAFETY: the port lock is held.
    unsafe { port.unlock() };

    Ok((port.as_ptr(), dnrequest))
}

/// `ipc_right_copyout()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The object must be live, active, and locked; it is unlocked on
/// return, and the call consumes a reference on success.
pub(crate) unsafe fn copyout(
    space: IpcSpace,
    name: c_uint,
    entry: *mut IpcEntry,
    msgt_name: c_uint,
    overflow: bool,
    object: *mut c_void,
) -> Result<(), KernError> {
    // SAFETY: the caller promises the live entry.
    let bits = unsafe { (*entry).bits() };
    // SAFETY: the caller promises the live locked object.
    let port = unsafe { IpcPort::from_raw(object) };

    match msgt_name {
        MACH_MSG_TYPE_MOVE_SEND_ONCE => {
            // SAFETY: the port lock is held; the send-once right and its
            // reference transfer to the entry.
            unsafe { port.unlock() };
            // SAFETY: the entry is live.
            unsafe {
                (*entry).set_bits(bits | (MACH_PORT_TYPE_SEND_ONCE | 1));
            }
            Ok(())
        }

        MACH_MSG_TYPE_MOVE_SEND => {
            if bits & MACH_PORT_TYPE_SEND != 0 {
                let urefs = bits & IE_BITS_UREFS_MASK;

                if urefs.wrapping_add(1) == MACH_PORT_UREFS_MAX {
                    if overflow {
                        // Leave the user references pegged to the maximum.
                        // SAFETY: the port is live and locked.
                        unsafe {
                            port.decrement_srights();
                            port.decrement_references();
                            port.unlock();
                        }
                        return Ok(());
                    }

                    // SAFETY: the port lock is held.
                    unsafe { port.unlock() };
                    return Err(KernError::UrefsOverflow);
                }

                // SAFETY: the port is live and locked.
                unsafe {
                    port.decrement_srights();
                    port.decrement_references();
                    port.unlock();
                }
            } else if bits & MACH_PORT_TYPE_RECEIVE != 0 {
                // The send right transfers to the entry.
                // SAFETY: the port is live and locked.
                unsafe {
                    port.decrement_references();
                    port.unlock();
                }
            } else {
                // The send right and its reference transfer to the entry.
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };
                // SAFETY: the entry is live and the space is write-locked.
                unsafe {
                    (*entry).set_name(name);
                    let _ = space.reverse_insert(port.as_ptr(), entry);
                }
            }

            // SAFETY: the entry is live.
            unsafe {
                (*entry)
                    .set_bits((bits | MACH_PORT_TYPE_SEND).wrapping_add(1));
            }
            Ok(())
        }

        MACH_MSG_TYPE_MOVE_RECEIVE => {
            // SAFETY: the port is live and locked.
            let dest = unsafe { port.destination() };

            // SAFETY: the port is live and locked.
            unsafe {
                port.set_receiver_name(name);
                port.set_receiver(space.as_ptr());
                port.clear_protected_flag();
            }

            if bits & MACH_PORT_TYPE_SEND != 0 {
                // SAFETY: the port is live and locked.
                unsafe {
                    port.decrement_references();
                    port.unlock();
                }

                // SAFETY: the entry holds a reference, so the port is alive.
                let _ = unsafe { space.reverse_remove(port.as_ptr()) };
            } else {
                // The reference transfers to the entry.
                // SAFETY: the port lock is held.
                unsafe { port.unlock() };
            }

            // SAFETY: the entry is live.
            unsafe { (*entry).set_bits(bits | MACH_PORT_TYPE_RECEIVE) };

            if !dest.is_null() {
                // SAFETY: a nonzero destination is a live port holding a
                // reference.
                unsafe { ipc_object::release(dest) };
            }

            Ok(())
        }

        _ => strange_rights(
            c"ipc_right_copyout",
            c"ipc_right_copyout: strange rights",
        ),
    }
}

/// `ipc_right_rename()` in C.
///
/// # Safety
///
/// The space must be live, active, and write-locked; `oentry` and `nentry`
/// live entries of it, with `nentry` unused.  The space is unlocked on return.
pub(crate) unsafe fn rename(
    space: IpcSpace,
    oname: c_uint,
    oentry: *mut IpcEntry,
    nname: c_uint,
    nentry: *mut IpcEntry,
) {
    // SAFETY: the caller promises the live entries.
    let mut bits = unsafe { (*oentry).bits() };
    // SAFETY: as above.
    let mut request = unsafe { (*oentry).request() };
    // SAFETY: as above.
    let mut object = unsafe { (*oentry).object() };

    if request != 0 {
        // SAFETY: a request entry names a live port.
        let port = unsafe { IpcPort::from_raw(object) };

        // SAFETY: the space is write-locked and the entry is live.
        if unsafe { check(space, port, oname, oentry) } {
            // SAFETY: the check converted the entry to a dead name.
            bits = unsafe { (*oentry).bits() };
            request = 0;
            object = ptr::null_mut();
        } else {
            // The port is locked and active.  This is the
            // `ipc_port_dnrename()` macro of <ipc/ipc_port.h>.
            // SAFETY: the port is live and locked, and the request names a
            // live slot.
            unsafe { ipc_port::dnrename(port, request, nname) };
            // SAFETY: the port lock is held.
            unsafe { port.unlock() };
            // SAFETY: the entry is live.
            unsafe { (*oentry).set_request(0) };
        }
    }

    if bits & IE_BITS_MAREQUEST != 0 {
        // SAFETY: the caller holds the space lock.
        unsafe { glue::ipc_marequest_rename(space.as_ptr(), oname, nname) };
    }

    // SAFETY: the new entry is live and unused.
    unsafe {
        (*nentry).or_bits(bits & IE_BITS_RIGHT_MASK);
        (*nentry).set_request(request);
        (*nentry).set_object(object);
    }

    match bits & IE_BITS_TYPE_MASK {
        MACH_PORT_TYPE_SEND => {
            // SAFETY: a send entry names a live port.
            let port = unsafe { IpcPort::from_raw(object) };

            // SAFETY: the caller holds the space lock.
            let _ = unsafe { space.reverse_remove(port.as_ptr()) };
            // SAFETY: the new entry is live.
            unsafe { (*nentry).set_name(nname) };
            // SAFETY: the caller holds the space lock.
            let _ = unsafe { space.reverse_insert(port.as_ptr(), nentry) };
        }

        MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND_RECEIVE => {
            // SAFETY: a receive entry names a live port.
            let port = unsafe { IpcPort::from_raw(object) };

            // SAFETY: the port is live and unlocked.
            unsafe {
                port.lock();
                port.set_receiver_name(nname);
                port.unlock();
            }
        }

        MACH_PORT_TYPE_PORT_SET => {
            // SAFETY: a port-set entry names a live port set.
            let target = object.cast::<IpcTarget>();

            // SAFETY: the port set is live and unlocked.
            unsafe {
                (*target).lock();
                (*target).set_local_name(nname);
                (*target).unlock();
            }
        }

        MACH_PORT_TYPE_SEND_ONCE | MACH_PORT_TYPE_DEAD_NAME => (),

        _ => strange_rights(
            c"ipc_right_rename",
            c"ipc_right_rename: strange rights",
        ),
    }

    // SAFETY: the old entry is live and the space lock is held.
    unsafe {
        (*oentry).set_object(ptr::null_mut());
        ipc_entry::dealloc(space, oname, oentry);
        space.lock_done();
    }
}
