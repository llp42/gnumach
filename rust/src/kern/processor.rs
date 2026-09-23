// SPDX-License-Identifier: CMU-Mach
// Derived from kern/processor.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/processor.c:
//   Copyright (c) 1993-1988 Carnegie Mellon University
// Derived from kern/sched.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Processors, processor sets and run queues, which `kern/processor.h`
//! and `kern/sched.h` declare.
//!
//! `RunQueue` and `Processor` are complete mirrors: a per-CPU
//! `struct percpu` embeds one `struct processor`, so the offset of
//! `active_thread` inside the per-CPU block depends on this layout
//! being exact.  `ProcessorSet` is mirrored through `quantum_adj_lock`;
//! its NCPUS-sized tail (`machine_quantum` through `sched_load`) is
//! set by the `kern/processor_glue.c` shim, because its offset depends
//! on the configure-time NCPUS.
//!
//! [`Processor::init()`] and [`ProcessorSet::init()`] are the bodies
//! `processor_init()` and `pset_init()` used to hold, and the adapters
//! below keep the symbols the C bootstrap calls.

use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::queue::{QueueEntry, queue_init};
use crate::kern::thread::{BASEPRI_SYSTEM, POLICY_TIMESHARE, Thread};
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr;

/// `NRQS` in <kern/sched.h>: one run queue per priority.
pub const NRQS: usize = 65;

/// `RUN_QUEUE_NULL` in <kern/sched.h>: not on any run queue.
pub const RUN_QUEUE_NULL: *mut RunQueue = core::ptr::null_mut();

/// `PROCESSOR_OFF_LINE` in <kern/processor.h>: not in the system.
pub const PROCESSOR_OFF_LINE: c_int = 0;
/// `PROCESSOR_RUNNING`: running normally.
pub const PROCESSOR_RUNNING: c_int = 1;
/// `PROCESSOR_IDLE`: idle.
pub const PROCESSOR_IDLE: c_int = 2;
/// `PROCESSOR_DISPATCHING`: dispatching an idle processor.
pub const PROCESSOR_DISPATCHING: c_int = 3;
/// `PROCESSOR_ASSIGN`: assignment is changing.
pub const PROCESSOR_ASSIGN: c_int = 4;
/// `PROCESSOR_SHUTDOWN`: being shut down.
pub const PROCESSOR_SHUTDOWN: c_int = 5;

/// `struct run_queue` of <kern/sched.h>: the `NRQS` priority queues
/// and their lock.
#[repr(C)]
pub struct RunQueue {
    /// `runq`: one queue per priority.
    pub runq: [QueueEntry; NRQS],
    /// `lock`: one lock for all the queues, taken at splsched.
    pub lock: SimpleLock,
    /// `low`: the lowest non-empty queue.
    pub low: c_int,
    /// `count`: the number of runnable threads.
    pub count: c_int,
}

/// `struct processor` of <kern/processor.h>.
#[repr(C)]
pub struct Processor {
    /// `runq`: the processor-local run queue.
    pub runq: RunQueue,
    /// `processor_queue`: the idle/assign/shutdown queue link.
    pub processor_queue: QueueEntry,
    /// `state`: one of the `PROCESSOR_*` values.
    pub state: c_int,
    /// `next_thread`: the thread to run if dispatched.
    pub next_thread: *mut Thread,
    /// `idle_thread`: this processor's idle thread.
    pub idle_thread: *mut Thread,
    /// `quantum`: the quantum for the current thread.
    pub quantum: c_int,
    /// `first_quantum`: whether this is the first quantum in a row.
    pub first_quantum: c_int,
    /// `last_quantum`: the last quantum assigned.
    pub last_quantum: c_int,
    /// `processor_set`: the set this processor belongs to.
    pub processor_set: *mut ProcessorSet,
    /// `processor_set_next`: the set it will belong to.
    pub processor_set_next: *mut ProcessorSet,
    /// `processors`: the set's processor list.
    pub processors: QueueEntry,
    /// `lock`: taken at splsched.
    pub lock: SimpleLock,
    /// `processor_self`: the port for operations.
    pub processor_self: *mut c_void,
    /// `processor_name_self`: the unprivileged name port.
    pub processor_name_self: *mut c_void,
    /// `slot_num`: the machine-independent slot number.
    pub slot_num: c_int,
    /// `ast_check_data`: for remote `ast_check()` invocation.
    pub ast_check_data: c_int,
}

