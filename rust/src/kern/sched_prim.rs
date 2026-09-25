// SPDX-License-Identifier: CMU-Mach
// Derived from kern/sched_prim.c:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The wait/wake scheduler primitives, which `kern/sched_prim.c` used to
//! define.

use crate::arch::i386::ast_check::cause_ast_check;
use crate::arch::i386::percpu::{
    cpu_number, current_processor, current_thread, percpu_at,
};
use crate::glue;
use crate::kern::ast::{AST_BLOCK, ast_on};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::policy::{POLICY_FIXEDPRI, POLICY_TIMESHARE};
use crate::kern::processor::{
    PROCESSOR_DISPATCHING, PROCESSOR_IDLE, PROCESSOR_OFF_LINE,
    PROCESSOR_RUNNING, Processor, ProcessorSet,
};
use crate::kern::queue::{
    QueueEntry, dequeue_head, enqueue_tail, queue_empty, queue_end,
    queue_enter_head, queue_enter_tail, queue_first, queue_next,
    queue_remove_generic, remqueue,
};
use crate::kern::sched::{
    NRQS, PRI_SHIFT, RUN_QUEUE_NULL, RunQueue, SCHED_SHIFT,
};
use crate::kern::smp::smp_get_numcpus;
use crate::kern::thread::{
    TH_HALTED, TH_IDLE, TH_RUN, TH_SCHED_STATE, TH_SUSP, TH_SW_COMING_IN,
    TH_SWAP_STATE, TH_SWAPPED, TH_UNINT, TH_WAIT, Thread,
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

pub const TH_WAIT_UNINT: u32 = TH_WAIT | TH_UNINT;
pub const TH_WAIT_SUSP: u32 = TH_WAIT | TH_SUSP;
pub const TH_WAIT_SUSP_UNINT: u32 = TH_WAIT | TH_SUSP | TH_UNINT;
pub const TH_RUN_WAIT: u32 = TH_RUN | TH_WAIT;
pub const TH_RUN_WAIT_SUSP: u32 = TH_RUN | TH_WAIT | TH_SUSP;
pub const TH_RUN_WAIT_UNINT: u32 = TH_RUN | TH_WAIT | TH_UNINT;
pub const TH_RUN_WAIT_SUSP_UNINT: u32 = TH_RUN | TH_WAIT | TH_SUSP | TH_UNINT;
pub const TH_RUN_UNINT: u32 = TH_RUN | TH_UNINT;
pub const TH_RUN_SUSP: u32 = TH_RUN | TH_SUSP;
pub const TH_RUN_SUSP_HALTED: u32 = TH_RUN | TH_SUSP | TH_HALTED;
pub const TH_RUN_SUSP_UNINT: u32 = TH_RUN | TH_SUSP | TH_UNINT;
pub const TH_RUN_IDLE: u32 = TH_RUN | TH_IDLE;

/// The `wait_hash()` macro of kern/sched_prim.c.
fn wait_hash(event: *mut c_void) -> usize {
    let bits = event as isize;
    let folded = if bits < 0 { !bits } else { bits };
    // The folded value is non-negative and the modulo is below `NUMQUEUES`, so
    // the cast cannot lose anything.
    (folded % NUMQUEUES as isize) as usize
}

/// The `state_panic()` macro of kern/sched_prim.c: a thread state the
/// scheduler cannot classify is fatal, with the C message.
fn state_panic(thread: *mut Thread) -> ! {
    // SAFETY: the caller holds the thread lock and `thread` is live.
    let state = unsafe { (*thread).state() };
    let tag = |bit: u32, on: &'static CStr| -> *const c_char {
        if state & bit != 0 {
            on.as_ptr()
        } else {
            c"".as_ptr()
        }
    };
    // SAFETY: the format is the C one: a thread pointer, the state, and the
    // eight tag strings.
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

/// The `run_queue_enqueue()` macro of kern/sched_prim.c, non-DEBUG branch.
///
/// # Safety
///
/// `rq` must be a live run queue and `th` a locked thread, at splsched; the
/// caller may hold the thread lock.
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
        enqueue_tail(
            &raw mut (*rq).runq[whichq as usize],
            &raw mut (*th).links,
        );
        // The C compares the unsigned index against the signed `low`; `low` is
        // a queue index in `0..NRQS`.
        if whichq < (*rq).low as c_uint || (*rq).count == 0 {
            (*rq).low = whichq as c_int;
        }
        (*rq).count += 1;
        (*th).runq = rq;
        (*rq).lock.unlock();
    }
}

