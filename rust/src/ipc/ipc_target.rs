// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_target.c and ipc/ipc_target.h:
//   Copyright (c) 1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The common part of IPC ports and port sets, which `ipc/ipc_target.c`
//! defines and `ipc/ipc_target.h` declares.

use crate::glue;
use crate::ipc::IpcTarget;
use core::ffi::{c_uint, c_void};

/// `ipc_target_init()` in C.
///
/// # Safety
///
/// `target` must be a fresh `struct ipc_target` this call initializes.
pub(crate) unsafe fn init(target: *mut IpcTarget, name: c_uint) {
    // SAFETY: the caller promises the fresh target.
    unsafe {
        (*target).name = name;
        glue::ipc_mqueue_init((*target).messages().cast());
    }
}

/// `ipc_target_init()` of ipc/ipc_target.c.
///
/// # Safety
///
/// `target` must be a fresh, live `struct ipc_target`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_target_init(target: *mut c_void, name: c_uint) {
    // SAFETY: the caller's contract.
    unsafe { init(target.cast::<IpcTarget>(), name) };
}

/// `ipc_target_terminate()` of ipc/ipc_target.c.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_target_terminate(_ipt: *mut c_void) {}
