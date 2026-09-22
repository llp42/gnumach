// SPDX-License-Identifier: CMU-Mach
// Derived from kern/sched_prim.c:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The wait/wake scheduler primitives, which `kern/sched_prim.c` used
//! to define.
//!
//! The wait hash and its reason for existing are the C file's; this is
//! a line-for-line translation, including the state transitions of
//! `clear_wait()` and the lost-wakeup protocol they implement, so that
//! the next commit can be a change of protocol in one language.  The
//! waiting protocol itself, the run-queue enqueue and the idle
//! handoff all keep their C order of operations, locks and spl level.
//!
//! The wait buckets stay C (`wait_queue`, `wait_lock`) and are reached
//! with raw pointers; `wait_queue_init()` still builds them.  The
//! functions that stay in C (`thread_block()`, `thread_invoke()`,
//! `thread_select()`, `update_priority()`, `set_pri()`, `rem_runq()`)
//! are called through `glue`.

use crate::arch::i386::percpu::{
    cpu_number, current_processor, current_thread,
};
use crate::glue;
use crate::kern::ast::{AST_BLOCK, ast_on};
use crate::kern::lock::SimpleLock;
use crate::kern::processor::{
    NRQS, PROCESSOR_DISPATCHING, PROCESSOR_IDLE, PROCESSOR_OFF_LINE,
    Processor, RunQueue,
};
use crate::kern::queue::{
    QueueEntry, enqueue_tail, queue_end, queue_first, queue_next,
    queue_remove_generic, remqueue,
};
use crate::kern::thread::{
    TH_HALTED, TH_IDLE, TH_RUN, TH_SCHED_STATE, TH_SUSP, TH_SW_COMING_IN,
    TH_SWAP_STATE, TH_SWAPPED, TH_UNINT, TH_WAIT, TIMEOUT_ACTIVE, Thread,
    Timeout,
};
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::offset_of;
use core::ptr;

/// `NUMQUEUES` in kern/sched_prim.c: the size of the event hash table.
pub const NUMQUEUES: usize = 1031;

/// `THREAD_AWAKENED` in <kern/sched_prim.h>: a normal wakeup.
pub const THREAD_AWAKENED: c_int = 0;
/// `THREAD_TIMED_OUT`: the timeout expired.
pub const THREAD_TIMED_OUT: c_int = 1;
/// `THREAD_INTERRUPTED`: `clear_wait()` interrupted the wait.
pub const THREAD_INTERRUPTED: c_int = 2;
/// `THREAD_RESTART`: restart the operation entirely.
pub const THREAD_RESTART: c_int = 3;

// The switch labels of the wait-state machines, named as the C writes
// them.  They must be values, not or-patterns, so each one is folded
// here; `state` is masked with `TH_SCHED_STATE` before the match.
const TH_WAIT_UNINT: u32 = TH_WAIT | TH_UNINT;
const TH_WAIT_SUSP: u32 = TH_WAIT | TH_SUSP;
const TH_WAIT_SUSP_UNINT: u32 = TH_WAIT | TH_SUSP | TH_UNINT;
const TH_RUN_WAIT: u32 = TH_RUN | TH_WAIT;
const TH_RUN_WAIT_SUSP: u32 = TH_RUN | TH_WAIT | TH_SUSP;
const TH_RUN_WAIT_UNINT: u32 = TH_RUN | TH_WAIT | TH_UNINT;
const TH_RUN_WAIT_SUSP_UNINT: u32 = TH_RUN | TH_WAIT | TH_SUSP | TH_UNINT;
const TH_RUN_UNINT: u32 = TH_RUN | TH_UNINT;
const TH_RUN_SUSP: u32 = TH_RUN | TH_SUSP;
const TH_RUN_SUSP_HALTED: u32 = TH_RUN | TH_SUSP | TH_HALTED;
const TH_RUN_SUSP_UNINT: u32 = TH_RUN | TH_SUSP | TH_UNINT;
const TH_RUN_IDLE: u32 = TH_RUN | TH_IDLE;

/// The `wait_hash()` macro of kern/sched_prim.c.
///
/// The sign fold is the hash: C takes the bitwise complement of an
/// event whose signed value is negative, and a wakeup misses unless the
/// Rust computes the same bucket.
fn wait_hash(event: *mut c_void) -> usize {
    let bits = event as isize;
    let folded = if bits < 0 { !bits } else { bits };
    // The folded value is non-negative and the modulo is below
    // `NUMQUEUES`, so the cast cannot lose anything.
    (folded % NUMQUEUES as isize) as usize
}