/// `struct processor_set` of <kern/processor.h>, mirrored through
/// `quantum_adj_lock`.
///
/// The record continues in C: the NCPUS-sized `machine_quantum` and
/// the three load fields that follow it are set by the
/// `kern/processor_glue.c` shim, whose offset this side cannot name.
#[repr(C)]
pub struct ProcessorSet {
    /// `runq`: the set's shared run queue.
    pub runq: RunQueue,
    /// `idle_queue`: the idle processors.
    pub idle_queue: QueueEntry,
    /// `idle_count`: how many processors are idle.
    pub idle_count: c_int,
    /// `idle_lock`: protects the two fields above, at splsched.
    pub idle_lock: SimpleLock,
    /// `processors`: all processors in this set.
    pub processors: QueueEntry,
    /// `processor_count`: how many processors are in the set.
    pub processor_count: c_int,
    /// `empty`: true when the set has no processors.
    pub empty: c_int,
    /// `tasks`: the tasks assigned to the set.
    pub tasks: QueueEntry,
    /// `task_count`: how many tasks are assigned.
    pub task_count: c_int,
    /// `threads`: the threads in this set.
    pub threads: QueueEntry,
    /// `thread_count`: how many threads are in the set.
    pub thread_count: c_int,
    /// `ref_count`: the structure reference count.
    pub ref_count: c_int,
    /// `ref_lock`: protects `ref_count`.
    pub ref_lock: SimpleLock,
    /// `all_psets`: the link in the global processor-set list.
    pub all_psets: QueueEntry,
    /// `active`: whether the set is in use.
    pub active: c_int,
    /// `lock`: protects everything else.
    pub lock: SimpleLock,
    /// `pset_self`: the port for operations.
    pub pset_self: *mut c_void,
    /// `pset_name_self`: the port for information.
    pub pset_name_self: *mut c_void,
    /// `max_priority`: the maximum priority allowed.
    pub max_priority: c_int,
    /// `policies`: the bit vector of enabled policies.
    pub policies: c_int,
    /// `set_quantum`: the current default quantum.
    pub set_quantum: c_int,
    /// `quantum_adj_index`: the runtime quantum adjustment.
    pub quantum_adj_index: c_int,
    /// `quantum_adj_lock`: protects `quantum_adj_index`; the C
    /// `struct slock_irq` wraps one `struct slock`, so it is a
    /// [`SimpleLock`] here.
    pub quantum_adj_lock: SimpleLock,
}

// `struct run_queue`: 65 `struct queue_entry`s, then the lock word and
// the two counts; the C compiler's sizes are 1056 and 532.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<RunQueue>() == 1056);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<RunQueue>() == 532);
const _: () =
    assert!(offset_of!(RunQueue, lock) == NRQS * size_of::<QueueEntry>());
const _: () = assert!(
    offset_of!(RunQueue, low)
        == offset_of!(RunQueue, lock) + size_of::<SimpleLock>()
);
const _: () = assert!(
    offset_of!(RunQueue, count)
        == offset_of!(RunQueue, low) + size_of::<c_int>()
);

// `struct processor`: the run queue, the queue link, the state and
// pointers, the second queue link, the lock, the ports and the slot;
// 1176 and 600 bytes.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Processor>() == 1176);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<Processor>() == 600);
const _: () = assert!(offset_of!(Processor, runq) == 0);
const _: () =
    assert!(offset_of!(Processor, processor_queue) == size_of::<RunQueue>());
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(Processor, state) == 1072);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(offset_of!(Processor, state) == 540);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(Processor, next_thread) == 1080);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(offset_of!(Processor, next_thread) == 544);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(Processor, lock) == 1144);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(offset_of!(Processor, lock) == 580);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(Processor, ast_check_data) == 1172);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(offset_of!(Processor, ast_check_data) == 596);

