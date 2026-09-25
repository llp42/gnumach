// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_subr.c and kern/syscall_subr.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/syscall_subr.c`, which the file used to
//! define for <kern/syscall_subr.h>.

use crate::kern::syscall_subr;
use crate::kern::thread::Thread;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// `mach_print()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `s` must point at a NUL-terminated string that stays readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_print(s: *const c_char) {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::print(s) };
}

/// `thread_depress_priority()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the thread
/// lock itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_priority(
    thread: *mut Thread,
    depress_time: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::depress_priority(thread, depress_time) };
}

/// `thread_depress_timeout()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live `thread_t` the timer subsystem owns until the
/// timeout fires; C passes the value stored in `depress_timer.param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_timeout(thread: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::depress_timeout(thread) };
}

/// `thread_depress_abort()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be null or a live thread; the routine takes splsched and the
/// thread lock itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_abort(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::depress_abort(thread) }
}

/// `swtch()` of kern/syscall_subr.c.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swtch() -> c_int {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::swtch() }
}

/// `swtch_pri()` of kern/syscall_subr.c.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swtch_pri(_pri: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::swtch_pri() }
}

/// `thread_switch()` of kern/syscall_subr.c.
///
/// # Safety
///
/// Must run on the current thread with no lock held and no wait state set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_switch(
    thread_name: c_uint,
    option: c_int,
    option_time: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { syscall_subr::thread_switch(thread_name, option, option_time) }
}
