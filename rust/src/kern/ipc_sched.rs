// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_sched.c:
//   Copyright (c) 1993, 1992,1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread scheduling entries of `kern/ipc_sched.c`, declared in
//! <kern/sched_prim.h> and <kern/ipc_sched.h>.

use crate::glue;
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::sched_prim::{
    TH_RUN_WAIT, TH_RUN_WAIT_SUSP, TH_RUN_WAIT_SUSP_UNINT, TH_RUN_WAIT_UNINT,
    TH_WAIT_SUSP, TH_WAIT_SUSP_UNINT, TH_WAIT_UNINT, THREAD_AWAKENED,
    thread_setrun,
};
use crate::kern::thread::{TH_RUN, TH_SCHED_STATE, TH_WAIT, Thread};
use core::ffi::c_uint;

/// `convert_ipc_timeout_to_ticks()` of <kern/sched_prim.h>: round a
/// millisecond timeout up to whole ticks.
pub(crate) fn ipc_timeout_to_ticks(msecs: c_uint) -> c_uint {
    let hz = mach_clock::hz;
    // The C expression is unsigned arithmetic over the `int` rate converted to
    // unsigned.
    msecs.wrapping_mul(hz as c_uint).wrapping_add(999) / 1000
}

/// `thread_go()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread; IPC locks may be held, as the C
/// documented.
unsafe fn go(thread: *mut Thread) {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock is taken at splsched
    // exactly as the C did, and it protects every field and the timer element
    // below.
    unsafe {
        (*thread).lock.lock();
        reset_timeout_check(&raw mut (*thread).timer);

        let state = (*thread).state();
        match state & TH_SCHED_STATE {
            TH_WAIT | TH_WAIT_UNINT | TH_WAIT_SUSP_UNINT => {
                (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                (*thread).wait_result = THREAD_AWAKENED;
                thread_setrun(thread, 1);
            }
            TH_WAIT_SUSP
            | TH_RUN_WAIT
            | TH_RUN_WAIT_SUSP
            | TH_RUN_WAIT_UNINT
            | TH_RUN_WAIT_SUSP_UNINT => {
                (*thread).set_state(state & !TH_WAIT);
                (*thread).wait_result = THREAD_AWAKENED;
            }
            _ => (),
        }

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_will_wait()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread; the routine takes the thread lock
/// itself.
unsafe fn will_wait(thread: *mut Thread) {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the two fields,
    // and `-1` is the C's "checkable by later assertions" marker.
    unsafe {
        (*thread).lock.lock();
        (*thread).wait_result = -1;
        (*thread).set_state((*thread).state() | TH_WAIT);
        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_will_wait_with_timeout()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread; the routine takes the thread lock
/// itself.
pub(crate) unsafe fn will_wait_with_timeout(
    thread: *mut Thread,
    msecs: c_uint,
) {
    let ticks = ipc_timeout_to_ticks(msecs);
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the fields and
    // the timer element that `set_timeout()` queues.
    unsafe {
        (*thread).lock.lock();
        (*thread).wait_result = -1;
        (*thread).set_state((*thread).state() | TH_WAIT);
        mach_clock::set_timeout(&raw mut (*thread).timer, ticks);
        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_go()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread, the caller must not hold its lock,
/// and the caller may hold IPC locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_go(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { go(thread) };
}

/// `thread_will_wait()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread and the caller must not hold its lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_will_wait(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { will_wait(thread) };
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
    unsafe { will_wait_with_timeout(thread, msecs) };
}
