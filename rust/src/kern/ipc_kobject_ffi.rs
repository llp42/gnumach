// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_kobject.c and kern/ipc_kobject.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/ipc_kobject.c`, which the file used to define.

use crate::arch::types::VmOffset;
use crate::ipc::ipc_kmsg::Kmsg;
use crate::kern::ipc_kobject;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr;

/// `ipc_kobject_server()` of kern/ipc_kobject.c.
///
/// # Safety
///
/// `request` must be a live kernel message the caller owns, and nothing may
/// be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kobject_server(
    request: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller promises the live message.
    let request = unsafe { Kmsg::from_raw(request) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_kobject::server(request) } {
        Some(reply) => reply.as_ptr(),
        None => ptr::null_mut(),
    }
}

/// `ipc_kobject_set()` of kern/ipc_kobject.c.
///
/// # Safety
///
/// `port` must be a live, active port and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kobject_set(
    port: *mut c_void,
    kobject: VmOffset,
    type_: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_kobject::set(port, kobject, type_) };
}

/// `ipc_kobject_set_locked()` of kern/ipc_kobject.c.
///
/// # Safety
///
/// `port` must be a live, active port whose lock the caller holds.  It stays
/// locked after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kobject_set_locked(
    port: *mut c_void,
    kobject: VmOffset,
    type_: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_kobject::set_locked(port, kobject, type_) };
}

/// `ipc_kobject_destroy()` of kern/ipc_kobject.c.
///
/// # Safety
///
/// `port` must be a live but inactive port that nothing holds a lock on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kobject_destroy(port: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_kobject::destroy(port) };
}

/// `ipc_kobject_notify()` of kern/ipc_kobject.c.
///
/// # Safety
///
/// `request_header` must be a live notification request and `reply_header` a
/// writable MIG reply header.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kobject_notify(
    request_header: *mut c_void,
    reply_header: *mut c_void,
) -> c_int {
    // SAFETY: the caller promises a live request and a writable reply, both
    // shaped as `mach_msg_header_t`, which the C header declared.
    c_int::from(unsafe {
        ipc_kobject::notify(request_header.cast(), reply_header.cast())
    })
}