/// `reset_timeout_check()` of <kern/mach_clock.h>: cancel the thread's
/// wait timeout if one is active.
///
/// # Safety
///
/// The caller holds the thread lock, so `t` is stable and only this
/// thread's timeout can be set.
unsafe fn reset_timeout_check(t: *mut Timeout) {
    // SAFETY: the caller's contract; `set` is a plain byte field.
    if unsafe { (*t).set } & TIMEOUT_ACTIVE != 0 {
        // SAFETY: the same contract, and `reset_timeout()` takes the
        // element off the timeout queue at splsched.
        unsafe { glue::reset_timeout(t) };
    }
}

/// The `state_panic()` macro of kern/sched_prim.c: a thread state the
/// scheduler cannot classify is fatal, with the C message.
fn state_panic(thread: *mut Thread) -> ! {
    // SAFETY: the caller holds the thread lock and `thread` is live.
    let state = unsafe { (*thread).state() };
    // The C chooses one tag per set bit, the empty string otherwise.
    let tag = |bit: u32, on: &'static CStr| -> *const c_char {
        if state & bit != 0 {
            on.as_ptr()
        } else {
            c"".as_ptr()
        }
    };
    // SAFETY: the format is the C one: a thread pointer, the state,
    // and the eight tag strings.  `Panic` does not return.
    unsafe {
        glue::Panic(
            c"kern/sched_prim.c".as_ptr(),
            line!() as c_int,
            c"state_panic".as_ptr(),
            c"thread %p has unexpected state %x (%s%s%s%s%s%s%s%s)".as_ptr(),
            thread,
            state,
            tag(TH_WAIT, c"TH_WAIT|"),
            tag(TH_SUSP, c"TH_SUSP|"),
            tag(TH_RUN, c"TH_RUN|"),
            tag(TH_UNINT, c"TH_UNINT|"),
            tag(TH_HALTED, c"TH_HALTED|"),
            tag(TH_IDLE, c"TH_IDLE|"),
            tag(TH_SWAPPED, c"TH_SWAPPED|"),
            tag(TH_SW_COMING_IN, c"TH_SW_COMING_IN|"),
        )
    }
}

/// The `run_queue_enqueue()` macro of kern/sched_prim.c, non-DEBUG
/// branch.  `DEBUG` is undefined in the configured kernels, so the
/// `checkrq()`/`thread_check()` calls the macro would make are not
/// compiled in C either.
///
/// # Safety
///
/// `rq` must be a live run queue and `th` a locked thread, at
/// splsched; the caller may hold the thread lock.
unsafe fn enqueue_run_queue(rq: *mut RunQueue, th: *mut Thread) {
    // SAFETY: the caller's contract; `th` is locked.
    unsafe {
        // The C assigns the signed priority to an `unsigned int`, so a
        // negative value fails the bounds check below and is clamped.
        let mut whichq = (*th).sched_pri as c_uint;
        if whichq >= NRQS as c_uint {
            glue::printf(
                c"thread_setrun: pri too high (%d)\n".as_ptr(),
                (*th).sched_pri,
            );
            whichq = NRQS as c_uint - 1;
        }

        (*rq).lock.lock();
        // `whichq` is below `NRQS` on both paths.
        enqueue_tail(
            &raw mut (*rq).runq[whichq as usize],
            &raw mut (*th).links,
        );
        // The C compares the unsigned index against the signed `low`;
        // `low` is a queue index in `0..NRQS`.
        if whichq < (*rq).low as c_uint || (*rq).count == 0 {
            (*rq).low = whichq as c_int;
        }
        (*rq).count += 1;
        (*th).runq = rq;
        (*rq).lock.unlock();
    }
}