/// Whether `th` is already scheduled: on a run queue, chosen as some
/// processor's `next_thread`, or running on a CPU.
///
/// # Safety
///
/// `th` must be a live thread locked by the caller, and the caller must be at
/// splsched.
unsafe fn already_scheduled(th: *mut Thread) -> bool {
    // SAFETY: the caller holds the thread lock, which protects `runq`.
    if unsafe { (*th).runq } != RUN_QUEUE_NULL {
        return true;
    }
    let ncpu = c_int::from(smp_get_numcpus());
    for cpu in 0..ncpu {
        // SAFETY: `cpu` is below the probe's count, so the block is in the C
        // array.
        unsafe {
            let block = percpu_at(cpu);
            if (*block).processor.next_thread == th {
                return true;
            }
            if (*block).active_thread == th {
                return true;
            }
        }
    }
    false
}

/// `thread_setrun()` of kern/sched_prim.c, the core: make `th` runnable,
/// dispatching it straight to an idle processor when one waits, else enqueuing
/// it.
///
/// # Safety
///
/// `th` must be a live thread locked by the caller, and the caller must be at
/// splsched.
fn setrun(th: *mut Thread, may_preempt: bool) {
    // SAFETY: the caller's contract; every field read is protected by the
    // thread lock, and the run queues by their own locks.
    unsafe {
        if already_scheduled(th) {
            return;
        }

        if (*th).sched_stamp != glue::sched_tick {
            glue::update_priority(th);
        }

        let mut processor = (*th).bound_processor;
        if processor.is_null() {
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
                        cause_ast_check(processor);
                    }
                    return;
                }
                (*pset).idle_lock.unlock();
            }
            let rq = &raw mut (*pset).runq;
            enqueue_run_queue(rq, th);
            if may_preempt
                && pset == (*current_processor()).processor_set
                && (*current_thread()).sched_pri > (*th).sched_pri
            {
                (*current_processor()).first_quantum = 0;
                ast_on(cpu_number(), AST_BLOCK);
            }
        } else {
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
                        cause_ast_check(processor);
                    }
                    return;
                }
                (*pset).idle_lock.unlock();
                (*processor).lock.unlock();
            }
            let rq = &raw mut (*processor).runq;
            enqueue_run_queue(rq, th);

            if processor == current_processor() {
                ast_on(cpu_number(), AST_BLOCK);
            } else if (*processor).state != PROCESSOR_OFF_LINE {
                cause_ast_check(processor);
            }
        }
    }
}

/// `thread_timeout_setup()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live, freshly created thread that no other CPU can see
/// yet, as in C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout_setup(thread: *mut Thread) {
    // SAFETY: the caller's contract; the C assigns the same six fields in the
    // same order.
    unsafe {
        (*thread).timer.fcn = Some(thread_timeout);
        (*thread).timer.param = thread.cast::<c_void>();
        (*thread).timer.set = 0;
        (*thread).depress_timer.fcn =
            Some(crate::kern::syscall_subr::thread_depress_timeout);
        (*thread).depress_timer.param = thread.cast::<c_void>();
        (*thread).depress_timer.set = 0;
    }
}

/// `thread_timeout()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live `thread_t` the timer subsystem owns until the
/// timeout fires; C passes the value stored in `timer.param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timeout(thread: *mut c_void) {
    // SAFETY: the caller's contract; `clear_wait()` locks the thread.
    unsafe {
        clear_wait(thread.cast::<Thread>(), THREAD_TIMED_OUT, 0);
    }
}

