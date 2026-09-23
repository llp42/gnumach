// SPDX-License-Identifier: CMU-Mach
// Derived from kern/sched.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The scheduler records of `kern/sched.h`.
//!
//! `RunQueue` and its `NRQS` heads are the only definitions the ported
//! code needed so far; the rest of the header stays C until
//! `kern/sched_prim.c` moves whole.

use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use core::ffi::c_int;
use core::mem::offset_of;

/// `NRQS` in <kern/sched.h>: one run queue per priority.
pub const NRQS: usize = 65;

/// `BASEPRI_SYSTEM` in <kern/sched.h>: the priority of kernel threads.
pub const BASEPRI_SYSTEM: c_int = 6;

/// `PRI_SHIFT` in <kern/sched.h>: where a thread's usage is scaled
/// into priorities.
pub(crate) const PRI_SHIFT: u32 = 17;

/// `SCHED_SHIFT` in <kern/sched.h>: the `SCHED_SCALE` scaling of
/// `sched_usage`.
pub(crate) const SCHED_SHIFT: u32 = 7;

/// `RUN_QUEUE_NULL` in <kern/sched.h>: not on any run queue.
pub const RUN_QUEUE_NULL: *mut RunQueue = core::ptr::null_mut();

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

/// Whether `priority` is outside the `NRQS` run queues; the C
/// `invalid_pri()` of <kern/sched.h>.
pub(crate) fn invalid_pri(priority: c_int) -> bool {
    match usize::try_from(priority) {
        Ok(priority) => priority >= NRQS,
        Err(_) => true,
    }
}