/// `thread_setrun()` of kern/sched_prim.c, the core: make `th`
/// runnable, dispatching it straight to an idle processor when one
/// waits, else enqueuing it.
///
/// # Safety
///
/// `th` must be a live thread locked by the caller, and the caller
/// must be at splsched.
fn setrun(th: *mut Thread, may_preempt: bool) {
    // SAFETY: the caller's contract; every field read is protected by
    // the thread lock, and the run queues by their own locks.
    unsafe {
        if (*th).sched_stamp != glue::sched_tick {
            glue::update_priority(th);
        }

        // Try to dispatch the thread directly onto an idle processor.
        let mut processor = (*th).bound_processor;
        if processor.is_null() {
            // Unbound: any processor in the set is acceptable.
            let pset = (*th).processor_set;
            if (*pset).idle_count > 0 {
                (*pset).idle_lock.lock();
                if (*pset).idle_count > 0 {
                    processor = queue_first(&raw mut (*pset).idle_queue)
                        .cast::<Processor>();
                    queue_remove_generic(
                        &raw mut (*pset).idle_queue,
                        processor.cast::<c_void>(),
                        offset_of!(Processor, processor_queue),
                    );
                    (*pset).idle_count -= 1;
                    (*processor).next_thread = th;
                    (*processor).state = PROCESSOR_DISPATCHING;
                    (*pset).idle_lock.unlock();
                    if processor != current_processor() {
                        glue::cause_ast_check(processor);
                    }
                    return;
                }
                (*pset).idle_lock.unlock();
            }
            let rq = &raw mut (*pset).runq;
            enqueue_run_queue(rq, th);
            // MACH_HOST is on in this build, so the set equality test
            // stays in.
            if may_preempt
                && pset == (*current_processor()).processor_set
                && (*current_thread()).sched_pri > (*th).sched_pri
            {
                // Turn off first_quantum to allow the context switch.
                (*current_processor()).first_quantum = 0;
                ast_on(cpu_number(), AST_BLOCK);
            }
        } else {
            // Bound: it can only run on its processor, whose lock must
            // be taken because this may not be the current one.
            if !processor.is_null() && (*processor).state == PROCESSOR_IDLE {
                (*processor).lock.lock();
                let pset = (*processor).processor_set;
                (*pset).idle_lock.lock();
                if (*processor).state == PROCESSOR_IDLE {
                    queue_remove_generic(
                        &raw mut (*pset).idle_queue,
                        processor.cast::<c_void>(),
                        offset_of!(Processor, processor_queue),
                    );
                    (*pset).idle_count -= 1;
                    (*processor).next_thread = th;
                    (*processor).state = PROCESSOR_DISPATCHING;
                    (*pset).idle_lock.unlock();
                    (*processor).lock.unlock();
                    if processor != current_processor() {
                        glue::cause_ast_check(processor);
                    }
                    return;
                }
                (*pset).idle_lock.unlock();
                (*processor).lock.unlock();
            }
            let rq = &raw mut (*processor).runq;
            enqueue_run_queue(rq, th);

            // Cause an AST on the processor if it is on line.
            if processor == current_processor() {
                ast_on(cpu_number(), AST_BLOCK);
            } else if (*processor).state != PROCESSOR_OFF_LINE {
                glue::cause_ast_check(processor);
            }
        }
    }
}

/// Sets up a thread's timeout elements when the thread is created.
/// `thread_timeout_setup()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live, freshly created thread that no other CPU
/// can see yet, as in C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout_setup(thread: *mut Thread) {
    // SAFETY: the caller's contract; the C assigns the same six
    // fields in the same order.
    unsafe {
        (*thread).timer.fcn = Some(thread_timeout);
        (*thread).timer.param = thread.cast::<c_void>();
        (*thread).timer.set = 0;
        (*thread).depress_timer.fcn = Some(glue::thread_depress_timeout);
        (*thread).depress_timer.param = thread.cast::<c_void>();
        (*thread).depress_timer.set = 0;
    }
}

/// The thread timeout routine, called at splsoftclock.
/// `thread_timeout()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live `thread_t` the timer subsystem owns until
/// the timeout fires; C passes the value stored in `timer.param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout(thread: *mut c_void) {
    // SAFETY: the caller's contract; `clear_wait()` locks the thread.
    unsafe {
        clear_wait(thread.cast::<Thread>(), THREAD_TIMED_OUT, 0);
    }
}

