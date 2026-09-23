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
//! `processor_init()` and `pset_init()` used to hold;
//! [`Processor::start()`], [`Processor::exit()`],
//! [`Processor::control()`], [`Processor::get_assignment()`],
//! [`ProcessorSet::reference()`], [`ProcessorSet::deallocate()`],
//! [`ProcessorSet::add_processor()`],
//! [`ProcessorSet::remove_processor()`], [`ProcessorSet::quantum_set()`],
//! [`ProcessorSet::add_thread()`], [`ProcessorSet::remove_thread()`],
//! [`Thread::change_psets()`] and the three processor-set policy
//! setters are also ported from the same file.  The adapters below
//! keep the symbols the C half calls.

use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::queue::{
    QueueEntry, queue_end, queue_enter_tail, queue_first, queue_init,
    queue_next, queue_remove_generic,
};
use crate::kern::thread::{BASEPRI_SYSTEM, POLICY_TIMESHARE, Thread};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::mem::offset_of;
use core::ptr;
use core::slice;

/// `NRQS` in <kern/sched.h>: one run queue per priority.
pub const NRQS: usize = 65;

/// `POLICY_LAST` in <mach/policy.h>: the highest defined policy.
const POLICY_LAST: c_int = 2;

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

/// The [`KernError`] a C `kern_return_t` stands for.
///
/// A `kern_return_t` is an `int`, and every code this module receives
/// fits a byte; one that does not cannot name a defined error and
/// becomes [`KernError::Failure`].
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// Whether `policy` names no policy a processor set could hold; the C
/// `invalid_policy()` of <mach/policy.h>.
fn invalid_policy(policy: c_int) -> bool {
    policy <= 0 || policy > POLICY_LAST
}

