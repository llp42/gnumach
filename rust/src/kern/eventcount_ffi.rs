// SPDX-License-Identifier: CMU-Mach
// Derived from kern/eventcount.c and kern/eventcount.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries `kern/eventcount.c` used to define.

use crate::kern::eventcount::{self, EventCounter};
use crate::kern::thread::Thread;
use core::ffi::{c_int, c_uint};

/// `evc_init()` of kern/eventcount.c.
///
/// # Safety
///
/// `ev` must point at writable storage for an [`EventCounter`] that no other
/// thread can see yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_init(ev: *mut EventCounter) {
    // SAFETY: the caller's contract.
    unsafe { eventcount::init(ev) };
}

/// `evc_destroy()` of kern/eventcount.c.
///
/// # Safety
///
/// `ev` must be a live counter registered by [`evc_init()`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_destroy(ev: *mut EventCounter) {
    // SAFETY: the caller's contract.
    unsafe { eventcount::destroy(ev) };
}

/// `evc_notify_abort()` of kern/eventcount.c.
///
/// # Safety
///
/// `thread` must be the live thread the thread-termination path is about to
/// take off the wait queues.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_notify_abort(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { eventcount::notify_abort(thread) };
}

/// `evc_wait()` of kern/eventcount.c, the trap <mach/syscall_sw.h> declares.
///
/// # Safety
///
/// None: the entry point only reads the eventcounter table, which is kernel
/// state the caller cannot alias.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_wait(ev_id: c_uint) -> c_int {
    match eventcount::wait(ev_id) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `evc_wait_clear()` of kern/eventcount.c, the trap <mach/syscall_sw.h>
/// declares.
///
/// # Safety
///
/// None: the entry point only reads the eventcounter table, which is kernel
/// state the caller cannot alias.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_wait_clear(ev_id: c_uint) -> c_int {
    match eventcount::wait_clear(ev_id) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `evc_signal()` of kern/eventcount.c.
///
/// # Safety
///
/// `ev` must be null or point at a live counter registered by [`evc_init()`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn evc_signal(ev: *mut EventCounter) {
    // SAFETY: the caller's contract.
    unsafe { eventcount::signal(ev) };
}