/// `assert_wait()` of kern/sched_prim.c.
///
/// # Safety
///
/// Called from a thread context with interrupts at a level that prevents the
/// wakeup from being lost, and with the current thread not already waiting on
/// an event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn assert_wait(
    event: *mut c_void,
    interruptible: c_int,
) {
    let thread = current_thread();
    // SAFETY: `thread` is the current thread; the C tests the field before
    // raising splsched, so the order stays.
    let wait_event = unsafe { (*thread).wait_event };
    if !wait_event.is_null() {
        // SAFETY: the C halts here; the format has one pointer argument as the
        // C does.
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
        // SAFETY: `index` is below `NUMQUEUES`; the buckets are the C globals
        // and the hash and thread locks are the C order: the bucket first.
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

/// `clear_wait()` of kern/sched_prim.c, one deliberate change: a thread woken
/// out of `TH_RUN | TH_WAIT` is put on a run queue here, because the dispatch
/// that would have cleared `TH_RUN` may have been bypassed, and a wakeup that
/// only cleared `TH_WAIT` would strand the thread off every run queue.
///
/// # Safety
///
/// `thread` must be a live thread; the routine takes splsched and the thread
/// and hash locks itself, as the C does.
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
            (*thread).lock.unlock();
            glue::splx(s);
            return;
        }

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
                TH_WAIT | TH_WAIT_UNINT | TH_WAIT_SUSP_UNINT => {
                    (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                    (*thread).wait_result = result;
                    setrun(thread, true);
                }
                TH_RUN_WAIT | TH_RUN_WAIT_UNINT => {
                    (*thread).set_state(state & !(TH_RUN | TH_WAIT));
                    (*thread).wait_result = result;
                    setrun(thread, true);
                }
                TH_RUN_WAIT_SUSP | TH_RUN_WAIT_SUSP_UNINT => {
                    (*thread).set_state(state & !(TH_RUN | TH_WAIT));
                    (*thread).wait_result = result;
                    if (*thread).wake_active() {
                        (*thread).set_wake_active(false);
                        (*thread).lock.unlock();
                        thread_wakeup_prim(
                            (*thread).wake_active_event(),
                            0,
                            THREAD_AWAKENED,
                        );
                        glue::splx(s);
                        return;
                    }
                }
                TH_WAIT_SUSP => {
                    (*thread).set_state(state & !TH_WAIT);
                    (*thread).wait_result = result;
                }
                _ => (),
            }
        }
        (*thread).lock.unlock();
    }
    unsafe { glue::splx(s) };
}

/// `thread_wakeup_prim()` of kern/sched_prim.c.
///
/// # Safety
///
/// `event` is an opaque key; the C signature passes a `boolean_t` for
/// `one_thread` and a wait result, and the routine takes the locks it needs
/// itself.
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
    // SAFETY: the bucket lock is held for the whole walk; the thread lock is
    // taken around each thread's fields.
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
                    TH_WAIT | TH_WAIT_UNINT | TH_WAIT_SUSP_UNINT => {
                        (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                        (*thread).wait_result = result;
                        setrun(thread, true);
                    }
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

/// `thread_sleep()` of kern/sched_prim.c.
///
/// # Safety
///
/// Same contract as `assert_wait()`, plus `lock` must be a live simple lock
/// held by the current thread, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_sleep(
    event: *mut c_void,
    lock: *mut SimpleLock,
    interruptible: c_int,
) {
    // SAFETY: the caller's contract; the C asserts the event, unlocks and
    // blocks with `thread_no_continuation`.
    unsafe {
        assert_wait(event, interruptible);
        (*lock).unlock();
        glue::thread_block(None);
    }
}

/// `thread_dispatch()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread that is not on a run queue, and the caller
/// must be at splsched; the i386 context switch calls this symbol directly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_dispatch(thread: *mut Thread) {
    // SAFETY: the caller's contract; the thread lock protects the state below.
    unsafe {
        (*thread).lock.lock();

        if (*thread).state() & TH_RUN == 0 {
            (*thread).lock.unlock();
            return;
        }

        if (*thread).swap_func.is_some() {
            (*thread).set_state((*thread).state() | TH_SWAPPED);
            (*thread).stack_free();
        }

        match (*thread).state() & !TH_SWAP_STATE {
            TH_RUN_SUSP | TH_RUN_SUSP_HALTED | TH_RUN_WAIT_SUSP => {
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
                setrun(thread, false);
            }
            TH_RUN_WAIT_SUSP_UNINT | TH_RUN_WAIT_UNINT | TH_RUN_WAIT => {
                (*thread).set_state((*thread).state() & !TH_RUN);
            }
            // The idle thread is already in `idle_thread_array`.
            TH_RUN_IDLE => (),
            _ => state_panic(thread),
        }
        (*thread).lock.unlock();
    }
}

/// `thread_setrun()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread locked by the caller, and the caller must be
/// at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_setrun(
    thread: *mut Thread,
    may_preempt: c_int,
) {
    setrun(thread, may_preempt != 0);
}

