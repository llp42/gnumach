// SPDX-License-Identifier: CMU-Mach
// Derived from kern/sched_prim.c:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The wait/wake and run-queue scheduler primitives, which `kern/sched_prim.c`
//! used to define and `kern/sched_prim.h` declares.

use crate::arch::i386::ast_check::cause_ast_check;
use crate::arch::i386::model_dep::machine_idle;
use crate::arch::i386::pcb::{stack_handoff, switch_context};
use crate::arch::i386::percpu::{
    cpu_number, current_processor, current_stack, current_thread, percpu_at,
};
use crate::glue;
use crate::kern::ast::{
    AST_BLOCK, ast_clear_scheduling, ast_context, ast_off, ast_on,
    ast_scheduling_pending,
};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::{self, reset_timeout_check};
use crate::kern::mach_factor;
use crate::kern::machine;
use crate::kern::policy::{POLICY_FIXEDPRI, POLICY_TIMESHARE};
use crate::kern::processor::{
    self, PROCESSOR_ASSIGN, PROCESSOR_DISPATCHING, PROCESSOR_IDLE,
    PROCESSOR_OFF_LINE, PROCESSOR_RUNNING, PROCESSOR_SHUTDOWN, Processor,
    ProcessorSet,
};
use crate::kern::queue::{
    QueueEntry, dequeue_head, enqueue_tail, queue_empty, queue_end,
    queue_enter_head, queue_enter_tail, queue_first, queue_init, queue_next,
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
use crate::kern::thread_swap::thread_swapin;
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::offset_of;
use core::ptr::{self, addr_of_mut};
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, Ordering};

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

/// `MAX_STUCK_THREADS` in kern/sched_prim.c: the stuck-thread scan's array
/// size.
const MAX_STUCK_THREADS: usize = 16;

/// `sched_tick` of kern/sched_prim.c: the seconds counter that ages
/// priorities.  `kern/priority.c` still reads the symbol, so it keeps the C
/// name and width; the accesses are `Relaxed` because no data rides on the
/// counter, the thread lock serializes the fields it compares.
#[unsafe(export_name = "sched_tick")]
static SCHED_TICK: AtomicU32 = AtomicU32::new(0);

/// `min_quantum` of kern/sched_prim.c: the shortest processor quantum, in
/// ticks.  `kern/priority.c` and `kern/syscall_subr.c` still read the symbol.
#[unsafe(export_name = "min_quantum")]
static MIN_QUANTUM_TICKS: AtomicI32 = AtomicI32::new(0);

/// `sched_thread_id` of kern/sched_prim.c: the scheduler thread
/// `recompute_priorities()` wakes.
static SCHED_THREAD_ID: AtomicPtr<Thread> = AtomicPtr::new(ptr::null_mut());

/// `recompute_priorities_timer` of kern/sched_prim.c.
static RECOMPUTE_PRIORITIES_TIMER: SyncCell<mach_clock::Timeout> =
    SyncCell(UnsafeCell::new(mach_clock::Timeout::unlinked()));

/// `wait_queue[NUMQUEUES]` of kern/sched_prim.c: one bucket per hash value.
static WAIT_QUEUE: SyncCell<[QueueEntry; NUMQUEUES]> = SyncCell(
    UnsafeCell::new([const { QueueEntry::unlinked() }; NUMQUEUES]),
);

/// `wait_lock[NUMQUEUES]` of kern/sched_prim.c: the bucket locks.
static WAIT_LOCK: [SimpleLock; NUMQUEUES] =
    [const { SimpleLock::new() }; NUMQUEUES];

/// `stuck_threads[MAX_STUCK_THREADS]` of kern/sched_prim.c.  Only the
/// stuck-thread scan touches it, at splsched on one CPU at a time, so the
/// accesses are `Relaxed`.
static STUCK_THREADS: [AtomicPtr<Thread>; MAX_STUCK_THREADS] =
    [const { AtomicPtr::new(ptr::null_mut()) }; MAX_STUCK_THREADS];

/// `stuck_count` of kern/sched_prim.c, with the same `Relaxed` accesses as
/// [`STUCK_THREADS`].
static STUCK_COUNT: AtomicI32 = AtomicI32::new(0);

/// `do_thread_scan_debug` of kern/sched_prim.c, read by the scan under the
/// run-queue lock; the flag never changes at run time.
static DO_THREAD_SCAN_DEBUG: AtomicI32 = AtomicI32::new(0);

/// `no_dispatch_count` of kern/sched_prim.c: how often an idle processor went
/// non-idle without a dispatch.  The C incremented it and nothing read it.
static NO_DISPATCH_COUNT: AtomicI32 = AtomicI32::new(0);

/// `struct shift` of <kern/sched.h>: one `(5/8)**n` approximation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shift {
    shift1: c_int,
    shift2: c_int,
}