/// Whether `priority` is outside the `NRQS` run queues; the C
/// `invalid_pri()` of <kern/sched.h>.
fn invalid_pri(priority: c_int) -> bool {
    match usize::try_from(priority) {
        Ok(priority) => priority >= NRQS,
        Err(_) => true,
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

    /// Start the processor.  `processor_start()` of kern/processor.c.
    ///
    /// The C body can only report failure; the adapter's null check
    /// covers the other return.
    pub fn start(&mut self) -> Result<(), KernError> {
        Err(KernError::Failure)
    }

    /// Queue the processor for shutdown.  `processor_exit()` of
    /// kern/processor.c.
    pub fn exit(&mut self) -> Result<(), KernError> {
        // SAFETY: `self` is a live processor, and the C routine takes
        // the machine lock it needs itself.
        kern_error(unsafe { glue::processor_shutdown(self) })
    }

    /// Pass a control request to the machine-dependent hook.
    /// `processor_control()` of kern/processor.c.
    pub fn control(&mut self, info: &[c_int]) -> Result<(), KernError> {
        // The C count is a `natural_t`; a slice longer than one cannot
        // have come from the C boundary, but the conversion is still
        // checked.
        let Ok(count) = c_uint::try_from(info.len()) else {
            return Err(KernError::InvalidArgument);
        };

        // SAFETY: the C hook receives the slice's pointer and length,
        // and the hook only reads it.
        kern_error(unsafe {
            glue::cpu_control(self.slot_num, info.as_ptr(), count)
        })
    }

    /// Return the set the processor belongs to, taking a reference on
    /// it.  `processor_get_assignment()` of kern/processor.c.
    pub fn get_assignment(&self) -> Result<*mut ProcessorSet, KernError> {
        if self.state == PROCESSOR_SHUTDOWN || self.state == PROCESSOR_OFF_LINE
        {
            return Err(KernError::Failure);
        }

        let pset = self.processor_set;
        // SAFETY: a processor that is neither off-line nor shutting
        // down has a live set assigned, which the C dereferences here;
        // the set's lock serializes the count.
        unsafe { (*pset).reference() };
        Ok(pset)
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

    /// Add one reference to the processor set.  `pset_reference()` of
    /// kern/processor.c.
    pub fn reference(&mut self) {
        self.ref_lock.lock();
        // The C's `pset->ref_count++`; the arithmetic wraps as C's
        // does when it overflows.
        self.ref_count = self.ref_count.wrapping_add(1);
        self.ref_lock.unlock();
    }

    /// Remove one reference to the processor set, destroying it when
    /// the last reference goes.  `pset_deallocate()` of
    /// kern/processor.c.
    ///
    /// Only the `MACH_HOST` branch is ported: `MACH_HOST` is 1 in both
    /// configured kernels, so the C's `!MACH_HOST` panic is not
    /// compiled there.
    pub fn deallocate(&mut self) {
        self.ref_lock.lock();
        self.ref_count = self.ref_count.wrapping_sub(1);
        if self.ref_count > 0 {
            self.ref_lock.unlock();
            return;
        }

        // The count is zero, but `all_psets` holds an implicit
        // reference and may make new ones, and its lock dominates the
        // set lock.  Restore one reference, drop the lock and take
        // both in the C's order.
        self.ref_count = 1;
        self.ref_lock.unlock();

        // SAFETY: the lock is the C global for the list, and the C
        // order is `all_psets_lock` before the set's `ref_lock`.
        let all_psets_lock = ptr::addr_of_mut!(glue::all_psets_lock);
        unsafe {
            (*all_psets_lock).lock();
        }
        self.ref_lock.lock();
        self.ref_count = self.ref_count.wrapping_sub(1);
        if self.ref_count > 0 {
            // Someone took a reference while the lock was dropped.
            self.ref_lock.unlock();
            // SAFETY: the lock taken just above.
            unsafe {
                (*all_psets_lock).unlock();
            }
            return;
        }

        // The set is destroyable.  The C's paranoia checks, with its
        // message.
        let is_default = ptr::from_mut(self).cast::<c_void>()
            == ptr::addr_of_mut!(glue::default_pset);
        if is_default
            || self.thread_count > 0
            || self.task_count > 0
            || self.processor_count > 0
        {
            // SAFETY: `Panic` does not return; the message and the
            // function tag are the C `panic()` call's.
            unsafe {
                glue::Panic(
                    c"kern/processor.c".as_ptr(),
                    line!() as c_int,
                    c"pset_deallocate".as_ptr(),
                    c"pset_deallocate: destroy default or active pset"
                        .as_ptr(),
                )
            }
        }

        // SAFETY: the set is linked into `all_psets` and both locks
        // are held; the removal keeps the list consistent.
        unsafe {
            queue_remove_generic(
                ptr::addr_of_mut!(glue::all_psets),
                ptr::from_mut(self).cast::<c_void>(),
                offset_of!(ProcessorSet, all_psets),
            );
            glue::all_psets_count -= 1;
        }

        self.ref_lock.unlock();
        // SAFETY: the lock taken above.
        unsafe {
            (*all_psets_lock).unlock();
        }

        // SAFETY: the set came from `pset_cache` and nothing
        // references it any more; `.addr()` is the address the
        // allocator handed out.
        unsafe {
            glue::kmem_cache_free(
                ptr::addr_of_mut!(glue::pset_cache),
                ptr::from_mut(self).addr(),
            )
        };
    }

    /// Add `thread` to the set.  `pset_add_thread()` of
    /// kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the thread's lock, as
    /// the C requires, and `thread` must be live and not linked into a
    /// processor set's thread list.
    pub unsafe fn add_thread(&mut self, thread: *mut Thread) {
        // SAFETY: the caller promises a live, unlinked thread, and the
        // queue's links are its `pset_threads` field.
        unsafe {
            queue_enter_tail(
                &raw mut self.threads,
                thread.cast::<c_void>(),
                offset_of!(Thread, pset_threads),
            );
            (*thread).processor_set = ptr::from_mut(self);
            self.thread_count = self.thread_count.wrapping_add(1);
        }
    }

    /// Remove `thread` from the set.  `pset_remove_thread()` of
    /// kern/processor.c.
    ///
    /// The set's reference to the thread is not dropped here; the C
    /// requires the caller to `pset_deallocate()` it.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the thread's lock, as
    /// the C requires, and `thread` must be live and linked into this
    /// set's thread list.
    pub unsafe fn remove_thread(&mut self, thread: *mut Thread) {
        // SAFETY: the caller promises a live thread linked into this
        // set, and the queue's links are its `pset_threads` field.
        unsafe {
            queue_remove_generic(
                &raw mut self.threads,
                thread.cast::<c_void>(),
                offset_of!(Thread, pset_threads),
            );
            (*thread).processor_set = ptr::null_mut();
            self.thread_count = self.thread_count.wrapping_sub(1);
        }
    }

    /// Precalculate the set's quanta from its load.  `quantum_set()` of
    /// kern/processor.c.
    ///
    /// The index into `machine_quantum` is the number of threads on
    /// the set's run queue, limited to the number of processors in
    /// the set.
    pub fn quantum_set(&mut self) {
        let ncpus = self.processor_count;
        let runq_count = self.runq.count;

        // SAFETY: `min_quantum` is the live C global <kern/sched.h>
        // declares and kern/sched_prim.c sets.
        let min_quantum = unsafe { glue::min_quantum };
        // SAFETY: `self` is a live set, and the shim only forms the
        // address of its `machine_quantum` tail.
        let machine_quantum = unsafe {
            glue::processor_glue_pset_machine_quantum(ptr::from_mut(self))
        };

        for i in 1..=ncpus {
            // The C indexed with an `int`; the deliberate cast cannot
            // wrap because `1 <= i <= ncpus <= NCPUS`.
            let slot = unsafe { machine_quantum.add(i as usize) };
            // The C's `(min_quantum * ncpus) + i / 2` can overflow an
            // `int`; it wraps as the C compiled for this target does.
            let quantum =
                min_quantum.wrapping_mul(ncpus).wrapping_add(i / 2) / i;
            // SAFETY: `slot` is one of the tail's `NCPUS+1` entries.
            unsafe { slot.write(quantum) };
        }

        // SAFETY: the tail has at least two entries, so index one is
        // in bounds; the doubled value wraps as the C's does.
        unsafe {
            let first = machine_quantum.add(1).read();
            machine_quantum.write(first.wrapping_mul(2));
        }

        // The C's ternary: the run-queue count, limited to the
        // processor count.
        let i = core::cmp::min(runq_count, ncpus);
        // The C indexed with an `int`; the deliberate cast cannot wrap
        // because `0 <= i <= ncpus <= NCPUS`.
        let slot = unsafe { machine_quantum.add(i as usize) };
        // SAFETY: `slot` is one of the tail's `NCPUS+1` entries.
        self.set_quantum = unsafe { slot.read() };
    }

    /// Add `processor` to the set.  `pset_add_processor()` of
    /// kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the processor's lock,
    /// as the C requires, and `processor` must be live and not linked
    /// into any set's processor list.
    pub unsafe fn add_processor(&mut self, processor: *mut Processor) {
        // SAFETY: the caller promises a live, unlinked processor, and
        // the queue's links are its `processors` field.
        unsafe {
            queue_enter_tail(
                &raw mut self.processors,
                processor.cast::<c_void>(),
                offset_of!(Processor, processors),
            );
            (*processor).processor_set = ptr::from_mut(self);
            self.processor_count = self.processor_count.wrapping_add(1);
            self.empty = 0;
        }
        self.quantum_set();
    }

    /// Remove `processor` from the set.  `pset_remove_processor()` of
    /// kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the processor's lock,
    /// as the C requires, and `processor` must be live and linked into
    /// this set's processor list.
    ///
    /// # Panics
    ///
    /// Panics through [`glue::Panic`] when `processor` does not belong
    /// to this set, as the C `panic()` did.
    pub unsafe fn remove_processor(&mut self, processor: *mut Processor) {
        // SAFETY: the caller promises a live processor linked into
        // this set, and the check below is the C's own guard against a
        // wrong one.
        unsafe {
            if ptr::from_mut(self) != (*processor).processor_set {
                // SAFETY: `Panic` does not return; the message and the
                // function tag are the C `panic()` call's.
                glue::Panic(
                    c"kern/processor.c".as_ptr(),
                    line!() as c_int,
                    c"pset_remove_processor".as_ptr(),
                    c"pset_remove_processor: wrong pset".as_ptr(),
                )
            }
            queue_remove_generic(
                &raw mut self.processors,
                processor.cast::<c_void>(),
                offset_of!(Processor, processors),
            );
            (*processor).processor_set = ptr::null_mut();
            self.processor_count = self.processor_count.wrapping_sub(1);
        }
        self.quantum_set();
    }

    /// Allow `policy` on the set.  `processor_set_policy_enable()` of
    /// kern/processor.c.
    ///
    /// A `policy` outside the defined range reports
    /// [`KernError::InvalidArgument`].
    pub fn policy_enable(&mut self, policy: c_int) -> Result<(), KernError> {
        if invalid_policy(policy) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();
        self.policies |= policy;
        self.lock.unlock();

        Ok(())
    }

    /// Forbid `policy` on the set, sending every thread that uses it
    /// back to timesharing when `change_threads` is set.
    /// `processor_set_policy_disable()` of kern/processor.c.
    ///
    /// Timesharing cannot be forbidden, and a `policy` outside the
    /// defined range reports [`KernError::InvalidArgument`].
    pub fn policy_disable(
        &mut self,
        policy: c_int,
        change_threads: c_int,
    ) -> Result<(), KernError> {
        if policy == POLICY_TIMESHARE || invalid_policy(policy) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();

        // Disable the policy if it is enabled, then handle
        // `change_threads`, as the C does.
        if (self.policies & policy) != 0 {
            self.policies &= !policy;

            if change_threads != 0 {
                // The C walks `threads` under the set lock, with
                // `queue_first()` and `queue_next()` of <kern/queue.h>.
                let list = &raw mut self.threads;
                // SAFETY: the set lock is held, so every link is a
                // live thread whose chain stays put during the walk.
                let mut thread = unsafe { queue_first(list) }.cast::<Thread>();
                // SAFETY: as above; the head ends the walk.
                while unsafe { queue_end(list, thread.cast::<QueueEntry>()) }
                    == 0
                {
                    // SAFETY: `thread` is a live member of the list.
                    if unsafe { (*thread).policy == policy } {
                        // SAFETY: the C routine takes the thread lock
                        // itself, and timesharing is a policy this set
                        // can switch a thread to.  The C ignores the
                        // result.
                        unsafe {
                            glue::thread_policy(thread, POLICY_TIMESHARE, 0);
                        }
                    }
                    // SAFETY: `thread_policy()` does not unlink
                    // `thread`.
                    let next =
                        unsafe { queue_next(&raw mut (*thread).pset_threads) };
                    thread = next.cast::<Thread>();
                }
            }
        }

        self.lock.unlock();

        Ok(())
    }

    /// Set the set's maximum priority to `max_priority`, lowering the
    /// maximum of every thread above it when `change_threads` is set.
    /// `processor_set_max_priority()` of kern/processor.c.
    ///
    /// A priority outside the run queues reports
    /// [`KernError::InvalidArgument`].
    pub fn max_priority(
        &mut self,
        max_priority: c_int,
        change_threads: c_int,
    ) -> Result<(), KernError> {
        if invalid_pri(max_priority) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();
        self.max_priority = max_priority;

        if change_threads != 0 {
            // The walk touches the set through this raw pointer alone,
            // so no reference reborrow can invalidate the list head.
            let pset = ptr::from_mut(self);
            // SAFETY: the set lock is held, so every link is a live
            // thread whose chain stays put during the walk; `pset` is
            // this set, which `thread_max_priority()` only compares.
            unsafe {
                let list = ptr::addr_of_mut!((*pset).threads);
                let mut thread = queue_first(list).cast::<Thread>();
                while queue_end(list, thread.cast::<QueueEntry>()) == 0 {
                    // SAFETY: `thread` is a live member of the list.
                    if (*thread).max_priority < max_priority {
                        // SAFETY: the C routine takes the thread lock
                        // itself.  The C ignores the result.
                        glue::thread_max_priority(thread, pset, max_priority);
                    }
                    // SAFETY: `thread_max_priority()` does not unlink
                    // `thread`.
                    thread = queue_next(&raw mut (*thread).pset_threads)
                        .cast::<Thread>();
                }
            }
        }

        self.lock.unlock();

        Ok(())
    }
}

impl Thread {
    /// Move `thread` from `old_pset`'s thread list to `new_pset`'s.
    /// `thread_change_psets()` of kern/processor.c.
    ///
    /// Unlike the paired remove and add, the C does not null
    /// `thread->processor_set` between the two lists; only the final
    /// assignment happens here.
    ///
    /// # Safety
    ///
    /// The caller must hold the locks of both sets and of the thread,
    /// as the C requires, and `thread` must be live and linked into
    /// `old_pset`'s thread list.  The old set's reference is not
    /// dropped here; the C requires the caller to `pset_deallocate()`
    /// it.
    pub unsafe fn change_psets(
        thread: *mut Thread,
        old_pset: *mut ProcessorSet,
        new_pset: *mut ProcessorSet,
    ) {
        // SAFETY: the caller promises live sets and a thread linked
        // into the old one; the queue's links are the thread's
        // `pset_threads` field.
        unsafe {
            queue_remove_generic(
                &raw mut (*old_pset).threads,
                thread.cast::<c_void>(),
                offset_of!(Thread, pset_threads),
            );
            (*old_pset).thread_count =
                (*old_pset).thread_count.wrapping_sub(1);
            queue_enter_tail(
                &raw mut (*new_pset).threads,
                thread.cast::<c_void>(),
                offset_of!(Thread, pset_threads),
            );
            (*thread).processor_set = new_pset;
            (*new_pset).thread_count =
                (*new_pset).thread_count.wrapping_add(1);
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

/// Start a processor.  `processor_start()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_start(pr: *mut Processor) -> c_int {
    let Some(pr) = ptr::NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).start() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Queue a processor for shutdown.  `processor_exit()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_exit(pr: *mut Processor) -> c_int {
    let Some(pr) = ptr::NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).exit() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Pass a control request to a processor.  `processor_control()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `info`
/// must be readable for `count` integers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_control(
    pr: *mut Processor,
    info: *mut c_int,
    count: c_uint,
) -> c_int {
    let Some(pr) = ptr::NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // A zero count gets the empty slice, so a null `info` is never
    // formed into one, which `from_raw_parts` requires.
    let info: &[c_int] = if count == 0 {
        &[]
    } else {
        // SAFETY: the caller promises `count` readable integers.  A
        // `natural_t` widens to `usize` on both supported widths.
        unsafe { slice::from_raw_parts(info, count as usize) }
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).control(info) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Read the set a processor belongs to, taking a reference on it.
/// `processor_get_assignment()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `pset`
/// must be a valid out-parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_get_assignment(
    pr: *mut Processor,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    let Some(pr) = ptr::NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).get_assignment() } {
        Ok(assignment) => {
            // SAFETY: the caller passed the out-parameter the C
            // signature requires.
            unsafe { *pset = assignment };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// Add one reference to the processor set.  `pset_reference()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_reference(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).reference() };
}

/// Remove one reference to the processor set, destroying it when the
/// last reference goes.  `pset_deallocate()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set` that
/// the caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_deallocate(pset: *mut ProcessorSet) {
    let Some(pset) = ptr::NonNull::new(pset) else {
        // The C returns early on a null set.
        return;
    };

    // SAFETY: the caller's contract.
    unsafe { (*pset.as_ptr()).deallocate() };
}

/// Add a thread to the processor set.  `pset_add_thread()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at
/// a live thread that is not linked into a set; the caller must hold
/// both locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_thread(thread) };
}

/// Remove a thread from the processor set.  `pset_remove_thread()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at
/// a live thread linked into it; the caller must hold both locks, as
/// the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_thread(thread) };
}

