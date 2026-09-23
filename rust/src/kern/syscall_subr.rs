// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_subr.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The console-print trap and the priority-depression machinery of
//! `kern/syscall_subr.c`, declared in <kern/syscall_subr.h>.
//!
//! [`mach_print`] is a debugging tool that bypasses messaging
//! altogether; it is trap -30 in <mach/syscall_sw.h>.
//!
//! [`thread_depress_priority()`] lowers a thread to the worst
//! priority for a while, [`thread_depress_timeout()`] restores it when
//! the depression expires, and [`thread_depress_abort()`] restores it
//! early.  `thread_switch()` and the `swtch*` routines around them
//! stay C: they switch stacks through `thread_block()` and
//! `thread_run()`.

use crate::glue;
use crate::kern::ipc_sched::ipc_timeout_to_ticks;
use crate::kern::mach_clock::reset_timeout_check;
use crate::kern::sched::NRQS;
use crate::kern::sched_prim::compute_priority;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// Displays the NUL-terminated `s` on the Mach console.
/// `mach_print()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `s` must point at a NUL-terminated string that stays readable for
/// the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_print(s: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `s`, and
    // the format string is the C call's literal, which matches it.
    unsafe { glue::printf(c"%s".as_ptr(), s) };
}

/// Depress `thread`'s priority to the lowest possible for `depress_time`
/// milliseconds.  `thread_depress_priority()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the
/// thread lock itself.
unsafe fn depress_priority(thread: *mut Thread, depress_time: c_uint) {
    let ticks = ipc_timeout_to_ticks(depress_time);
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the
    // priority fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        // The C cancels a previous depression before replacing it.
        reset_timeout_check(&raw mut (*thread).depress_timer);

        (*thread).depress_priority = (*thread).priority;
        (*thread).priority = NRQS as c_int - 1;
        (*thread).sched_pri = NRQS as c_int - 1;
        if ticks != 0 {
            glue::set_timeout(&raw mut (*thread).depress_timer, ticks);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// Depress a thread's priority for a while.
/// `thread_depress_priority()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the
/// thread lock itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_priority(
    thread: *mut Thread,
    depress_time: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { depress_priority(thread, depress_time) };
}

/// Restore `thread`'s priority when its depression expires.
/// `thread_depress_timeout()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be the live thread the timer was armed for.
unsafe fn depress_timeout(thread: *mut Thread) {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the
    // priority fields.
    unsafe {
        (*thread).lock.lock();

        // A race with `thread_depress_abort()` leaves the thread
        // undepressed, and then there is nothing to restore.
        if (*thread).depress_priority >= 0 {
            (*thread).priority = (*thread).depress_priority;
            (*thread).depress_priority = -1;
            compute_priority(thread, 0);
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// The priority-depression timeout routine, stored in
/// `thread.depress_timer.fcn`.  `thread_depress_timeout()` of
/// kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be a live `thread_t` the timer subsystem owns until
/// the timeout fires; C passes the value stored in
/// `depress_timer.param`.  The C header spells the parameter
/// `thread_t` and the callback's `void *` is the same pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_timeout(thread: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { depress_timeout(thread.cast::<Thread>()) };
}

/// Abort a priority depression early.  `thread_depress_abort()` of
/// kern/syscall_subr.c.
///
/// # Safety
///
/// `thread` must be null or a live thread; the routine takes splsched
/// and the thread lock itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_depress_abort(thread: *mut Thread) -> c_int {
    if thread.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    let s = unsafe { glue::splsched() };
    // SAFETY: the null check above and the caller's contract; the
    // thread lock protects the priority fields and the timer element.
    unsafe {
        (*thread).lock.lock();

        // Only a depressed thread has anything to restore.
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
