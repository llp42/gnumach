// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_pset.c and ipc/ipc_pset.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the port sets, one adapter per symbol
//! `ipc/ipc_pset.c` used to define and `ipc/ipc_pset.h` declares.

use crate::ipc::ipc_pset;
use crate::ipc::{IpcPort, IpcSpace, IpcTarget};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_pset_alloc()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, nothing may be locked, and `namep` and
/// `psetp` must be writable storage for one name and one port set pointer,
/// written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_alloc(
    space: *mut c_void,
    namep: *mut c_uint,
    psetp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_pset::alloc(space) } {
        Ok((name, pset)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                namep.write(name);
                psetp.write(pset.cast());
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_pset_alloc_name()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, nothing may be locked, and `psetp`
/// must be writable storage for one port set pointer, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_alloc_name(
    space: *mut c_void,
    name: c_uint,
    psetp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_pset::alloc_name(space, name) } {
        Ok(pset) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { psetp.write(pset.cast()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_pset_add()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `pset` and `port` must be live port sets and ports, both locked, the port
/// must not be in a set, and the set's owner must be the port's receiver.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_add(pset: *mut c_void, port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_pset::add(pset.cast::<IpcTarget>(), port) };
}

/// `ipc_pset_remove()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `pset` and `port` must be live, both locked, and the port must be active
/// and a member of the set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_remove(
    pset: *mut c_void,
    port: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_pset::remove(pset.cast::<IpcTarget>(), port) };
}

/// `ipc_pset_move()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `space` must be a live read-locked `ipc_space`, `port` a live port, and
/// `nset` `IPS_NULL` or a live port set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_move(
    space: *mut c_void,
    port: *mut c_void,
    nset: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    match unsafe {
        ipc_pset::move_between(space, port, nset.cast::<IpcTarget>())
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_pset_destroy()` of ipc/ipc_pset.c.
///
/// # Safety
///
/// `pset` must be a live, locked, active port set, and the caller's reference
/// is consumed; on return the set is unlocked and dead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_destroy(pset: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_pset::destroy(pset.cast::<IpcTarget>()) };
}
