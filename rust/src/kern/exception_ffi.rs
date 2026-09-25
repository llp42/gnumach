// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/exception.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from kern/exception.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `kern/exception.c` symbols C still calls, over the cores in
//! [`crate::kern::exception`].

use crate::ipc::IpcPort;
use crate::ipc::ipc_kmsg::Kmsg;
use crate::kern::exception as exception_core;
use core::ffi::{c_int, c_long, c_uint, c_void};
use core::ptr::NonNull;

/// `exception()` of kern/exception.c.
///
/// # Safety
///
/// The caller must be the exception path with nothing locked and no resources
/// held; the trap and FPU handlers are the callers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception(
    exception: c_int,
    code: c_int,
    subcode: c_long,
) -> ! {
    // SAFETY: the caller's contract.
    unsafe { exception_core::exception(exception, code, subcode) }
}

/// `exception_try_task()` of kern/exception.c.
///
/// # Safety
///
/// The caller must be the exception path with nothing locked and no resources
/// held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_try_task(
    exception: c_int,
    code: c_int,
    subcode: c_long,
) -> ! {
    // SAFETY: the caller's contract.
    unsafe { exception_core::try_task(exception, code, subcode) }
}

/// `exception_no_server()` of kern/exception.c.
///
/// # Safety
///
/// The caller must be the exception path with nothing locked and no resources
/// held; `kern/exception.c` is the only caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_no_server() -> ! {
    // SAFETY: the caller's contract.
    unsafe { exception_core::no_server() }
}

/// `exception_raise()` of kern/exception.c.
///
/// # Safety
///
/// `dest_port`, `thread_port` and `task_port` must be live naked send rights
/// the call consumes; nothing may be locked, and the caller runs in an
/// exception context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_raise(
    dest_port: *mut c_void,
    thread_port: *mut c_void,
    task_port: *mut c_void,
    exception: c_int,
    code: c_int,
    subcode: c_long,
) -> ! {
    // SAFETY: the caller's contract.
    unsafe {
        exception_core::raise(
            dest_port,
            thread_port,
            task_port,
            exception,
            code,
            subcode,
        )
    }
}

/// `exception_parse_reply()` of kern/exception.c.
///
/// # Safety
///
/// `kmsg` must be a live reply message whose right and buffer this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_parse_reply(kmsg: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { exception_core::parse_reply(Kmsg::from_raw(kmsg)) }
}

/// `exception_raise_continue()` of kern/exception.c; the receive path passes
/// it as its continuation.
///
/// # Safety
///
/// Runs as the continuation `exception_raise()` left in the thread, with
/// `ith_port` naming the live reply port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_raise_continue() {
    // SAFETY: the caller's contract.
    unsafe { exception_core::raise_continue() }
}

/// `exception_raise_continue_slow()` of kern/exception.c.
///
/// # Safety
///
/// `kmsg` must be `IKM_NULL` or the live message the receive handed over;
/// nothing may be locked, and the caller holds the reply port's reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_raise_continue_slow(
    mr: c_int,
    kmsg: *mut c_void,
    seqno: c_uint,
) -> ! {
    let kmsg = NonNull::new(kmsg).map(|_| {
        // SAFETY: the caller promises a live message when it is non-null.
        unsafe { Kmsg::from_raw(kmsg) }
    });
    // SAFETY: the caller's contract.
    unsafe { exception_core::continue_slow(mr, kmsg, seqno) }
}

/// `exception_raise_continue_fast()` of kern/exception.c.
///
/// # Safety
///
/// `reply_port` must be a live port whose lock this call holds, together with
/// one send-once right and two references; `kmsg` must be the live reply
/// message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_raise_continue_fast(
    reply_port: *mut c_void,
    kmsg: *mut c_void,
) -> ! {
    // SAFETY: the caller's contract.
    unsafe {
        exception_core::raise_continue_fast(
            IpcPort::from_raw(reply_port),
            Kmsg::from_raw(kmsg),
        )
    }
}
