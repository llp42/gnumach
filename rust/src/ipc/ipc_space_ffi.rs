// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_space.c and ipc/ipc_space.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the space module, one adapter per symbol
//! `ipc/ipc_space.c` used to define and `ipc/ipc_space.h` declares.

use crate::ipc::IpcSpace;
use crate::ipc::ipc_space;
use core::ffi::{c_int, c_void};

/// `ipc_space_reference()` of ipc/ipc_space.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_space_reference(space: *mut c_void) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_space::reference(space) };
}

/// `ipc_space_release()` of ipc/ipc_space.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space` holding a reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_space_release(space: *mut c_void) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_space::release(space) };
}

/// `ipc_space_create()` of ipc/ipc_space.c.
///
/// # Safety
///
/// `spacep` must be writable storage for one space pointer, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_space_create(spacep: *mut *mut c_void) -> c_int {
    match ipc_space::create() {
        Ok(space) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { spacep.write(space.as_ptr()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_space_create_special()` of ipc/ipc_space.c.
///
/// # Safety
///
/// `spacep` must be writable storage for one space pointer, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_space_create_special(
    spacep: *mut *mut c_void,
) -> c_int {
    match ipc_space::create_special() {
        Ok(space) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { spacep.write(space.as_ptr()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_space_destroy()` of ipc/ipc_space.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space` and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_space_destroy(space: *mut c_void) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_space::destroy(space) };
}
