// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_sched.c:
//   Copyright (c) 1993, 1992,1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread scheduling entries of `kern/ipc_sched.c`, declared in
//! <kern/sched_prim.h> and <kern/ipc_sched.h>.

use crate::arch::i386::pcb::stack_handoff;
use crate::arch::i386::percpu::{
    cpu_number, current_processor, current_stack,
};
use crate::glue;
use crate::kern::ast::ast_context;
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::sched_prim::{
    TH_RUN_WAIT, TH_RUN_WAIT_SUSP, TH_RUN_WAIT_SUSP_UNINT, TH_RUN_WAIT_UNINT,
    TH_WAIT_SUSP, TH_WAIT_SUSP_UNINT, TH_WAIT_UNINT, THREAD_AWAKENED,
    thread_setrun, thread_wakeup_prim,
};
use crate::kern::thread::{
    Continuation, TH_RUN, TH_SCHED_STATE, TH_SUSP, TH_SWAPPED, TH_WAIT, Thread,
};
use core::ffi::{c_int, c_uint};

/// `convert_ipc_timeout_to_ticks()` of <kern/sched_prim.h>: round a
/// millisecond timeout up to whole ticks.
pub(crate) fn ipc_timeout_to_ticks(msecs: c_uint) -> c_uint {
    let hz = mach_clock::hz;
    // The C expression is unsigned arithmetic over the `int` rate converted to
    // unsigned.
    msecs.wrapping_mul(hz as c_uint).wrapping_add(999) / 1000
}

/// `thread_go()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread; IPC locks may be held, as the C
/// documented.
pub(crate) unsafe fn thread_go(thread: *mut Thread) {
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

/// `thread_will_wait()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread; the routine takes the thread lock
/// itself.
pub(crate) unsafe fn thread_will_wait(thread: *mut Thread) {
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

/// `thread_will_wait_with_timeout()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must point at a live thread; the routine takes the thread lock
/// itself.
pub(crate) unsafe fn thread_will_wait_with_timeout(
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

/// `check_processor_set()` of kern/ipc_sched.c, the `MACH_HOST` arm both
/// configured builds take.
///
/// # Safety
///
/// `thread` must be a live thread and this must run on a live processor.
unsafe fn check_processor_set(thread: *mut Thread) -> bool {
    // SAFETY: the caller's contract; the running processor is initialized
    // before any thread runs.
    unsafe { (*current_processor()).processor_set == (*thread).processor_set }
}

/// `check_bound_processor()` of kern/ipc_sched.c.
///
/// # Safety
///
/// `thread` must be a live thread and this must run on a live processor.
unsafe fn check_bound_processor(thread: *mut Thread) -> bool {
    // SAFETY: the caller's contract.
    let bound = unsafe { (*thread).bound_processor };
    bound.is_null() || bound == current_processor()
}

/// `thread_handoff()` of kern/ipc_sched.c: switch to `new`, leaving `old`
/// blocked with `continuation` as its resume point.
///
/// # Safety
///
/// `old` must be the running thread, `new` a live thread the caller has
/// validated to be wait-and-swapped with the continuation its queue expects,
/// and the caller must hold no thread lock.
pub(crate) unsafe fn thread_handoff(
    old: *mut Thread,
    continuation: Continuation,
    new: *mut Thread,
) -> bool {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the new thread's lock is taken at
    // splsched exactly as the C did, and it protects the fields read below.
    unsafe {
        (*new).lock.lock();

        let can_handoff = (*old).stack_privilege != current_stack()
            && (*new).state() == (TH_WAIT | TH_SWAPPED)
            && check_processor_set(new)
            && check_bound_processor(new);
        if !can_handoff {
            (*new).lock.unlock();
            glue::splx(s);
            return false;
        }

        reset_timeout_check(&raw mut (*new).timer);
        (*new).set_state(TH_RUN);
        (*new).lock.unlock();

        (*new).last_processor = current_processor();
        ast_context(new, cpu_number());

        stack_handoff(old, new);

        (*old).lock.lock();
        (*old).swap_func = continuation;
        (*old).wait_result = -1;
        match (*old).state() {
            TH_RUN => (*old).set_state(TH_WAIT | TH_SWAPPED),
            state if state == TH_RUN | TH_SUSP => {
                (*old).set_state(TH_WAIT | TH_SUSP | TH_SWAPPED);
                if (*old).wake_active() {
                    (*old).set_wake_active(false);
                    (*old).lock.unlock();
                    thread_wakeup_prim(
                        (*old).wake_active_event(),
                        0,
                        THREAD_AWAKENED,
                    );
                    glue::splx(s);
                    return true;
                }
            }
            _ => glue::Panic(
                c"kern/ipc_sched.c".as_ptr(),
                line!() as c_int,
                c"thread_handoff".as_ptr(),
                c"thread_handoff".as_ptr(),
            ),
        }
        (*old).lock.unlock();
        glue::splx(s);
        true
    }
}
