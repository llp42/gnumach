// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_sched.c:
//   Copyright (c) 1993, 1992,1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries `kern/ipc_sched.c` used to define.

use crate::kern::ipc_sched;
use crate::kern::thread::{Continuation, Thread};
use core::ffi::{c_int, c_uint};

/// `thread_go()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread, the caller must not hold its lock,
/// and the caller may hold IPC locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_go(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_sched::thread_go(thread) };
}

/// `thread_will_wait()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread and the caller must not hold its lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_will_wait(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_sched::thread_will_wait(thread) };
}

/// `thread_will_wait_with_timeout()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread and the caller must not hold its lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_will_wait_with_timeout(
    thread: *mut Thread,
    msecs: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_sched::thread_will_wait_with_timeout(thread, msecs) };
}

/// `thread_handoff()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `old` must be the running thread and `new` a live wait-and-swapped thread
/// whose queued continuation matches `continuation`; the caller must hold no
/// thread lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_handoff(
    old: *mut Thread,
    continuation: Continuation,
    new: *mut Thread,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { ipc_sched::thread_handoff(old, continuation, new) })
}