/// The effective priority of `thread`, from its base priority plus a shift of
/// its accumulated usage.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds.
unsafe fn priority_computation(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; the lock protects the two fields.
    let pri = unsafe {
        let usage = (*thread).sched_usage >> (PRI_SHIFT + SCHED_SHIFT);
        // The C adds an `unsigned` to the `int` priority and stores the sum
        // back into an `int`, so the result wraps.
        (*thread).priority.wrapping_add(usage as c_int)
    };
    // The C clamps with `if (pri > NRQS - 1)`; a negative sum is left alone,
    // exactly as the C leaves it.
    if pri > NRQS as c_int - 1 {
        NRQS as c_int - 1
    } else {
        pri
    }
}

/// `compute_priority()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds.
unsafe fn recompute_priority(thread: *mut Thread, resched: bool) {
    // SAFETY: the caller's contract; the lock protects the policy, the
    // priority fields and the run queue link.
    unsafe {
        if (*thread).policy == POLICY_TIMESHARE {
            let pri = priority_computation(thread);
            if (*thread).depress_priority < 0 {
                set_priority(thread, pri, resched);
            } else {
                (*thread).depress_priority = pri;
            }
        } else {
            set_priority(thread, (*thread).priority, resched);
        }
    }
}

/// `set_pri()` of kern/sched_prim.c.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds, and the caller must
/// be at splsched.
unsafe fn set_priority(th: *mut Thread, pri: c_int, resched: bool) {
    // SAFETY: the caller's contract; `rem_runq()` takes the run-queue lock
    // itself, and the enqueue path takes it again.
    unsafe {
        let rq = glue::rem_runq(th);
        (*th).sched_pri = pri;
        if rq != RUN_QUEUE_NULL {
            if resched {
                setrun(th, true);
            } else {
                enqueue_run_queue(rq, th);
            }
        }
    }
}

