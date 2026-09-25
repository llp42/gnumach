// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_right.c and ipc/ipc_right.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the capability module, one adapter per symbol
//! `ipc/ipc_right.c` used to define and `ipc/ipc_right.h` declares.

use crate::ipc::ipc_right;
use crate::ipc::{IpcPort, IpcSpace};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_right_lookup_write()` of ipc/ipc_right.c.
///
/// # Safety
///
/// `space` must be live and unlocked, and `entryp` writable storage for one
/// entry pointer, written only on success.  On success the space is
/// write-locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_lookup_write(
    space: *mut c_void,
    name: c_uint,
    entryp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::lookup_write(space, name) } {
        Ok(entry) => {
            // SAFETY: the caller promises `entryp` is writable; the C wrote
            // it only on success.
            unsafe { entryp.write(entry.cast()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_reverse()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live and locked for reading or writing; `object` must be
/// a live port; `namep` and `entryp` must be writable storage for one name
/// and one entry pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_reverse(
    space: *mut c_void,
    object: *mut c_void,
    namep: *mut c_uint,
    entryp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::reverse(space, object) } {
        Some((name, entry)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                namep.write(name);
                entryp.write(entry.cast());
            }
            1
        }
        None => 0,
    }
}

/// `ipc_right_dnrequest()` of ipc/ipc_right.c.
///
/// # Safety
///
/// `space` must be live and unlocked, `notify` must be `IP_NULL` or a live
/// send-once right, and `previousp` must be writable storage for one port
/// pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_dnrequest(
    space: *mut c_void,
    name: c_uint,
    immediate: c_int,
    notify: *mut c_void,
    previousp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::dnrequest(space, name, immediate != 0, notify) }
    {
        Ok(previous) => {
            // SAFETY: the caller promises `previousp` is writable; the C
            // wrote it only on success.
            unsafe { previousp.write(previous) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_dncancel()` of ipc/ipc_right.c.  Its `space` and `name`
/// arguments are unused.
///
/// # Safety
///
/// `port` must be live and locked, and `entry`'s `ie_request` must name a live
/// request in the port's table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_dncancel(
    _space: *mut c_void,
    port: *mut c_void,
    _name: c_uint,
    entry: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_right::dncancel(port, entry.cast()) }
}

/// `ipc_right_inuse()` of ipc/ipc_right.c.  Its `name` argument is unused.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked when the entry is in use.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_inuse(
    space: *mut c_void,
    _name: c_uint,
    entry: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    c_int::from(unsafe { ipc_right::inuse(space, entry.cast()) })
}

/// `ipc_right_check()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be write-locked, `port` live, and `entry` a live entry of
/// the space.  On success the port is dead and converted to a dead name;
/// otherwise the port is live and locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_check(
    space: *mut c_void,
    port: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    c_int::from(unsafe { ipc_right::check(space, port, name, entry.cast()) })
}

/// `ipc_right_clean()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, dead, and unlocked, and `entry` a live entry of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_clean(
    _space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_right::clean(name, entry.cast()) };
}
/// `ipc_right_destroy()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_destroy(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_right::destroy(space, name, entry.cast()) };
    0
}

/// `ipc_right_dealloc()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_dealloc(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::dealloc(space, name, entry.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_delta()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The space is unlocked on return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_delta(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    right: c_uint,
    delta: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::delta(space, name, entry.cast(), right, delta) }
    {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_info()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  `typep` and `urefsp` must be writable storage for one word each.
/// The space stays locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_info(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    typep: *mut c_uint,
    urefsp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    let (type_, urefs) = unsafe { ipc_right::info(space, name, entry.cast()) };

    // SAFETY: the caller promises both out-pointers are writable; the C
    // fills them on success.
    unsafe {
        typep.write(type_);
        urefsp.write(urefs);
    }
    0
}

/// `ipc_right_copyin_check()` of ipc/ipc_right.c.  Its `space` and `name`
/// arguments are unused.
///
/// # Safety
///
/// The space must be live, active, and locked for reading or writing, and
/// `entry` a live entry of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_copyin_check(
    _space: *mut c_void,
    _name: c_uint,
    entry: *mut c_void,
    msgt_name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { ipc_right::copyin_check(entry.cast(), msgt_name) })
}

/// `ipc_right_copyin()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it; `objectp` and `sorightp` must be writable storage for one pointer
/// each, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_copyin(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    msgt_name: c_uint,
    deadok: c_int,
    objectp: *mut *mut c_void,
    sorightp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe {
        ipc_right::copyin(space, name, entry.cast(), msgt_name, deadok != 0)
    } {
        Ok((object, soright)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                objectp.write(object);
                sorightp.write(soright);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_copyin_undo()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked; `entry` a live entry of
/// it; and `object` either `IO_DEAD` or a dead port the entry refers to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_copyin_undo(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    msgt_name: c_uint,
    object: *mut c_void,
    soright: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe {
        ipc_right::copyin_undo(
            space,
            name,
            entry.cast(),
            msgt_name,
            object,
            soright,
        );
    }
}

/// `ipc_right_copyin_two()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it; `objectp` and `sorightp` must be writable storage for one pointer
/// each, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_copyin_two(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    objectp: *mut *mut c_void,
    sorightp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_right::copyin_two(space, name, entry.cast()) } {
        Ok((object, soright)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                objectp.write(object);
                sorightp.write(soright);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_copyout()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked, and `entry` a live entry
/// of it.  The object must be live, active, and locked; it is unlocked on
/// return, and the call consumes a reference on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_copyout(
    space: *mut c_void,
    name: c_uint,
    entry: *mut c_void,
    msgt_name: c_uint,
    overflow: c_int,
    object: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe {
        ipc_right::copyout(
            space,
            name,
            entry.cast(),
            msgt_name,
            overflow != 0,
            object,
        )
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_right_rename()` of ipc/ipc_right.c.
///
/// # Safety
///
/// The space must be live, active, and write-locked; `oentry` and `nentry`
/// live entries of it, with `nentry` unused.  The space is unlocked on return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_right_rename(
    space: *mut c_void,
    oname: c_uint,
    oentry: *mut c_void,
    nname: c_uint,
    nentry: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe {
        ipc_right::rename(space, oname, oentry.cast(), nname, nentry.cast());
    }
    0
}