/// Assert that the current thread is about to wait on `event`.
/// `assert_wait()` of kern/sched_prim.c.
///
/// # Safety
///
/// Called from a thread context with interrupts at a level that
/// prevents the wakeup from being lost, and with the current thread
/// not already waiting on an event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn assert_wait(
    event: *mut c_void,
    interruptible: c_int,
) {
    let thread = current_thread();
    // SAFETY: `thread` is the current thread; the C tests the field
    // before raising splsched, so the order stays.
    let wait_event = unsafe { (*thread).wait_event };
    if !wait_event.is_null() {
        // SAFETY: the C halts here; the format has one pointer
        // argument as the C does.
        unsafe {
            glue::Panic(
                c"kern/sched_prim.c".as_ptr(),
                line!() as c_int,
                c"assert_wait".as_ptr(),
                c"assert_wait: already asserted event %p\n".as_ptr(),
                wait_event,
            )
        }
    }
    let s = unsafe { glue::splsched() };
    let add = if interruptible != 0 {
        TH_WAIT
    } else {
        TH_WAIT | TH_UNINT
    };
    if !event.is_null() {
        let index = wait_hash(event);
        // SAFETY: `index` is below `NUMQUEUES`; the buckets are the
        // C globals and the hash and thread locks are the C order:
        // the bucket first.
        unsafe {
            let q = &raw mut glue::wait_queue[index];
            let lock = &raw mut glue::wait_lock[index];
            (*lock).lock();
            (*thread).lock.lock();
            enqueue_tail(q, &raw mut (*thread).links);
            (*thread).wait_event = event;
            let state = (*thread).state();
            (*thread).set_state(state | add);
            (*thread).lock.unlock();
            (*lock).unlock();
        }
    } else {
        // SAFETY: as above, without a hash bucket.
        unsafe {
            (*thread).lock.lock();
            let state = (*thread).state();
            (*thread).set_state(state | add);
            (*thread).lock.unlock();
        }
    }
    unsafe { glue::splx(s) };
}

/// Clear the wait condition for `thread` and start it if appropriate.
/// `clear_wait()` of kern/sched_prim.c, reproduced exactly: the
/// `TH_RUN | TH_WAIT` arms keep their current transitions.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the
/// thread and hash locks itself, as the C does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clear_wait(
    thread: *mut Thread,
    result: c_int,
    interrupt_only: c_int,
) {
    let s = unsafe { glue::splsched() };
    // SAFETY: `thread` is live; the lock protects every field below.
    unsafe {
        (*thread).lock.lock();
        if interrupt_only != 0 && (*thread).state() & TH_UNINT != 0 {
            // Cannot interrupt the thread.
            (*thread).lock.unlock();
            glue::splx(s);
            return;
        }

        // If the thread waits on an event, take it off the bucket.
        // The hash lock must be taken before any thread lock, so the
        // thread lock is dropped first.
        let mut event = (*thread).wait_event;
        if !event.is_null() {
            (*thread).lock.unlock();
            let index = wait_hash(event);
            let q = &raw mut glue::wait_queue[index];
            let lock = &raw mut glue::wait_lock[index];
            (*lock).lock();
            (*thread).lock.lock();
            if (*thread).wait_event == event {
                remqueue(q, thread.cast::<QueueEntry>());
                (*thread).wait_event = ptr::null_mut();
                event = ptr::null_mut(); // cause the wakeup below
            }
            (*lock).unlock();
        }

        if event.is_null() {
            let state = (*thread).state();
            reset_timeout_check(&raw mut (*thread).timer);
            match state & TH_SCHED_STATE {
                // Sleeping and not suspendable: put on a run queue.
                TH_WAIT | TH_WAIT_UNINT | TH_WAIT_SUSP_UNINT => {
                    (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                    (*thread).wait_result = result;
                    setrun(thread, true);
                }
                // Either already running, or suspended.
                TH_WAIT_SUSP
                | TH_RUN_WAIT
                | TH_RUN_WAIT_SUSP
                | TH_RUN_WAIT_UNINT
                | TH_RUN_WAIT_SUSP_UNINT => {
                    (*thread).set_state(state & !TH_WAIT);
                    (*thread).wait_result = result;
                }
                // Not waiting.
                _ => (),
            }
        }
        (*thread).lock.unlock();
    }
    unsafe { glue::splx(s) };
}

/// Wake every thread (or the first one) waiting on `event`.
/// `thread_wakeup_prim()` of kern/sched_prim.c.
///
/// # Safety
///
/// `event` is an opaque key; the C signature passes a `boolean_t` for
/// `one_thread` and a wait result, and the routine takes the locks it
/// needs itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_wakeup_prim(
    event: *mut c_void,
    one_thread: c_int,
    result: c_int,
) -> c_int {
    let index = wait_hash(event);
    let q = unsafe { &raw mut glue::wait_queue[index] };
    let s = unsafe { glue::splsched() };
    let lock = unsafe { &raw mut glue::wait_lock[index] };
    // SAFETY: the bucket lock is held for the whole walk; the thread
    // lock is taken around each thread's fields.
    let mut woke = false;
    unsafe {
        (*lock).lock();
        let mut thread = queue_first(q).cast::<Thread>();
        while queue_end(q, thread.cast::<QueueEntry>()) == 0 {
            let next_th =
                queue_next(thread.cast::<QueueEntry>()).cast::<Thread>();

            if (*thread).wait_event == event {
                (*thread).lock.lock();
                remqueue(q, thread.cast::<QueueEntry>());
                (*thread).wait_event = ptr::null_mut();
                reset_timeout_check(&raw mut (*thread).timer);

                let state = (*thread).state();
                match state & TH_SCHED_STATE {
                    // Sleeping and not suspendable: put on a run queue.
                    TH_WAIT | TH_WAIT_UNINT | TH_WAIT_SUSP_UNINT => {
                        (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                        (*thread).wait_result = result;
                        setrun(thread, true);
                    }
                    // Either already running, or suspended.
                    TH_WAIT_SUSP
                    | TH_RUN_WAIT
                    | TH_RUN_WAIT_SUSP
                    | TH_RUN_WAIT_UNINT
                    | TH_RUN_WAIT_SUSP_UNINT => {
                        (*thread).set_state(state & !TH_WAIT);
                        (*thread).wait_result = result;
                    }
                    _ => state_panic(thread),
                }
                (*thread).lock.unlock();
                woke = true;
                if one_thread != 0 {
                    break;
                }
            }
            thread = next_th;
        }
        (*lock).unlock();
    }
    unsafe { glue::splx(s) };
    c_int::from(woke)
}

/// Wait until `event` occurs, releasing `lock` before giving up the
/// CPU.  `thread_sleep()` of kern/sched_prim.c.
///
/// # Safety
///
/// Same contract as `assert_wait()`, plus `lock` must be a live simple
/// lock held by the current thread, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_sleep(
    event: *mut c_void,
    lock: *mut SimpleLock,
    interruptible: c_int,
) {
    // SAFETY: the caller's contract; the C asserts the event, unlocks
    // and blocks with `thread_no_continuation`.
    unsafe {
        assert_wait(event, interruptible);
        (*lock).unlock();
        glue::thread_block(None);
    }
}