// `struct processor_set` is mirrored through `quantum_adj_lock`; these
// are the C compiler's offsets for that prefix, which the
// configure-time NCPUS does not move.
const _: () = assert!(offset_of!(ProcessorSet, runq) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(ProcessorSet, idle_queue) == 1056);
    assert!(offset_of!(ProcessorSet, idle_count) == 1072);
    assert!(offset_of!(ProcessorSet, idle_lock) == 1076);
    assert!(offset_of!(ProcessorSet, processors) == 1080);
    assert!(offset_of!(ProcessorSet, processor_count) == 1096);
    assert!(offset_of!(ProcessorSet, empty) == 1100);
    assert!(offset_of!(ProcessorSet, tasks) == 1104);
    assert!(offset_of!(ProcessorSet, task_count) == 1120);
    assert!(offset_of!(ProcessorSet, threads) == 1128);
    assert!(offset_of!(ProcessorSet, thread_count) == 1144);
    assert!(offset_of!(ProcessorSet, ref_count) == 1148);
    assert!(offset_of!(ProcessorSet, ref_lock) == 1152);
    assert!(offset_of!(ProcessorSet, all_psets) == 1160);
    assert!(offset_of!(ProcessorSet, active) == 1176);
    assert!(offset_of!(ProcessorSet, lock) == 1180);
    assert!(offset_of!(ProcessorSet, pset_self) == 1184);
    assert!(offset_of!(ProcessorSet, pset_name_self) == 1192);
    assert!(offset_of!(ProcessorSet, max_priority) == 1200);
    assert!(offset_of!(ProcessorSet, policies) == 1204);
    assert!(offset_of!(ProcessorSet, set_quantum) == 1208);
    assert!(offset_of!(ProcessorSet, quantum_adj_index) == 1212);
    assert!(offset_of!(ProcessorSet, quantum_adj_lock) == 1216);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(ProcessorSet, idle_queue) == 532);
    assert!(offset_of!(ProcessorSet, idle_count) == 540);
    assert!(offset_of!(ProcessorSet, idle_lock) == 544);
    assert!(offset_of!(ProcessorSet, processors) == 548);
    assert!(offset_of!(ProcessorSet, processor_count) == 556);
    assert!(offset_of!(ProcessorSet, empty) == 560);
    assert!(offset_of!(ProcessorSet, tasks) == 564);
    assert!(offset_of!(ProcessorSet, task_count) == 572);
    assert!(offset_of!(ProcessorSet, threads) == 576);
    assert!(offset_of!(ProcessorSet, thread_count) == 584);
    assert!(offset_of!(ProcessorSet, ref_count) == 588);
    assert!(offset_of!(ProcessorSet, ref_lock) == 592);
    assert!(offset_of!(ProcessorSet, all_psets) == 596);
    assert!(offset_of!(ProcessorSet, active) == 604);
    assert!(offset_of!(ProcessorSet, lock) == 608);
    assert!(offset_of!(ProcessorSet, pset_self) == 612);
    assert!(offset_of!(ProcessorSet, pset_name_self) == 616);
    assert!(offset_of!(ProcessorSet, max_priority) == 620);
    assert!(offset_of!(ProcessorSet, policies) == 624);
    assert!(offset_of!(ProcessorSet, set_quantum) == 628);
    assert!(offset_of!(ProcessorSet, quantum_adj_index) == 632);
    assert!(offset_of!(ProcessorSet, quantum_adj_lock) == 636);
};

/// Put an unlocked simple lock in `storage`, as the C
/// `simple_lock_init()` did.  The write happens before any reference
/// is formed, so the storage may still be uninitialized on entry.
///
/// # Safety
///
/// `storage` must point at writable [`SimpleLock`] storage that no
/// other thread can see yet.
unsafe fn init_lock(storage: *mut SimpleLock) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe { storage.write(SimpleLock::new()) };
}

/// Self-link the `NRQS` run-queue heads of `runq`, as the C
/// `queue_init()` loop did.
///
/// # Safety
///
/// `runq` must point at writable storage for a [`RunQueue`] that no
/// other thread can see yet.
unsafe fn init_runq(runq: *mut RunQueue) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe {
        init_lock(&raw mut (*runq).lock);
        (*runq).low = 0;
        (*runq).count = 0;
        for i in 0..NRQS {
            // `addr_of_mut!` names the element without forming a
            // reference over storage that is not initialized yet.
            queue_init(ptr::addr_of_mut!((*runq).runq[i]));
        }
    }
}

