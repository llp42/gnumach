// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_subr.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The console-print trap and the priority-depression machinery of
//! `kern/syscall_subr.c`, declared in <kern/syscall_subr.h>.

use crate::glue;
use crate::kern::ipc_sched::ipc_timeout_to_ticks;
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::sched::NRQS;
use crate::kern::sched_prim::compute_priority;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// `mach_print()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `s` must point at a NUL-terminated string that stays readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_print(s: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `s`, and the
    // format string is the C call's literal, which matches it.
    unsafe { glue::printf(c"%s".as_ptr(), s) };
}

/// `thread_depress_priority()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the thread
/// lock itself.
unsafe fn depress_priority(thread: *mut Thread, depress_time: c_uint) {
    let ticks = ipc_timeout_to_ticks(depress_time);
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the priority
    // fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        reset_timeout_check(&raw mut (*thread).depress_timer);

        (*thread).depress_priority = (*thread).priority;
        (*thread).priority = NRQS as c_int - 1;
        (*thread).sched_pri = NRQS as c_int - 1;
        if ticks != 0 {
            mach_clock::set_timeout(&raw mut (*thread).depress_timer, ticks);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
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
    unsafe { depress_priority(thread, depress_time) };
}

/// `thread_depress_timeout()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be the live thread the timer was armed for.
unsafe fn depress_timeout(thread: *mut Thread) {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the priority
    // fields.
    unsafe {
        (*thread).lock.lock();

        if (*thread).depress_priority >= 0 {
            (*thread).priority = (*thread).depress_priority;
            (*thread).depress_priority = -1;
            compute_priority(thread, 0);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
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
    unsafe { depress_timeout(thread.cast::<Thread>()) };
}

/// `thread_depress_abort()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be null or a live thread; the routine takes splsched and the
/// thread lock itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_abort(thread: *mut Thread) -> c_int {
    if thread.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    let s = unsafe { glue::splsched() };
    // SAFETY: the null check above and the caller's contract; the thread lock
    // protects the priority fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        if (*thread).depress_priority >= 0 {
            reset_timeout_check(&raw mut (*thread).depress_timer);
            (*thread).priority = (*thread).depress_priority;
            (*thread).depress_priority = -1;
            compute_priority(thread, 0);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }

    0
}