/// Dispatch a running thread that is not on a run queue.
/// `thread_dispatch()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread that is not on a run queue, and the
/// caller must be at splsched; the i386 context switch calls this
/// symbol directly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_dispatch(thread: *mut Thread) {
    // SAFETY: the caller's contract; the thread lock protects the
    // state below.
    unsafe {
        (*thread).lock.lock();

        // If the thread's stack is being discarded, free it before the
        // thread has a chance to run.
        if (*thread).swap_func.is_some() {
            (*thread).set_state((*thread).state() | TH_SWAPPED);
            glue::stack_free(thread);
        }

        match (*thread).state() & !TH_SWAP_STATE {
            TH_RUN_SUSP | TH_RUN_SUSP_HALTED | TH_RUN_WAIT_SUSP => {
                // Suspend the thread.
                (*thread).set_state((*thread).state() & !TH_RUN);
                if (*thread).wake_active() {
                    (*thread).set_wake_active(false);
                    (*thread).lock.unlock();
                    let event = (*thread).wake_active_event();
                    thread_wakeup_prim(event, 0, THREAD_AWAKENED);
                    return;
                }
            }
            TH_RUN_SUSP_UNINT | TH_RUN | TH_RUN_UNINT => {
                // No reason to stop: put back on a run queue.
                setrun(thread, false);
            }
            TH_RUN_WAIT_SUSP_UNINT | TH_RUN_WAIT_UNINT | TH_RUN_WAIT => {
                // Waiting, and not suspended.
                (*thread).set_state((*thread).state() & !TH_RUN);
            }
            TH_RUN_IDLE => {
                // Drop the idle thread: it is already in
                // `idle_thread_array`.
            }
            _ => state_panic(thread),
        }
        (*thread).lock.unlock();
    }
}

/// Make `thread` runnable.  `thread_setrun()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread locked by the caller, and the caller
/// must be at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_setrun(
    thread: *mut Thread,
    may_preempt: c_int,
) {
    setrun(thread, may_preempt != 0);
}
