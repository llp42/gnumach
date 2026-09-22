// SPDX-License-Identifier: CMU-Mach
// Derived from kern/processor.h and kern/sched.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Processors, processor sets and run queues, which `kern/processor.h`
//! and `kern/sched.h` declare.
//!
//! `RunQueue` and `Processor` are complete mirrors: a per-CPU
//! `struct percpu` embeds one `struct processor`, so the offset of
//! `active_thread` inside the per-CPU block depends on this layout
//! being exact.  `ProcessorSet` is a prefix mirror: the scheduler
//! reaches only `runq`, `idle_queue`, `idle_count` and `idle_lock`,
//! and the rest of the record stays C for now.

use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use crate::kern::thread::Thread;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;

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

/// `struct processor_set` of <kern/processor.h>, mirrored through the
/// fields the scheduler reads.
///
/// The record continues in C (`tasks`, `threads`, `pset_self`, the
/// quantum bookkeeping); the mirror ends at `idle_lock`, the last field
/// the ported code touches.
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

// `struct processor_set` is a prefix; these are the C compiler's
// offsets for the four fields the scheduler reads.
const _: () = assert!(offset_of!(ProcessorSet, runq) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(ProcessorSet, idle_queue) == 1056);
    assert!(offset_of!(ProcessorSet, idle_count) == 1072);
    assert!(offset_of!(ProcessorSet, idle_lock) == 1076);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(ProcessorSet, idle_queue) == 532);
    assert!(offset_of!(ProcessorSet, idle_count) == 540);
    assert!(offset_of!(ProcessorSet, idle_lock) == 544);
};
