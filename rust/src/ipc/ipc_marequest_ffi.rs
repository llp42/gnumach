// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_marequest.c and ipc/ipc_marequest.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the msg-accepted requests, one adapter per
//! symbol `ipc/ipc_marequest.c` used to define and `ipc/ipc_marequest.h`
//! declares.

use crate::ipc::ipc_marequest;
use crate::ipc::{HashInfoBucket, IpcMarequest, IpcPort, IpcSpace};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_marequest_init()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `ipc_bootstrap()` must call this once, before any request is created.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_init() {
    // SAFETY: the caller's contract.
    unsafe { ipc_marequest::init() };
}

/// `ipc_marequest_create()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space` with nothing locked, `port` a live
/// port, `notify` a name in `space`, and `marequestp` writable storage for
/// one request pointer, written only on success; may allocate memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_create(
    space: *mut c_void,
    port: *mut c_void,
    notify: c_uint,
    marequestp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_marequest::create(space, port, notify) } {
        Ok(marequest) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { marequestp.write(marequest.cast()) };
            0
        }
        Err(code) => code.raw(),
    }
}

/// `ipc_marequest_cancel()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, write-locked and active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_cancel(
    space: *mut c_void,
    name: c_uint,
) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_marequest::cancel(space, name) };
}

/// `ipc_marequest_rename()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, write-locked and active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_rename(
    space: *mut c_void,
    old: c_uint,
    new: c_uint,
) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_marequest::rename(space, old, new) };
}

/// `ipc_marequest_destroy()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `marequest` must be a live request that nothing else can reach; nothing
/// may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_destroy(marequest: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_marequest::destroy(marequest.cast::<IpcMarequest>()) };
}

/// `ipc_marequest_info()` of ipc/ipc_marequest.c.
///
/// # Safety
///
/// `maxp` must be writable storage for one count, and `info` writable
/// storage for `count` bucket records.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_marequest_info(
    maxp: *mut c_uint,
    info: *mut HashInfoBucket,
    count: c_uint,
) -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_marequest::info(maxp, info, count) }
}