impl Processor {
    /// Initialize the processor for the slot `slot_num`.  The body of
    /// `processor_init()` in kern/processor.c.
    ///
    /// `ast_check_data` is left alone, as the C leaves it:
    /// `init_ast_check()` fills it in later.
    ///
    /// # Safety
    ///
    /// `pr` must point at writable storage for a [`Processor`] that no
    /// other thread can see yet; `pset_sys_bootstrap()` is the only
    /// caller.
    pub unsafe fn init(pr: *mut Self, slot_num: c_int) {
        // SAFETY: the caller promises writable, unshared storage; the
        // writes below cover the fields the C routine set, and the
        // run-queue heads are self-linked so the scheduler can walk
        // them.
        unsafe {
            init_runq(&raw mut (*pr).runq);
            queue_init(&raw mut (*pr).processor_queue);
            (*pr).state = PROCESSOR_OFF_LINE;
            (*pr).next_thread = ptr::null_mut();
            (*pr).idle_thread = ptr::null_mut();
            (*pr).quantum = 0;
            (*pr).first_quantum = 0;
            (*pr).last_quantum = 0;
            (*pr).processor_set = ptr::null_mut();
            (*pr).processor_set_next = ptr::null_mut();
            queue_init(&raw mut (*pr).processors);
            init_lock(&raw mut (*pr).lock);
            (*pr).processor_self = ptr::null_mut();
            (*pr).processor_name_self = ptr::null_mut();
            (*pr).slot_num = slot_num;
        }
    }
}

impl ProcessorSet {
    /// Initialize the processor set.  The body of `pset_init()` in
    /// kern/processor.c.
    ///
    /// `ref_count` starts at one, `empty` at true and `active` at
    /// false; `max_priority` starts at [`BASEPRI_SYSTEM`] so that
    /// privileged tasks can raise it later.  The NCPUS-sized tail is
    /// set by the glue shim, which receives `min_quantum` for the
    /// `machine_quantum` entries.
    ///
    /// # Safety
    ///
    /// `pset` must point at writable storage for a full `struct
    /// processor_set` that no other thread can see yet;
    /// `pset_sys_bootstrap()` and `processor_set_create()` are the
    /// callers.
    pub unsafe fn init(pset: *mut Self) {
        // SAFETY: the caller promises writable, unshared storage.
        unsafe {
            init_runq(&raw mut (*pset).runq);
            queue_init(&raw mut (*pset).idle_queue);
            (*pset).idle_count = 0;
            init_lock(&raw mut (*pset).idle_lock);
            queue_init(&raw mut (*pset).processors);
            (*pset).processor_count = 0;
            (*pset).empty = 1;
            queue_init(&raw mut (*pset).tasks);
            (*pset).task_count = 0;
            queue_init(&raw mut (*pset).threads);
            (*pset).thread_count = 0;
            (*pset).ref_count = 1;
            init_lock(&raw mut (*pset).ref_lock);
            queue_init(&raw mut (*pset).all_psets);
            (*pset).active = 0;
            init_lock(&raw mut (*pset).lock);
            (*pset).pset_self = ptr::null_mut();
            (*pset).pset_name_self = ptr::null_mut();
            (*pset).max_priority = BASEPRI_SYSTEM;
            (*pset).policies = POLICY_TIMESHARE;
            // SAFETY: `min_quantum` is the live C global <kern/sched.h>
            // declares and kern/sched_prim.c sets; `pset_sys_bootstrap`
            // runs after it is set.
            let min_quantum = glue::min_quantum;
            (*pset).set_quantum = min_quantum;
            (*pset).quantum_adj_index = 0;
            init_lock(&raw mut (*pset).quantum_adj_lock);
            glue::processor_glue_pset_tail_init(pset, min_quantum);
        }
    }
}

/// Initialize the processor in slot `slot_num`.  `processor_init()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pr` must point at writable storage for a [`Processor`] that no
/// other thread can see yet, as `pset_sys_bootstrap()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_init(pr: *mut Processor, slot_num: c_int) {
    // SAFETY: the caller's contract.
    unsafe { Processor::init(pr, slot_num) };
}

/// Initialize the processor set.  `pset_init()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at writable storage for a full `struct
/// processor_set` that no other thread can see yet;
/// `pset_sys_bootstrap()` and `processor_set_create()` are the
/// callers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_init(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ProcessorSet::init(pset) };
}
