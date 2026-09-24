// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_entry.c and ipc/ipc_entry.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the entry module, one adapter per symbol
//! `ipc/ipc_entry.c` used to define and `ipc/ipc_entry.h` declares.

use crate::ipc::IpcSpace;
use crate::ipc::ipc_entry;
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_entry_alloc()` of ipc/ipc_entry.c.
///
/// # Safety
///
/// `space` must be a live write-locked `ipc_space`, and `namep` and `entryp`
/// writable storage for one name and one entry pointer, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_entry_alloc(
    space: *mut c_void,
    namep: *mut c_uint,
    entryp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_entry::alloc(space) } {
        Ok((name, entry)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                namep.write(name);
                entryp.write(entry.cast());
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_entry_alloc_name()` of ipc/ipc_entry.c.
///
/// # Safety
///
/// `space` must be a live write-locked `ipc_space`, and `entryp` writable
/// storage for one entry pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_entry_alloc_name(
    space: *mut c_void,
    name: c_uint,
    entryp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_entry::alloc_name(space, name) } {
        Ok(entry) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { entryp.write(entry.cast()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}