/// `wait_shift[32]` of kern/sched_prim.c: the shift pairs.
const WAIT_SHIFT: [Shift; 32] = [
    Shift {
        shift1: 1,
        shift2: 1,
    },
    Shift {
        shift1: 1,
        shift2: 3,
    },
    Shift {
        shift1: 1,
        shift2: -3,
    },
    Shift {
        shift1: 2,
        shift2: -7,
    },
    Shift {
        shift1: 3,
        shift2: 5,
    },
    Shift {
        shift1: 3,
        shift2: -5,
    },
    Shift {
        shift1: 4,
        shift2: -8,
    },
    Shift {
        shift1: 5,
        shift2: 7,
    },
    Shift {
        shift1: 5,
        shift2: -7,
    },
    Shift {
        shift1: 6,
        shift2: -10,
    },
    Shift {
        shift1: 7,
        shift2: 10,
    },
    Shift {
        shift1: 7,
        shift2: -9,
    },
    Shift {
        shift1: 8,
        shift2: -11,
    },
    Shift {
        shift1: 9,
        shift2: 12,
    },
    Shift {
        shift1: 9,
        shift2: -11,
    },
    Shift {
        shift1: 10,
        shift2: -13,
    },
    Shift {
        shift1: 11,
        shift2: 14,
    },
    Shift {
        shift1: 11,
        shift2: -13,
    },
    Shift {
        shift1: 12,
        shift2: -15,
    },
    Shift {
        shift1: 13,
        shift2: 17,
    },
    Shift {
        shift1: 13,
        shift2: -15,
    },
    Shift {
        shift1: 14,
        shift2: -17,
    },
    Shift {
        shift1: 15,
        shift2: 19,
    },
    Shift {
        shift1: 16,
        shift2: 18,
    },
    Shift {
        shift1: 16,
        shift2: -19,
    },
    Shift {
        shift1: 17,
        shift2: 22,
    },
    Shift {
        shift1: 18,
        shift2: 20,
    },
    Shift {
        shift1: 18,
        shift2: -20,
    },
    Shift {
        shift1: 19,
        shift2: 26,
    },
    Shift {
        shift1: 20,
        shift2: 22,
    },
    Shift {
        shift1: 20,
        shift2: -22,
    },
    Shift {
        shift1: 21,
        shift2: -27,
    },
];

/// The `sched_tick` counter, as the C read it.
pub(crate) fn sched_tick() -> c_uint {
    SCHED_TICK.load(Ordering::Relaxed)
}

/// The `min_quantum` value, as the C read it.
pub(crate) fn min_quantum() -> c_int {
    MIN_QUANTUM_TICKS.load(Ordering::Relaxed)
}

/// The `wait_hash()` macro of kern/sched_prim.c.
fn wait_hash(event: *mut c_void) -> usize {
    let bits = event as isize;
    let folded = if bits < 0 { !bits } else { bits };
    // The folded value is non-negative and the modulo is below `NUMQUEUES`, so
    // the cast cannot lose anything.
    (folded % NUMQUEUES as isize) as usize
}

/// The wait bucket `index` names.
fn wait_queue(index: usize) -> *mut QueueEntry {
    // SAFETY: `index` is below `NUMQUEUES`, the array's length, and only a
    // pointer is handed out.
    unsafe { (WAIT_QUEUE.0.get() as *mut QueueEntry).add(index) }
}

/// The wait lock `index` names.
fn wait_lock(index: usize) -> &'static SimpleLock {
    &WAIT_LOCK[index]
}