/// `choose_pset_thread()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must be at splsched, must hold `pset`'s run-queue lock, and
/// `myprocessor` must be the current processor with `pset` its processor set.
unsafe fn pset_thread(
    myprocessor: *mut Processor,
    pset: *mut ProcessorSet,
) -> *mut Thread {
    // SAFETY: the caller's contract; the run-queue lock serializes every queue
    // operation below, and the thread lock is not needed because the run-queue
    // lock protects the `runq` link.
    unsafe {
        let runq = &raw mut (*pset).runq;
        let mut i = (*runq).low;
        if (*runq).count > 0 {
            while i < NRQS as c_int {
                let mut q = &raw mut (*runq).runq[i as usize];
                if queue_empty(q) == 0 {
                    let th = dequeue_head(q).cast::<Thread>();
                    (*th).runq = RUN_QUEUE_NULL;
                    (*runq).count = (*runq).count.wrapping_sub(1);
                    if (*runq).count > 0
                        && (*pset).policies & POLICY_FIXEDPRI != 0
                    {
                        while queue_empty(q) != 0 {
                            i += 1;
                            if i >= NRQS as c_int {
                                glue::Panic(
                                    c"kern/sched_prim.c".as_ptr(),
                                    line!() as c_int,
                                    c"choose_pset_thread".as_ptr(),
                                    c"choose_pset_thread".as_ptr(),
                                )
                            }
                            q = &raw mut (*runq).runq[i as usize];
                        }
                    }
                    (*runq).low = i;
                    (*runq).lock.unlock();
                    return th;
                }
                i += 1;
            }
            glue::Panic(
                c"kern/sched_prim.c".as_ptr(),
                line!() as c_int,
                c"choose_pset_thread".as_ptr(),
                c"choose_pset_thread".as_ptr(),
            )
        }
        (*runq).lock.unlock();
    }

    // SAFETY: the caller's contract; `idle_lock` protects the idle queue and
    // count, and the run-queue lock is already released.
    unsafe {
        (*pset).idle_lock.lock();
        if (*myprocessor).state == PROCESSOR_RUNNING {
            (*myprocessor).state = PROCESSOR_IDLE;
            if myprocessor == glue::master_processor {
                queue_enter_tail(
                    &raw mut (*pset).idle_queue,
                    myprocessor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
            } else {
                queue_enter_head(
                    &raw mut (*pset).idle_queue,
                    myprocessor.cast::<c_void>(),
                    offset_of!(Processor, processor_queue),
                );
            }
            (*pset).idle_count = (*pset).idle_count.wrapping_add(1);
        }
        (*pset).idle_lock.unlock();

        (*myprocessor).idle_thread
    }
}

/// `thread_set_timeout()` of kern/sched_prim.c.
///
/// # Safety
///
/// Must be called between `assert_wait()` and `thread_block()` for the current
/// thread, as the C documents.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_timeout(t: c_int) {
    let thread = current_thread();
    let s = unsafe { glue::splsched() };
    // SAFETY: `thread` is the current thread; its lock protects the state and
    // the timer element, as in C.
    unsafe {
        (*thread).lock.lock();
        if (*thread).state() & TH_WAIT != 0 {
            // The C passes the `int` to an `unsigned` parameter, so a negative
            // interval wraps; the cast is that conversion.
            mach_clock::set_timeout(&raw mut (*thread).timer, t as c_uint);
        }
        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_bind()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread, and `processor` must be a live processor or
/// the C `PROCESSOR_NULL`, which the caller spells as a null pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_bind(
    thread: *mut Thread,
    processor: *mut Processor,
) {
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller's contract; the thread lock protects the binding.
    unsafe {
        (*thread).lock.lock();
        (*thread).bound_processor = processor;
        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `thread_continue()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller runs this on the current thread, at splsched, after a stack
/// swap; `old_thread` must be a live thread the context switch left to
/// dispatch, or null when there is none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_continue(old_thread: *mut Thread) {
    // SAFETY: the caller's contract; `swap_func` is set before the thread is
    // resumed on the new stack.
    let continuation = unsafe { (*current_thread()).swap_func };

    if !old_thread.is_null() {
        // SAFETY: the caller passes the thread the context switch left to
        // dispatch, as `thread_dispatch()` requires.
        unsafe { thread_dispatch(old_thread) };
    }
    // SAFETY: `spl0()` is the real asm routine of <machine/spl.h>.
    unsafe { glue::spl0() };

    // SAFETY: the C calls the continuation unconditionally; a null `swap_func`
    // means the thread was resumed without one, which cannot happen, so the
    // halt spells out what the C null call did.
    unsafe {
        match continuation {
            Some(continuation) => continuation(),
            None => glue::Panic(
                c"kern/sched_prim.c".as_ptr(),
                line!() as c_int,
                c"thread_continue".as_ptr(),
                c"thread_continue: null continuation".as_ptr(),
            ),
        }
    }
}

/// `compute_priority()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compute_priority(
    thread: *mut Thread,
    resched: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { recompute_priority(thread, resched != 0) };
}

/// `compute_my_priority()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must hold the thread lock and know the thread is timesharing and
/// not depressed, as the C documents.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compute_my_priority(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).sched_pri = priority_computation(thread) };
}

/// `recompute_priorities()` of kern/sched_prim.c.
///
/// # Safety
///
/// Called by the timeout machinery at splsoftclock, and once at boot; `param`
/// is unused, as in C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recompute_priorities(_param: *mut c_void) {
    // SAFETY: `sched_tick` and the private timer are the C globals; the clock
    // lock inside `set_timeout()` serializes against the clock interrupt,
    // exactly as the C call's did.
    unsafe {
        glue::sched_tick = glue::sched_tick.wrapping_add(1);
        mach_clock::set_timeout(
            &raw mut glue::recompute_priorities_timer,
            mach_clock::hz as c_uint,
        );
        if !glue::sched_thread_id.is_null() {
            // SAFETY: `clear_wait()` takes the thread and hash locks itself;
            // the scheduler thread may be waiting.
            clear_wait(glue::sched_thread_id, THREAD_AWAKENED, 0);
        }
    }
}

/// `set_pri()` of kern/sched_prim.c.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds, and the caller must
/// be at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_pri(th: *mut Thread, pri: c_int, resched: c_int) {
    // SAFETY: the caller's contract.
    unsafe { set_priority(th, pri, resched != 0) };
}

/// `choose_pset_thread()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must be at splsched and must hold `pset`'s run-queue lock;
/// `myprocessor` must be the current processor and `pset` its processor set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn choose_pset_thread(
    myprocessor: *mut Processor,
    pset: *mut ProcessorSet,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { pset_thread(myprocessor, pset) }
}