/// Precalculate a processor set's quanta.  `quantum_set()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn quantum_set(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).quantum_set() };
}

/// Add a processor to a processor set.  `pset_add_processor()` of
/// kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor`
/// at a live processor that is not linked into a set; the caller must
/// hold both locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_processor(processor) };
}

/// Remove a processor from a processor set.  `pset_remove_processor()`
/// of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor`
/// at a live processor linked into it; the caller must hold both
/// locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_processor(processor) };
}

/// Move a thread between processor sets.  `thread_change_psets()` of
/// kern/processor.c.
///
/// # Safety
///
/// `thread` must point at a live thread linked into the live set
/// `old_pset`, and `new_pset` at a live set; the caller must hold the
/// locks of both sets and of the thread, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_change_psets(
    thread: *mut Thread,
    old_pset: *mut ProcessorSet,
    new_pset: *mut ProcessorSet,
) {
    // SAFETY: the caller's contract.
    unsafe { Thread::change_psets(thread, old_pset, new_pset) };
}

/// Set the maximum priority of a processor set.
/// `processor_set_max_priority()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_max_priority(
    pset: *mut ProcessorSet,
    max_priority: c_int,
    change_threads: c_int,
) -> c_int {
    let Some(pset) = ptr::NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe {
        (*pset.as_ptr()).max_priority(max_priority, change_threads)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Enable a scheduling policy on a processor set.
/// `processor_set_policy_enable()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_policy_enable(
    pset: *mut ProcessorSet,
    policy: c_int,
) -> c_int {
    let Some(pset) = ptr::NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).policy_enable(policy) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Disable a scheduling policy on a processor set.
/// `processor_set_policy_disable()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_policy_disable(
    pset: *mut ProcessorSet,
    policy: c_int,
    change_threads: c_int,
) -> c_int {
    let Some(pset) = ptr::NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).policy_disable(policy, change_threads) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}