/// The timer element `init()` arms.
fn recompute_timer() -> *mut mach_clock::Timeout {
    RECOMPUTE_PRIORITIES_TIMER.0.get()
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

/// `sched_init()` of kern/sched_prim.c.
///
/// # Safety
///
/// `kern/startup.c` calls this once during the boot, before any other CPU or
/// thread can reach the scheduler.
pub(crate) unsafe fn sched_init() {
    // SAFETY: the caller promises a single-threaded boot, so the timer element
    // is unshared and the C fields are writable.
    unsafe {
        let timer = recompute_timer();
        (*timer).fcn = Some(recompute_priorities);
        (*timer).param = ptr::null_mut();
        (*timer).set = 0;
    }

    MIN_QUANTUM_TICKS.store(mach_clock::hz / 33, Ordering::Relaxed);

    // SAFETY: as above; the wait heads are unshared static storage, and
    // `SimpleLock::new()` already left the bucket locks unlocked.
    unsafe {
        for i in 0..NUMQUEUES {
            queue_init(wait_queue(i));
        }
    }

    // SAFETY: the processor module owns the processor sets and the machine
    // module the action globals, and this is the boot step that builds them.
    unsafe {
        processor::bootstrap();
        queue_init(machine::action_queue());
        (*machine::action_lock()).init();
    }

    SCHED_TICK.store(0, Ordering::Relaxed);
    // SAFETY: no other CPU is running yet, so no AST can be pending.
    unsafe { crate::kern::ast::init() };
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

        if (*th).sched_stamp != sched_tick() {
            update_priority(th);
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
pub(crate) unsafe fn thread_timeout_setup(thread: *mut Thread) {
    // SAFETY: the caller's contract; the C assigns the same six fields in the
    // same order.
    unsafe {
        (*thread).timer.fcn = Some(thread_timeout);
        (*thread).timer.param = thread.cast::<c_void>();
        (*thread).timer.set = 0;
        (*thread).depress_timer.fcn =
            Some(crate::kern::syscall_subr::depress_timeout);
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
pub(crate) unsafe extern "C" fn thread_timeout(thread: *mut c_void) {
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
pub(crate) unsafe fn assert_wait(event: *mut c_void, interruptible: c_int) {
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
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>;
    // the value is only handed back to the matching `splx()`.
    let s = unsafe { glue::splsched() };
    let add = if interruptible != 0 {
        TH_WAIT
    } else {
        TH_WAIT | TH_UNINT
    };
    if !event.is_null() {
        let index = wait_hash(event);
        let q = wait_queue(index);
        let lock = wait_lock(index);
        // SAFETY: `index` is below `NUMQUEUES`; the buckets are static
        // storage, and the hash and thread locks are the C order: the bucket
        // first.
        unsafe {
            lock.lock();
            (*thread).lock.lock();
            enqueue_tail(q, &raw mut (*thread).links);
            (*thread).wait_event = event;
            let state = (*thread).state();
            (*thread).set_state(state | add);
            (*thread).lock.unlock();
            lock.unlock();
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
    // SAFETY: `s` is the level `splsched()` returned.
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
pub(crate) unsafe fn clear_wait(
    thread: *mut Thread,
    result: c_int,
    interrupt_only: c_int,
) {
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>;
    // the value is only handed back to the matching `splx()`.
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
            let q = wait_queue(index);
            let lock = wait_lock(index);
            lock.lock();
            (*thread).lock.lock();
            if (*thread).wait_event == event {
                remqueue(q, thread.cast::<QueueEntry>());
                (*thread).wait_event = ptr::null_mut();
                event = ptr::null_mut(); // cause the wakeup below
            }
            lock.unlock();
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
    // SAFETY: `s` is the level `splsched()` returned.
    unsafe { glue::splx(s) };
}

/// `thread_wakeup_prim()` of kern/sched_prim.c.
///
/// # Safety
///
/// `event` is an opaque key; the C signature passes a `boolean_t` for
/// `one_thread` and a wait result, and the routine takes the locks it needs
/// itself.
pub(crate) unsafe fn thread_wakeup_prim(
    event: *mut c_void,
    one_thread: c_int,
    result: c_int,
) -> c_int {
    let index = wait_hash(event);
    let q = wait_queue(index);
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>;
    // the value is only handed back to the matching `splx()`.
    let s = unsafe { glue::splsched() };
    let lock = wait_lock(index);
    // SAFETY: the bucket lock is held for the whole walk; the thread lock is
    // taken around each thread's fields.
    let mut woke = false;
    unsafe {
        lock.lock();
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
        lock.unlock();
    }
    // SAFETY: `s` is the level `splsched()` returned.
    unsafe { glue::splx(s) };
    c_int::from(woke)
}

/// `thread_sleep()` of kern/sched_prim.c.
///
/// # Safety
///
/// Same contract as `assert_wait()`, plus `lock` must be a live simple lock
/// held by the current thread, as the C requires.
pub(crate) unsafe fn thread_sleep(
    event: *mut c_void,
    lock: *mut SimpleLock,
    interruptible: c_int,
) {
    // SAFETY: the caller's contract; the C asserts the event, unlocks and
    // blocks with `thread_no_continuation`.
    unsafe {
        assert_wait(event, interruptible);
        (*lock).unlock();
        thread_block(None);
    }
}

/// `thread_dispatch()` of kern/sched_prim.c.
///
/// # Safety
///
/// `thread` must be a live thread that is not on a run queue, and the caller
/// must be at splsched; the i386 context switch calls this symbol directly.
pub(crate) unsafe fn thread_dispatch(thread: *mut Thread) {
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
pub(crate) unsafe fn thread_setrun(thread: *mut Thread, may_preempt: c_int) {
    setrun(thread, may_preempt != 0);
}

/// `thread_select()` of kern/sched_prim.c: pick the thread this processor runs
/// next, possibly the current one.
///
/// # Safety
///
/// The caller must be at splsched, hold no run-queue lock, and `myprocessor`
/// must be the current processor.
unsafe fn thread_select(myprocessor: *mut Processor) -> *mut Thread {
    // SAFETY: the caller's contract; the run-queue locks serialize the queues
    // and the thread lock protects the fields read below.
    unsafe {
        (*myprocessor).first_quantum = 1;

        if (*myprocessor).runq.count > 0 {
            let thread = choose_thread(myprocessor);
            (*myprocessor).quantum = min_quantum();
            return thread;
        }

        let pset = (*myprocessor).processor_set;
        let runq = &raw mut (*pset).runq;
        (*runq).lock.lock();

        let thread = if (*runq).count == 0 {
            let thread = current_thread();
            if (*thread).state() == TH_RUN
                && (*thread).processor_set == pset
                && ((*thread).bound_processor.is_null()
                    || (*thread).bound_processor == myprocessor)
            {
                (*runq).lock.unlock();
                (*thread).lock.lock();
                if (*thread).sched_stamp != sched_tick() {
                    update_priority(thread);
                }
                (*thread).lock.unlock();
                thread
            } else {
                pset_thread(myprocessor, pset)
            }
        } else {
            let mut low = (*runq).low;
            let mut q = &raw mut (*runq).runq[low as usize];
            if queue_empty(q) != 0 {
                low += 1;
                (*runq).low = low;
                pset_thread(myprocessor, pset)
            } else {
                let thread = dequeue_head(q).cast::<Thread>();
                (*thread).runq = RUN_QUEUE_NULL;
                (*runq).count = (*runq).count.wrapping_sub(1);
                // The fixed-priority policy cannot lazily evaluate `runq.low`.
                if (*runq).count > 0 && (*pset).policies & POLICY_FIXEDPRI != 0
                {
                    while queue_empty(q) != 0 {
                        low += 1;
                        // The count guarantees a non-empty queue above; the
                        // guard keeps a corrupt queue from walking off the
                        // array as the C would.
                        if low >= NRQS as c_int {
                            glue::Panic(
                                c"kern/sched_prim.c".as_ptr(),
                                line!() as c_int,
                                c"thread_select".as_ptr(),
                                c"thread_select".as_ptr(),
                            )
                        }
                        (*runq).low = low;
                        q = &raw mut (*runq).runq[low as usize];
                    }
                }
                (*runq).lock.unlock();
                thread
            }
        };

        if (*thread).policy == POLICY_TIMESHARE {
            (*myprocessor).quantum = (*pset).set_quantum;
        } else {
            (*myprocessor).quantum = (*thread).sched_data;
        }
        thread
    }
}

/// `thread_invoke()` of kern/sched_prim.c: stop running `old_thread` and start
/// `new_thread`; `false` means a stack is not ready yet and the caller must
/// select again.
///
/// # Safety
///
/// The caller must be at splsched, hold no run-queue lock, and both threads
/// must be live.
pub(crate) unsafe fn thread_invoke(
    old_thread: *mut Thread,
    continuation: crate::kern::thread::Continuation,
    new_thread: *mut Thread,
) -> bool {
    if old_thread == new_thread {
        // SAFETY: the caller's contract; the new thread is the running one,
        // and the wakeup takes the hash lock itself.
        unsafe {
            (*new_thread).lock.lock();
            (*new_thread).set_state((*new_thread).state() & !TH_UNINT);
            (*new_thread).lock.unlock();
            thread_wakeup_prim(
                (*new_thread).state_event(),
                0,
                THREAD_AWAKENED,
            );
        }

        if continuation.is_some() {
            // SAFETY: `spl0()` and `call_continuation()` are the real asm
            // routines, and the continuation does not return into the caller.
            unsafe {
                glue::spl0();
                glue::call_continuation(continuation);
            }
        }
        return true;
    }

    // SAFETY: the caller's contract; the new thread's lock protects its state
    // and stack fields, and the old thread is the running one.
    let handoff = unsafe {
        (*new_thread).lock.lock();
        (*old_thread).stack_privilege != current_stack()
            && continuation.is_some()
    };

    if handoff {
        // SAFETY: the caller's contract and the lock taken above; every field
        // below is protected by one of the two thread locks.
        unsafe {
            match (*new_thread).state() & TH_SWAP_STATE {
                TH_SWAPPED => {
                    (*new_thread).set_state(
                        (*new_thread).state() & !(TH_SWAPPED | TH_UNINT),
                    );
                    (*new_thread).lock.unlock();
                    thread_wakeup_prim(
                        (*new_thread).state_event(),
                        0,
                        THREAD_AWAKENED,
                    );

                    (*new_thread).last_processor = current_processor();
                    ast_context(new_thread, cpu_number());
                    stack_handoff(old_thread, new_thread);

                    (*old_thread).lock.lock();
                    (*old_thread).swap_func = continuation;
                    match (*old_thread).state() {
                        TH_RUN_SUSP | TH_RUN_SUSP_HALTED
                        | TH_RUN_WAIT_SUSP => {
                            (*old_thread).set_state(
                                ((*old_thread).state() & !TH_RUN) | TH_SWAPPED,
                            );
                            if (*old_thread).wake_active() {
                                (*old_thread).set_wake_active(false);
                                (*old_thread).lock.unlock();
                                thread_wakeup_prim(
                                    (*old_thread).wake_active_event(),
                                    0,
                                    THREAD_AWAKENED,
                                );
                                glue::spl0();
                                glue::call_continuation(
                                    (*new_thread).swap_func,
                                );
                            }
                        }
                        TH_RUN_SUSP_UNINT | TH_RUN_UNINT | TH_RUN => {
                            (*old_thread)
                                .set_state((*old_thread).state() | TH_SWAPPED);
                            thread_setrun(old_thread, 0);
                        }
                        TH_RUN_WAIT_SUSP_UNINT
                        | TH_RUN_WAIT_UNINT
                        | TH_RUN_WAIT => {
                            (*old_thread).set_state(
                                ((*old_thread).state() & !TH_RUN) | TH_SWAPPED,
                            );
                        }
                        TH_RUN_IDLE => {
                            (*old_thread)
                                .set_state(TH_RUN | TH_IDLE | TH_SWAPPED);
                        }
                        _ => state_panic(old_thread),
                    }
                    (*old_thread).lock.unlock();

                    glue::spl0();
                    glue::call_continuation((*new_thread).swap_func);
                }
                TH_SW_COMING_IN => {
                    thread_swapin(new_thread);
                    (*new_thread).lock.unlock();
                    return false;
                }
                _ => (),
            }
        }
    } else {
        // SAFETY: as above; the new thread's lock is held here.
        unsafe {
            if (*new_thread).state() & TH_SWAPPED != 0
                && ((*new_thread).state() & TH_SW_COMING_IN != 0
                    || !(*new_thread).stack_alloc_try(Some(thread_continue)))
            {
                thread_swapin(new_thread);
                (*new_thread).lock.unlock();
                return false;
            }
        }
    }

    // SAFETY: the caller's contract; the lock taken above is held unless a
    // branch returned or diverged.
    unsafe {
        (*new_thread)
            .set_state((*new_thread).state() & !(TH_SWAPPED | TH_UNINT));
        (*new_thread).lock.unlock();
        thread_wakeup_prim((*new_thread).state_event(), 0, THREAD_AWAKENED);

        (*new_thread).last_processor = current_processor();
        ast_context(new_thread, cpu_number());

        let resuming = switch_context(old_thread, continuation, new_thread);
        thread_dispatch(resuming);
    }
    true
}

/// `thread_block()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must be the current thread, must not hold a spin lock, and must
/// have set its wait state first when it means to block.
pub(crate) unsafe fn thread_block(
    continuation: crate::kern::thread::Continuation,
) {
    let thread = current_thread();
    let myprocessor = current_processor();
    // SAFETY: the caller's contract; the loop runs at splsched.
    let s = unsafe { glue::splsched() };

    ast_off(cpu_number(), AST_BLOCK);

    loop {
        // SAFETY: at splsched with no run-queue lock, as `thread_select()`
        // requires.
        let new_thread = unsafe { thread_select(myprocessor) };
        if unsafe { thread_invoke(thread, continuation, new_thread) } {
            break;
        }
    }

    // SAFETY: `s` is the level `splsched()` returned.
    unsafe { glue::splx(s) };
}

/// `thread_run()` of kern/sched_prim.c: switch directly from the current
/// thread to `new_thread`, both runnable.
///
/// # Safety
///
/// The caller must be the current thread, must not hold a spin lock, and
/// `new_thread` must be live and runnable.
pub(crate) unsafe fn thread_run(
    continuation: crate::kern::thread::Continuation,
    mut new_thread: *mut Thread,
) {
    let thread = current_thread();
    let myprocessor = current_processor();
    // SAFETY: the caller's contract; the loop runs at splsched.
    let s = unsafe { glue::splsched() };

    while !unsafe { thread_invoke(thread, continuation, new_thread) } {
        // SAFETY: at splsched with no run-queue lock, as `thread_select()`
        // requires.
        new_thread = unsafe { thread_select(myprocessor) };
    }

    // SAFETY: `s` is the level `splsched()` returned.
    unsafe { glue::splx(s) };
}

/// `update_priority()` of kern/sched_prim.c: the priority catch-up of a thread
/// that has been asleep or suspended, with the `(5/8)**n` decay of used CPU.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds at splsched.
pub(crate) unsafe fn update_priority(thread: *mut Thread) {
    // SAFETY: the caller's contract; the thread lock serializes the fields
    // below.
    unsafe {
        let ticks = sched_tick().wrapping_sub((*thread).sched_stamp);
        (*thread).sched_stamp = (*thread).sched_stamp.wrapping_add(ticks);
        Thread::timer_delta(thread);

        if ticks > 30 {
            (*thread).cpu_usage = 0;
            (*thread).sched_usage = 0;
        } else {
            (*thread).cpu_usage =
                (*thread).cpu_usage.wrapping_add((*thread).cpu_delta);
            (*thread).sched_usage =
                (*thread).sched_usage.wrapping_add((*thread).sched_delta);
            // The reset branch above keeps `ticks` at most 30.
            let shift = WAIT_SHIFT[ticks as usize];
            (*thread).cpu_usage = decay_usage((*thread).cpu_usage, shift);
            (*thread).sched_usage = decay_usage((*thread).sched_usage, shift);
        }
        (*thread).cpu_delta = 0;
        (*thread).sched_delta = 0;

        if (*thread).policy == POLICY_TIMESHARE
            && (*thread).depress_priority < 0
        {
            (*thread).sched_pri = priority_computation(thread);
        }
    }
}

/// One `(usage >> shift1) +/- (usage >> -shift2)` step of the C.
fn decay_usage(usage: c_uint, shift: Shift) -> c_uint {
    if shift.shift2 > 0 {
        (usage >> (shift.shift1 as u32)) + (usage >> (shift.shift2 as u32))
    } else {
        (usage >> (shift.shift1 as u32)) - (usage >> ((-shift.shift2) as u32))
    }
}

/// `rem_runq()` of kern/sched_prim.c: take `th` off its run queue and return
/// that queue, or `RUN_QUEUE_NULL` when it was not on one.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds.
pub(crate) unsafe fn rem_runq(th: *mut Thread) -> *mut RunQueue {
    // SAFETY: the caller's contract; the run-queue lock serializes the queue
    // and the thread lock keeps `th->runq` from changing on another CPU.
    unsafe {
        let mut rq = (*th).runq;
        if rq != RUN_QUEUE_NULL {
            (*rq).lock.lock();
            if rq == (*th).runq {
                remqueue(&raw mut (*rq).runq[0], th.cast::<QueueEntry>());
                (*rq).count = (*rq).count.wrapping_sub(1);
                (*th).runq = RUN_QUEUE_NULL;
                (*rq).lock.unlock();
            } else {
                // The thread left the queue before the lock; the caller's
                // thread lock keeps it from moving again.
                (*rq).lock.unlock();
                rq = RUN_QUEUE_NULL;
            }
        }
        rq
    }
}

/// `choose_thread()` of kern/sched_prim.c: take the next thread off the
/// processor's own run queue, or hand the search to `choose_pset_thread()`.
///
/// # Safety
///
/// The caller must be at splsched and hold no run-queue lock; `myprocessor`
/// must be the current processor.
pub(crate) unsafe fn choose_thread(
    myprocessor: *mut Processor,
) -> *mut Thread {
    // SAFETY: the caller's contract; the run-queue lock serializes the queue.
    unsafe {
        let runq = &raw mut (*myprocessor).runq;
        (*runq).lock.lock();
        if (*runq).count > 0 {
            let mut i = (*runq).low;
            while i < NRQS as c_int {
                let q = &raw mut (*runq).runq[i as usize];
                if queue_empty(q) == 0 {
                    let th = dequeue_head(q).cast::<Thread>();
                    (*th).runq = RUN_QUEUE_NULL;
                    (*runq).count = (*runq).count.wrapping_sub(1);
                    (*runq).low = i;
                    (*runq).lock.unlock();
                    return th;
                }
                i += 1;
            }
            glue::Panic(
                c"kern/sched_prim.c".as_ptr(),
                line!() as c_int,
                c"choose_thread".as_ptr(),
                c"choose_thread".as_ptr(),
            )
        }
        (*runq).lock.unlock();

        let pset = (*myprocessor).processor_set;
        (*pset).runq.lock.lock();
        pset_thread(myprocessor, pset)
    }
}

/// `idle_thread_continue()` of kern/sched_prim.c: the idle loop, which parks
/// the processor until `thread_setrun()` dispatches a thread to it.
///
/// # Safety
///
/// Runs as the processor's own idle thread, at spl0 except where the C raised
/// it.
unsafe extern "C" fn idle_thread_continue() {
    let mycpu = cpu_number();
    let myprocessor = current_processor();
    // SAFETY: `myprocessor` is the current processor, so both fields live in
    // its per-CPU block.
    let threadp = unsafe { addr_of_mut!((*myprocessor).next_thread) };
    // SAFETY: as above.
    let lcount = unsafe { addr_of_mut!((*myprocessor).runq.count) };

    loop {
        // `MACH_HOST` is 1 in both configured builds, so the global count is
        // the processor set's.
        // SAFETY: as above; the processor set is the processor's own.
        let gcount = unsafe {
            addr_of_mut!((*(*myprocessor).processor_set).runq.count)
        };

        // SAFETY: `thread_setrun()` on another CPU writes `next_thread`, and
        // the counts change at interrupt level, so the loop reads all three
        // volatile, as the C did.
        while unsafe {
            threadp.read_volatile().is_null()
                && gcount.read_volatile() == 0
                && lcount.read_volatile() == 0
        } {
            if ast_scheduling_pending(mycpu) {
                // SAFETY: `ast_taken()` is the routine of kern/ast.c, and it
                // lowers the level itself.
                unsafe {
                    glue::splsched();
                    ast_clear_scheduling(mycpu);
                    crate::kern::ast::taken();
                }
            }
            machine_idle(mycpu);
        }

        // SAFETY: the idle thread raises the level before touching the
        // processor state and queues.
        let s = unsafe { glue::splsched() };

        'retry: loop {
            // SAFETY: the state is read at splsched; the idle lock protects
            // the queue and the count below.
            let state = unsafe { (*myprocessor).state };
            match state {
                PROCESSOR_DISPATCHING => {
                    // SAFETY: as above; the dispatch set `next_thread` and the
                    // state together.
                    let new_thread = unsafe { threadp.read_volatile() };
                    unsafe {
                        threadp.write_volatile(ptr::null_mut());
                        (*myprocessor).state = PROCESSOR_RUNNING;
                        if (*new_thread).policy == POLICY_TIMESHARE {
                            (*myprocessor).quantum =
                                (*(*new_thread).processor_set).set_quantum;
                        } else {
                            (*myprocessor).quantum = (*new_thread).sched_data;
                        }
                        (*myprocessor).first_quantum = 1;
                    }
                    // SAFETY: the dispatch and the run-queue locks make
                    // `new_thread` the thread this processor must run.
                    unsafe {
                        thread_run(Some(idle_thread_continue), new_thread);
                    }
                    break;
                }
                PROCESSOR_IDLE => {
                    // SAFETY: as above.
                    let pset = unsafe { (*myprocessor).processor_set };
                    unsafe { (*pset).idle_lock.lock() };
                    if unsafe { (*myprocessor).state } != PROCESSOR_IDLE {
                        // Something happened; try again.
                        // SAFETY: the state changed while the idle lock was held, so
                        // this path releases it.
                        unsafe { (*pset).idle_lock.unlock() };
                        continue 'retry;
                    }
                    let _ = NO_DISPATCH_COUNT.fetch_add(1, Ordering::Relaxed);
                    // SAFETY: the idle lock is held, and the state above
                    // confirms the processor is on the idle queue.
                    unsafe {
                        (*pset).idle_count =
                            (*pset).idle_count.wrapping_sub(1);
                        queue_remove_generic(
                            &raw mut (*pset).idle_queue,
                            myprocessor.cast::<c_void>(),
                            offset_of!(Processor, processor_queue),
                        );
                        (*myprocessor).state = PROCESSOR_RUNNING;
                        (*pset).idle_lock.unlock();
                    }
                    // SAFETY: the idle lock is released and the processor is
                    // running, so blocking here parks it as the C did.
                    unsafe { thread_block(Some(idle_thread_continue)) };
                    break;
                }
                PROCESSOR_ASSIGN | PROCESSOR_SHUTDOWN => {
                    // SAFETY: as above; a thread dispatched to a processor
                    // that is leaving must go back on a queue.
                    let new_thread = unsafe { threadp.read_volatile() };
                    if !new_thread.is_null() {
                        // SAFETY: the dispatch set `next_thread`; the thread
                        // lock protects its run-queue link.
                        unsafe {
                            threadp.write_volatile(ptr::null_mut());
                            (*new_thread).lock.lock();
                            thread_setrun(new_thread, 0);
                            (*new_thread).lock.unlock();
                        }
                    }
                    // SAFETY: the processor is leaving its set, so blocking
                    // here parks it as the C did.
                    unsafe { thread_block(Some(idle_thread_continue)) };
                    break;
                }
                _ => {
                    // SAFETY: the C prints the state and halts; the format and
                    // its two arguments are the C pair.
                    unsafe {
                        glue::printf(
                            c" Bad processor state %d (Cpu %d)\n".as_ptr(),
                            (*myprocessor).state,
                            mycpu,
                        );
                        glue::Panic(
                            c"kern/sched_prim.c".as_ptr(),
                            line!() as c_int,
                            c"idle_thread".as_ptr(),
                            c"idle_thread".as_ptr(),
                        )
                    }
                }
            }
        }
        // SAFETY: `s` is the level `splsched()` returned.
        unsafe { glue::splx(s) };
    }
}

/// `idle_thread()` of kern/sched_prim.c: the processor's idle thread start.
///
/// # Safety
///
/// `kern/startup.c` starts this as the processor's idle thread; it never
/// returns.
pub(crate) unsafe fn idle_thread() {
    // SAFETY: the caller's contract; this runs as the current thread on the
    // processor it idles.
    unsafe {
        let me = current_thread();
        (*me).stack_privilege();
        let s = glue::splsched();
        (*me).priority = NRQS as c_int - 1;
        (*me).sched_pri = NRQS as c_int - 1;

        (*me).lock.lock();
        (*me).set_state((*me).state() | TH_IDLE);
        (*me).lock.unlock();
        (*current_processor()).idle_thread = me;
        glue::splx(s);

        thread_block(Some(idle_thread_continue));
        idle_thread_continue();
    }
}

/// `sched_thread_continue()` of kern/sched_prim.c.
unsafe extern "C" fn sched_thread_continue() {
    loop {
        // SAFETY: the scan runs at spl0 with no lock held, as the C did.
        unsafe {
            mach_factor::compute();
            if sched_tick() & 1 != 0 {
                do_thread_scan();
            }
            assert_wait(ptr::null_mut(), 0);
            thread_block(Some(sched_thread_continue));
        }
    }
}

/// `sched_thread()` of kern/sched_prim.c: the scheduler thread, woken by
/// `recompute_priorities()` once a second.
///
/// # Safety
///
/// `kern/startup.c` starts this as the "sched" kernel thread; it never
/// returns.
pub(crate) unsafe fn sched_thread() {
    // SAFETY: the caller's contract; the release store publishes the thread to
    // the clock interrupt that wakes it.
    unsafe {
        SCHED_THREAD_ID.store(current_thread(), Ordering::Release);
        assert_wait(ptr::null_mut(), 0);
        thread_block(Some(sched_thread_continue));
        sched_thread_continue()
    }
}

/// `do_runq_scan()` of kern/sched_prim.c: pass one of the stuck-thread scan,
/// moving the candidates off `runq`.  `true` means the array ran out of room.
///
/// # Safety
///
/// The caller must hold no run-queue lock, and `runq` must be live.
unsafe fn do_runq_scan(runq: *mut RunQueue) -> bool {
    // SAFETY: the caller's contract; the run-queue lock is taken below.
    let s = unsafe { glue::splsched() };
    // SAFETY: as above; the candidate's fields are read under the run-queue
    // lock, which keeps the thread from moving.
    unsafe {
        (*runq).lock.lock();
        let mut count = (*runq).count;
        if count > 0 {
            let mut q = &raw mut (*runq).runq[(*runq).low as usize];
            while count > 0 {
                let mut thread = queue_first(q).cast::<Thread>();
                while queue_end(q, thread.cast::<QueueEntry>()) == 0 {
                    let next = queue_next(thread.cast::<QueueEntry>())
                        .cast::<Thread>();

                    if (*thread).state() & TH_SCHED_STATE == TH_RUN
                        && sched_tick().wrapping_sub((*thread).sched_stamp) > 1
                    {
                        if STUCK_COUNT.load(Ordering::Relaxed)
                            == MAX_STUCK_THREADS as c_int
                        {
                            (*runq).lock.unlock();
                            glue::splx(s);
                            return true;
                        }
                        // A RUN thread cannot be deallocated until it stops
                        // running, so taking it off the queue here makes the
                        // later unlocked update safe.
                        remqueue(q, thread.cast::<QueueEntry>());
                        (*runq).count = (*runq).count.wrapping_sub(1);
                        (*thread).runq = RUN_QUEUE_NULL;
                        let index =
                            STUCK_COUNT.fetch_add(1, Ordering::Relaxed);
                        STUCK_THREADS[index as usize]
                            .store(thread, Ordering::Relaxed);
                        if DO_THREAD_SCAN_DEBUG.load(Ordering::Relaxed) != 0 {
                            glue::printf(
                                c"do_runq_scan: adding thread %p\n".as_ptr(),
                                thread,
                            );
                        }
                    }
                    count -= 1;
                    thread = next;
                }
                q = q.add(1);
            }
        }
        (*runq).lock.unlock();
        glue::splx(s);
    }
    false
}

/// `do_thread_scan()` of kern/sched_prim.c: pass two of the stuck-thread scan,
/// updating the priority of every thread it found.
///
/// # Safety
///
/// Runs in thread context with no lock held; the scan takes the locks it
/// needs.
pub(crate) unsafe fn do_thread_scan() {
    let mut restart_needed = false;
    loop {
        // `MACH_HOST` is 1 in both configured builds.
        // SAFETY: the all-psets lock serializes the list, and each run queue
        // has its own lock taken by `do_runq_scan()`.
        unsafe {
            let lock = processor::all_psets_lock();
            (*lock).lock();
            let head = processor::all_psets();
            // `queue_enter_tail()` links the container, so the first link is
            // the set itself; the walk follows its `all_psets` field.
            let mut pset = queue_first(head).cast::<ProcessorSet>();
            while queue_end(head, pset.cast::<QueueEntry>()) == 0 {
                if do_runq_scan(&raw mut (*pset).runq) {
                    restart_needed = true;
                    break;
                }
                pset = queue_next(&raw mut (*pset).all_psets)
                    .cast::<ProcessorSet>();
            }
            (*lock).unlock();
        }

        if !restart_needed {
            for i in 0..c_int::from(smp_get_numcpus()) {
                // SAFETY: the probe counted `i`, so its per-CPU block exists.
                let runq = unsafe { &raw mut (*percpu_at(i)).processor.runq };
                if unsafe { do_runq_scan(runq) } {
                    restart_needed = true;
                    break;
                }
            }
        }

        while STUCK_COUNT.load(Ordering::Relaxed) > 0 {
            // SAFETY: the array holds `stuck_count` live threads, and the
            // splsched level below is the one the fix-up needs.
            let (thread, s) = unsafe {
                let index =
                    (STUCK_COUNT.fetch_sub(1, Ordering::Relaxed) - 1) as usize;
                let thread = STUCK_THREADS[index]
                    .swap(ptr::null_mut(), Ordering::Relaxed);
                (thread, glue::splsched())
            };
            // SAFETY: the thread is live and off every run queue; its lock
            // protects the state and priority fields.
            unsafe {
                (*thread).lock.lock();
                if (*thread).state() & TH_SCHED_STATE == TH_RUN {
                    update_priority(thread);
                    thread_setrun(thread, 1);
                }
                (*thread).lock.unlock();
                glue::splx(s);
            }
        }

        if !restart_needed {
            break;
        }
    }
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
        let rq = rem_runq(th);
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
            if myprocessor == processor::master_processor() {
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
pub(crate) unsafe fn thread_set_timeout(t: c_int) {
    let thread = current_thread();
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>;
    // the value is only handed back to the matching `splx()`.
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
pub(crate) unsafe fn thread_bind(
    thread: *mut Thread,
    processor: *mut Processor,
) {
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>;
    // the value is only handed back to the matching `splx()`.
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
pub(crate) unsafe extern "C" fn thread_continue(old_thread: *mut Thread) {
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
pub(crate) unsafe fn compute_priority(thread: *mut Thread, resched: c_int) {
    // SAFETY: the caller's contract.
    unsafe { recompute_priority(thread, resched != 0) };
}

/// `compute_my_priority()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must hold the thread lock and know the thread is timesharing and
/// not depressed, as the C documents.
pub(crate) unsafe fn compute_my_priority(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).sched_pri = priority_computation(thread) };
}

/// `recompute_priorities()` of kern/sched_prim.c.
///
/// # Safety
///
/// Called by the timeout machinery at splsoftclock, and once at boot; `param`
/// is unused, as in C.
pub(crate) unsafe extern "C" fn recompute_priorities(_param: *mut c_void) {
    SCHED_TICK.fetch_add(1, Ordering::Relaxed);
    // SAFETY: the clock lock inside `set_timeout()` serializes against the
    // clock interrupt, exactly as the C call's did; the scheduler thread may
    // be waiting.
    unsafe {
        mach_clock::set_timeout(recompute_timer(), mach_clock::hz as c_uint);
        let thread = SCHED_THREAD_ID.load(Ordering::Acquire);
        if !thread.is_null() {
            clear_wait(thread, THREAD_AWAKENED, 0);
        }
    }
}

/// `set_pri()` of kern/sched_prim.c.
///
/// # Safety
///
/// `th` must be a live thread whose lock the caller holds, and the caller must
/// be at splsched.
pub(crate) unsafe fn set_pri(th: *mut Thread, pri: c_int, resched: c_int) {
    // SAFETY: the caller's contract.
    unsafe { set_priority(th, pri, resched != 0) };
}

/// `choose_pset_thread()` of kern/sched_prim.c.
///
/// # Safety
///
/// The caller must be at splsched and must hold `pset`'s run-queue lock;
/// `myprocessor` must be the current processor and `pset` its processor set.
pub(crate) unsafe fn choose_pset_thread(
    myprocessor: *mut Processor,
    pset: *mut ProcessorSet,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { pset_thread(myprocessor, pset) }
}
