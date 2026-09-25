// SPDX-License-Identifier: CMU-Mach
// Derived from kern/processor.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/processor.c:
//   Copyright (c) 1993-1988 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Processors and processor sets, which `kern/processor.h` declares and
//! `kern/processor.c` used to define.

use crate::arch::i386::mp_desc::cpu_control;
use crate::arch::i386::percpu::percpu_at;
use crate::config::NCPUS;
use crate::ipc::IpcPort;
use crate::kern::debug::kpanic;
use crate::kern::ipc_host;
use crate::kern::ipc_tt::{convert_task_to_port, convert_thread_to_port};
use crate::kern::lock::SimpleLock;
use crate::kern::machine;
use crate::kern::policy::{POLICY_TIMESHARE, invalid_policy};
use crate::kern::processor_info::{
    PROCESSOR_BASIC_INFO, PROCESSOR_BASIC_INFO_COUNT,
    PROCESSOR_SET_BASIC_INFO, PROCESSOR_SET_BASIC_INFO_COUNT,
    PROCESSOR_SET_SCHED_INFO, PROCESSOR_SET_SCHED_INFO_COUNT,
    ProcessorBasicInfo, ProcessorSetBasicInfo, ProcessorSetSchedInfo,
};
use crate::kern::queue::{
    QueueEntry, queue_empty, queue_end, queue_enter_tail, queue_first,
    queue_init, queue_next, queue_remove_generic,
};
use crate::kern::sched::{
    BASEPRI_SYSTEM, NRQS, RunQueue, SCHED_SCALE, invalid_pri,
};
use crate::kern::sched_prim::min_quantum;
use crate::kern::slab::{CacheInitFlags, KmemCache, kalloc, kfree};
use crate::kern::task::{self as task, Task};
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_long, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull};

/// `PROCESSOR_OFF_LINE` in <kern/processor.h>: not in the system.
pub const PROCESSOR_OFF_LINE: c_int = 0;
pub const PROCESSOR_RUNNING: c_int = 1;
pub const PROCESSOR_IDLE: c_int = 2;
pub const PROCESSOR_DISPATCHING: c_int = 3;
pub const PROCESSOR_ASSIGN: c_int = 4;
pub const PROCESSOR_SHUTDOWN: c_int = 5;

/// `struct processor` of <kern/processor.h>.
#[repr(C)]
pub struct Processor {
    pub runq: RunQueue,
    /// `processor_queue`: the idle/assign/shutdown queue link.
    pub processor_queue: QueueEntry,
    /// `state`: one of the `PROCESSOR_*` values.
    pub state: c_int,
    /// `next_thread`: the thread to run if dispatched.
    pub next_thread: *mut Thread,
    pub idle_thread: *mut Thread,
    pub quantum: c_int,
    pub first_quantum: c_int,
    pub last_quantum: c_int,
    pub processor_set: *mut ProcessorSet,
    pub processor_set_next: *mut ProcessorSet,
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

/// `struct processor_set` of <kern/processor.h>.
#[repr(C)]
pub struct ProcessorSet {
    pub runq: RunQueue,
    pub idle_queue: QueueEntry,
    pub idle_count: c_int,
    /// `idle_lock`: protects the two fields above, at splsched.
    pub idle_lock: SimpleLock,
    pub processors: QueueEntry,
    pub processor_count: c_int,
    pub empty: c_int,
    pub tasks: QueueEntry,
    pub task_count: c_int,
    pub threads: QueueEntry,
    pub thread_count: c_int,
    pub ref_count: c_int,
    /// `ref_lock`: protects `ref_count`.
    pub ref_lock: SimpleLock,
    /// `all_psets`: the link in the global processor-set list.
    pub all_psets: QueueEntry,
    pub active: c_int,
    /// `lock`: protects everything else.
    pub lock: SimpleLock,
    /// `pset_self`: the port for operations.
    pub pset_self: *mut c_void,
    /// `pset_name_self`: the port for information.
    pub pset_name_self: *mut c_void,
    pub max_priority: c_int,
    /// `policies`: the bit vector of enabled policies.
    pub policies: c_int,
    pub set_quantum: c_int,
    pub quantum_adj_index: c_int,
    /// `quantum_adj_lock`: protects `quantum_adj_index`; the C `struct
    /// slock_irq` wraps one `struct slock`, so it is a [`SimpleLock`] here.
    pub quantum_adj_lock: SimpleLock,
    pub machine_quantum: [c_int; NCPUS + 1],
    pub mach_factor: c_long,
    pub load_average: c_long,
    pub sched_load: c_long,
}

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
    assert!(offset_of!(ProcessorSet, machine_quantum) == 1220);
    assert!(offset_of!(ProcessorSet, mach_factor) == 1232);
    assert!(offset_of!(ProcessorSet, load_average) == 1240);
    assert!(offset_of!(ProcessorSet, sched_load) == 1248);
};
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ProcessorSet>() == 1256);
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
    assert!(offset_of!(ProcessorSet, machine_quantum) == 640);
    assert!(offset_of!(ProcessorSet, mach_factor) == 652);
    assert!(offset_of!(ProcessorSet, load_average) == 656);
    assert!(offset_of!(ProcessorSet, sched_load) == 660);
};
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<ProcessorSet>() == 664);

/// `master_cpu` of <kern/cpu_number.h>: the processor that keeps time.
#[unsafe(export_name = "master_cpu")]
static mut MASTER_CPU: c_int = 0;

/// `default_pset` of <kern/processor.h>: the set every task starts in.
#[unsafe(export_name = "default_pset")]
static mut DEFAULT_PSET: ProcessorSet = ProcessorSet::zeroed();

/// `all_psets` of <kern/processor.h>: the chain of every processor set.
#[unsafe(export_name = "all_psets")]
static mut ALL_PSETS: QueueEntry = QueueEntry::unlinked();

/// `all_psets_count` of <kern/processor.h>.
#[unsafe(export_name = "all_psets_count")]
static mut ALL_PSETS_COUNT: c_int = 0;

/// `all_psets_lock` of <kern/processor.h>.
#[unsafe(export_name = "all_psets_lock")]
static mut ALL_PSETS_LOCK: SimpleLock = SimpleLock::new();

/// `master_processor` of <kern/processor.h>.
#[unsafe(export_name = "master_processor")]
static mut MASTER_PROCESSOR: *mut Processor = ptr::null_mut();

/// `pset_cache` of kern/processor.c: the `struct processor_set` slab cache.
#[unsafe(export_name = "pset_cache")]
static mut PSET_CACHE: KmemCache = KmemCache::zeroed();

/// `slave_pset` of <kern/processor.h>: the set of every CPU but the master.
#[unsafe(export_name = "slave_pset")]
static mut SLAVE_PSET: *mut ProcessorSet = ptr::null_mut();

/// The live `default_pset` static.
pub(crate) fn default_pset() -> *mut ProcessorSet {
    ptr::addr_of_mut!(DEFAULT_PSET)
}

/// The live `all_psets` queue head.
pub(crate) fn all_psets() -> *mut QueueEntry {
    ptr::addr_of_mut!(ALL_PSETS)
}

/// The live `all_psets_count` counter.
pub(crate) fn all_psets_count() -> *mut c_int {
    ptr::addr_of_mut!(ALL_PSETS_COUNT)
}

/// The live `all_psets_lock`.
pub(crate) fn all_psets_lock() -> *mut SimpleLock {
    ptr::addr_of_mut!(ALL_PSETS_LOCK)
}

/// The master processor `pset_sys_bootstrap()` selected.
pub(crate) fn master_processor() -> *mut Processor {
    // SAFETY: `pset_sys_bootstrap()` sets it before any other thread runs, and
    // it only ever changes during that boot step.
    unsafe { MASTER_PROCESSOR }
}

/// The `master_cpu` the boot set.
pub(crate) fn master_cpu() -> c_int {
    // SAFETY: the boot sets it before the clock starts.
    unsafe { MASTER_CPU }
}

/// The live `slave_pset`, or null before `pset_sys_init()` sets it.
pub(crate) fn slave_pset() -> *mut ProcessorSet {
    // SAFETY: only the boot's `pset_sys_init()` writes it.
    unsafe { SLAVE_PSET }
}

/// The `pset_cache` the boot initialized.
fn pset_cache() -> *mut KmemCache {
    ptr::addr_of_mut!(PSET_CACHE)
}

/// The data a [`ProcessorSet::info()`] call reports, one variant per accepted
/// flavor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorSetInfo {
    /// The `PROCESSOR_SET_BASIC_INFO` record.
    Basic(ProcessorSetBasicInfo),
    /// The `PROCESSOR_SET_SCHED_INFO` record.
    Sched(ProcessorSetSchedInfo),
}

/// Put an unlocked simple lock in `storage`, as the C `simple_lock_init()`
/// did.
///
/// # Safety
///
/// `storage` must point at writable [`SimpleLock`] storage that no other
/// thread can see yet.
unsafe fn init_lock(storage: *mut SimpleLock) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe { storage.write(SimpleLock::new()) };
}

/// Self-link the `NRQS` run-queue heads of `runq`, as the C `queue_init()`
/// loop did.
///
/// # Safety
///
/// `runq` must point at writable storage for a [`RunQueue`] that no other
/// thread can see yet.
unsafe fn init_runq(runq: *mut RunQueue) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe {
        init_lock(&raw mut (*runq).lock);
        (*runq).low = 0;
        (*runq).count = 0;
        for i in 0..NRQS {
            queue_init(ptr::addr_of_mut!((*runq).runq[i]));
        }
    }
}

/// The [`KernError`] a C `kern_return_t` stands for.
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// Narrow the C `long` a processor-set field holds to the `integer_t` a record
/// member holds.
fn narrow_long(value: c_long) -> c_int {
    value as c_int
}

impl Processor {
    /// Initialize the processor for the slot `slot_num`.
    ///
    /// # Safety
    ///
    /// `pr` must point at writable storage for a [`Processor`] that no other
    /// thread can see yet; `pset_sys_bootstrap()` is the only caller.
    pub unsafe fn init(pr: *mut Self, slot_num: c_int) {
        // SAFETY: the caller promises writable, unshared storage; the writes
        // below cover the fields the C routine set, and the run-queue heads
        // are self-linked so the scheduler can walk them.
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

    /// `processor_start()` of kern/processor.c.
    pub fn start(&mut self) -> Result<(), KernError> {
        Err(KernError::Failure)
    }

    /// `processor_exit()` of kern/processor.c.
    pub fn exit(&mut self) -> Result<(), KernError> {
        // SAFETY: `self` is a live processor, and the machine routine takes
        // the processor lock it needs itself.
        unsafe { machine::shutdown(ptr::from_mut(self)) }
    }

    /// `processor_control()` of kern/processor.c.
    pub fn control(&mut self, info: &[c_int]) -> Result<(), KernError> {
        let Ok(count) = c_uint::try_from(info.len()) else {
            return Err(KernError::InvalidArgument);
        };

        // SAFETY: the slice's pointer and length agree, so `info` is valid for
        // `count` reads; the hook only prints the pointer.
        kern_error(unsafe { cpu_control(self.slot_num, info.as_ptr(), count) })
    }

    /// `processor_get_assignment()` of kern/processor.c.
    pub fn get_assignment(&self) -> Result<*mut ProcessorSet, KernError> {
        if self.state == PROCESSOR_SHUTDOWN || self.state == PROCESSOR_OFF_LINE
        {
            return Err(KernError::Failure);
        }

        let pset = self.processor_set;
        // SAFETY: a processor that is neither off-line nor shutting down has a
        // live set assigned, which the C dereferences here; the set's lock
        // serializes the count.
        unsafe { (*pset).reference() };
        Ok(pset)
    }

    /// `processor_info()` of kern/processor.c.
    pub fn info(
        &self,
        flavor: c_int,
        count: c_uint,
    ) -> Result<ProcessorBasicInfo, KernError> {
        if flavor != PROCESSOR_BASIC_INFO || count < PROCESSOR_BASIC_INFO_COUNT
        {
            return Err(KernError::Failure);
        }

        let slot_num = self.slot_num;
        // The slot is the one `processor_init()` recorded from the machine's
        // own count, so it names an entry of the C table and is not negative;
        // widening it to pointer width cannot wrap.
        let slot = slot_num as usize;
        // SAFETY: `slot` is below the configured `NCPUS`, and every field read
        // below is a plain integer.
        let machine = unsafe { &*machine::slot(slot) };

        let state = self.state;
        let running =
            state != PROCESSOR_SHUTDOWN && state != PROCESSOR_OFF_LINE;
        let is_master = ptr::eq(self, master_processor());

        Ok(ProcessorBasicInfo {
            cpu_type: machine.cpu_type,
            cpu_subtype: machine.cpu_subtype,
            running: c_int::from(running),
            slot_num,
            is_master: c_int::from(is_master),
        })
    }
}

impl ProcessorSet {
    /// The all-zero image a C `static struct processor_set` began with.
    const fn zeroed() -> Self {
        Self {
            runq: RunQueue {
                runq: [const { QueueEntry::unlinked() }; NRQS],
                lock: SimpleLock::new(),
                low: 0,
                count: 0,
            },
            idle_queue: QueueEntry::unlinked(),
            idle_count: 0,
            idle_lock: SimpleLock::new(),
            processors: QueueEntry::unlinked(),
            processor_count: 0,
            empty: 0,
            tasks: QueueEntry::unlinked(),
            task_count: 0,
            threads: QueueEntry::unlinked(),
            thread_count: 0,
            ref_count: 0,
            ref_lock: SimpleLock::new(),
            all_psets: QueueEntry::unlinked(),
            active: 0,
            lock: SimpleLock::new(),
            pset_self: ptr::null_mut(),
            pset_name_self: ptr::null_mut(),
            max_priority: 0,
            policies: 0,
            set_quantum: 0,
            quantum_adj_index: 0,
            quantum_adj_lock: SimpleLock::new(),
            machine_quantum: [0; NCPUS + 1],
            mach_factor: 0,
            load_average: 0,
            sched_load: 0,
        }
    }

    /// Initialize the processor set.
    ///
    /// # Safety
    ///
    /// `pset` must point at writable storage for a full `struct processor_set`
    /// that no other thread can see yet; `pset_sys_bootstrap()` and
    /// `processor_set_create()` are the callers.
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
            // The quantum is set by `sched_init()` before
            // `pset_sys_bootstrap()` calls this init.
            let quantum_min = min_quantum();
            (*pset).set_quantum = quantum_min;
            (*pset).quantum_adj_index = 0;
            init_lock(&raw mut (*pset).quantum_adj_lock);
            for quantum in (*pset).machine_quantum.iter_mut() {
                *quantum = quantum_min;
            }
            (*pset).mach_factor = 0;
            (*pset).load_average = 0;
            (*pset).sched_load = c_long::from(SCHED_SCALE);
        }
    }

    /// `pset_reference()` of kern/processor.c.
    pub fn reference(&mut self) {
        self.ref_lock.lock();
        self.ref_count = self.ref_count.wrapping_add(1);
        self.ref_lock.unlock();
    }

    /// `pset_deallocate()` of kern/processor.c.
    pub fn deallocate(&mut self) {
        self.ref_lock.lock();
        self.ref_count = self.ref_count.wrapping_sub(1);
        if self.ref_count > 0 {
            self.ref_lock.unlock();
            return;
        }

        self.ref_count = 1;
        self.ref_lock.unlock();

        // SAFETY: the lock guards the list, and the C order is
        // `all_psets_lock` before the set's `ref_lock`.
        let all_psets_lock = all_psets_lock();
        unsafe {
            (*all_psets_lock).lock();
        }
        self.ref_lock.lock();
        self.ref_count = self.ref_count.wrapping_sub(1);
        if self.ref_count > 0 {
            self.ref_lock.unlock();
            // SAFETY: the lock taken just above.
            unsafe {
                (*all_psets_lock).unlock();
            }
            return;
        }

        let is_default =
            ptr::eq(ptr::from_ref(self), default_pset().cast_const());
        if is_default
            || self.thread_count > 0
            || self.task_count > 0
            || self.processor_count > 0
        {
            kpanic!(
                "pset_deallocate",
                "pset_deallocate: destroy default or active pset"
            )
        }

        // SAFETY: the set is linked into `all_psets` and both locks are held;
        // the removal keeps the list consistent.
        unsafe {
            queue_remove_generic(
                all_psets(),
                ptr::from_mut(self).cast::<c_void>(),
                offset_of!(ProcessorSet, all_psets),
            );
            let count = all_psets_count();
            *count = (*count).wrapping_sub(1);
        }

        self.ref_lock.unlock();
        // SAFETY: the lock taken above.
        unsafe {
            (*all_psets_lock).unlock();
        }

        // SAFETY: the set came from `pset_cache` and nothing references it any
        // more; `.addr()` is the address the allocator handed out.
        unsafe {
            (*pset_cache()).free(ptr::NonNull::from_mut(self).cast::<u8>())
        };
    }

    /// `pset_add_thread()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the thread's lock, as the C
    /// requires, and `thread` must be live and not linked into a processor
    /// set's thread list.
    pub unsafe fn add_thread(&mut self, thread: *mut Thread) {
        // SAFETY: the caller promises a live, unlinked thread, and the queue's
        // links are its `pset_threads` field.
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

    /// `pset_remove_thread()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the thread's lock, as the C
    /// requires, and `thread` must be live and linked into this set's thread
    /// list.
    pub unsafe fn remove_thread(&mut self, thread: *mut Thread) {
        // SAFETY: the caller promises a live thread linked into this set, and
        // the queue's links are its `pset_threads` field.
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

    /// `pset_add_task()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the task's lock, as the C
    /// requires, and `task` must be live and not linked into a processor set.
    pub unsafe fn add_task(&mut self, task: *mut Task) {
        // SAFETY: the caller promises a live, unlinked task, and the queue's
        // links are its `pset_tasks` field.
        unsafe {
            queue_enter_tail(
                &raw mut self.tasks,
                task.cast::<c_void>(),
                offset_of!(Task, pset_tasks),
            );
            (*task).processor_set = ptr::from_mut(self);
            self.task_count = self.task_count.wrapping_add(1);
        }
    }

    /// `pset_remove_task()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the task's lock, as the C
    /// requires; `task` must be live, and the routine is a no-op unless it is
    /// linked into this set.
    pub unsafe fn remove_task(&mut self, task: *mut Task) {
        if ptr::from_mut(self) != unsafe { (*task).processor_set } {
            return;
        }

        // SAFETY: the caller promises a live task linked into this set, and
        // the queue's links are its `pset_tasks` field.
        unsafe {
            queue_remove_generic(
                &raw mut self.tasks,
                task.cast::<c_void>(),
                offset_of!(Task, pset_tasks),
            );
            (*task).processor_set = ptr::null_mut();
            self.task_count = self.task_count.wrapping_sub(1);
        }
    }

    /// `quantum_set()` of kern/processor.c.
    pub fn quantum_set(&mut self) {
        let ncpus = self.processor_count;
        let runq_count = self.runq.count;

        // The quantum `sched_init()` stored before `pset_sys_bootstrap()`
        // built this set.
        let quantum_min = min_quantum();

        for i in 1..=ncpus {
            let quantum =
                quantum_min.wrapping_mul(ncpus).wrapping_add(i / 2) / i;
            // The C indexed with an `int`; the cast cannot wrap because
            // `1 <= i <= ncpus`, and a set holds at most `NCPUS` processors.
            let Some(slot) = self.machine_quantum.get_mut(i as usize) else {
                break;
            };
            *slot = quantum;
        }

        // The tail has at least two entries, so index one exists; the
        // doubled value wraps as the C's does.
        if let [first, second, ..] = &mut self.machine_quantum[..] {
            *first = second.wrapping_mul(2);
        }

        let i = core::cmp::min(runq_count, ncpus);
        // The C indexed with an `int`; the cast cannot wrap because
        // `0 <= i <= ncpus <= NCPUS`.
        if let Some(slot) = self.machine_quantum.get(i as usize) {
            self.set_quantum = *slot;
        }
    }

    /// `pset_add_processor()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the processor's lock, as the C
    /// requires, and `processor` must be live and not linked into any set's
    /// processor list.
    pub unsafe fn add_processor(&mut self, processor: *mut Processor) {
        // SAFETY: the caller promises a live, unlinked processor, and the
        // queue's links are its `processors` field.
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

    /// `pset_remove_processor()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the set's lock and the processor's lock, as the C
    /// requires, and `processor` must be live and linked into this set's
    /// processor list.
    ///
    /// # Panics
    ///
    /// Panics through [`kpanic!`] when `processor` does not belong to this
    /// set, as the C `panic()` did.
    pub unsafe fn remove_processor(&mut self, processor: *mut Processor) {
        // SAFETY: the caller promises a live processor linked into this set,
        // and the check below is the C's own guard against a wrong one.
        unsafe {
            if ptr::from_mut(self) != (*processor).processor_set {
                kpanic!(
                    "pset_remove_processor",
                    "pset_remove_processor: wrong pset"
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

    /// `processor_set_policy_enable()` of kern/processor.c.
    pub fn policy_enable(&mut self, policy: c_int) -> Result<(), KernError> {
        if invalid_policy(policy) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();
        self.policies |= policy;
        self.lock.unlock();

        Ok(())
    }

    /// `processor_set_policy_disable()` of kern/processor.c.
    pub fn policy_disable(
        &mut self,
        policy: c_int,
        change_threads: c_int,
    ) -> Result<(), KernError> {
        if policy == POLICY_TIMESHARE || invalid_policy(policy) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();

        if (self.policies & policy) != 0 {
            self.policies &= !policy;

            if change_threads != 0 {
                let list = &raw mut self.threads;
                // SAFETY: the set lock is held, so every link is a live thread
                // whose chain stays put during the walk.
                let mut thread = unsafe { queue_first(list) }.cast::<Thread>();
                // SAFETY: as above; the head ends the walk.
                while unsafe { queue_end(list, thread.cast::<QueueEntry>()) }
                    == 0
                {
                    // SAFETY: `thread` is a live member of the list.
                    if unsafe { (*thread).policy == policy } {
                        // SAFETY: the Rust `Thread::policy()` takes the thread
                        // lock itself, and timesharing is a policy this set
                        // can switch a thread to.
                        unsafe {
                            let _ =
                                Thread::policy(thread, POLICY_TIMESHARE, 0);
                        }
                    }
                    // SAFETY: `Thread::policy()` does not unlink `thread`.
                    let next =
                        unsafe { queue_next(&raw mut (*thread).pset_threads) };
                    thread = next.cast::<Thread>();
                }
            }
        }

        self.lock.unlock();

        Ok(())
    }

    /// `processor_set_max_priority()` of kern/processor.c.
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
            let pset = ptr::from_mut(self);
            // SAFETY: the set lock is held, so every link is a live thread
            // whose chain stays put during the walk; `pset` is this set, which
            // `thread_max_priority()` only compares.
            unsafe {
                let list = ptr::addr_of_mut!((*pset).threads);
                let mut thread = queue_first(list).cast::<Thread>();
                while queue_end(list, thread.cast::<QueueEntry>()) == 0 {
                    // SAFETY: `thread` is a live member of the list.
                    if (*thread).max_priority < max_priority {
                        // SAFETY: the Rust `Thread::max_priority()` takes the
                        // thread lock itself.
                        let _ =
                            Thread::max_priority(thread, pset, max_priority);
                    }
                    // SAFETY: `Thread::max_priority()` does not unlink
                    // `thread`.
                    thread = queue_next(&raw mut (*thread).pset_threads)
                        .cast::<Thread>();
                }
            }
        }

        self.lock.unlock();

        Ok(())
    }

    /// `processor_set_info()` of kern/processor.c.
    pub fn info(
        &self,
        flavor: c_int,
        count: c_uint,
    ) -> Result<ProcessorSetInfo, KernError> {
        match flavor {
            PROCESSOR_SET_BASIC_INFO => {
                if count < PROCESSOR_SET_BASIC_INFO_COUNT {
                    return Err(KernError::Failure);
                }

                self.lock.lock();
                let info = ProcessorSetBasicInfo {
                    processor_count: self.processor_count,
                    task_count: self.task_count,
                    thread_count: self.thread_count,
                    load_average: narrow_long(self.load_average),
                    mach_factor: narrow_long(self.mach_factor),
                };
                self.lock.unlock();

                Ok(ProcessorSetInfo::Basic(info))
            }
            PROCESSOR_SET_SCHED_INFO => {
                if count < PROCESSOR_SET_SCHED_INFO_COUNT {
                    return Err(KernError::Failure);
                }

                self.lock.lock();
                let info = ProcessorSetSchedInfo {
                    policies: self.policies,
                    max_priority: self.max_priority,
                };
                self.lock.unlock();

                Ok(ProcessorSetInfo::Sched(info))
            }
            _ => Err(KernError::InvalidArgument),
        }
    }
}

impl Thread {
    /// `thread_change_psets()` of kern/processor.c.
    ///
    /// # Safety
    ///
    /// The caller must hold the locks of both sets and of the thread, as the C
    /// requires, and `thread` must be live and linked into `old_pset`'s thread
    /// list.
    pub unsafe fn change_psets(
        thread: *mut Thread,
        old_pset: *mut ProcessorSet,
        new_pset: *mut ProcessorSet,
    ) {
        // SAFETY: the caller promises live sets and a thread linked into the
        // old one; the queue's links are the thread's `pset_threads` field.
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

/// Which member list `ProcessorSet::things()` copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thing {
    Task,
    Thread,
}

impl ProcessorSet {
    /// `processor_set_destroy()` of kern/processor.c: reassign everything in
    /// the set and release it.
    ///
    /// # Safety
    ///
    /// `self` must be a live set the caller holds a reference to, and the set
    /// must not be the default one.
    pub unsafe fn destroy(&mut self) -> Result<(), KernError> {
        let default = default_pset();
        if ptr::eq(self, default) {
            return Err(KernError::InvalidArgument);
        }

        self.lock.lock();
        if self.active == 0 {
            self.lock.unlock();
            return Err(KernError::Failure);
        }

        self.active = 0;
        ipc_host::pset_disable(self);

        // SAFETY: the set lock is held, so every link is a live task whose
        // chain stays put during the walk; each reference taken here moves to
        // `task_assign()`.
        unsafe {
            while queue_empty(&raw mut self.tasks) == 0 {
                let task = queue_first(&raw mut self.tasks).cast::<Task>();
                task::reference(task);
                self.lock.unlock();
                let _ = task::assign(task, default, false);
                task::deallocate(task);
                self.lock.lock();
            }
        }

        // SAFETY: as above, for the thread list; the reference moves to
        // `thread_assign()`.
        unsafe {
            while queue_empty(&raw mut self.threads) == 0 {
                let thread =
                    queue_first(&raw mut self.threads).cast::<Thread>();
                Thread::reference(thread);
                self.lock.unlock();
                let _ = Thread::assign(thread, default);
                Thread::deallocate(thread);
                self.lock.lock();
            }
        }

        // SAFETY: as above, for the processor list; `processor_assign()`
        // takes its own set reference.
        unsafe {
            while queue_empty(&raw mut self.processors) == 0 {
                let processor =
                    queue_first(&raw mut self.processors).cast::<Processor>();
                self.lock.unlock();
                let _ = machine::assign(processor, default, true);
                self.lock.lock();
            }
        }

        self.lock.unlock();

        ipc_host::pset_terminate(self);
        self.deallocate();
        Ok(())
    }

    /// `processor_set_things()` of kern/processor.c: the task or thread ports
    /// of every member of the set, in the array `kalloc()` built.
    ///
    /// # Safety
    ///
    /// `self` must be a live set.
    unsafe fn things(
        &mut self,
        kind: Thing,
    ) -> Result<(*mut c_void, c_uint), KernError> {
        let mut size: usize = 0;
        let mut addr: *mut u8 = ptr::null_mut();

        let (actual, size_needed) = loop {
            self.lock.lock();
            if self.active == 0 {
                self.lock.unlock();
                return Err(KernError::Failure);
            }

            let count = match kind {
                Thing::Task => self.task_count,
                Thing::Thread => self.thread_count,
            };
            // A live list's count is never negative.
            let Ok(actual) = usize::try_from(count) else {
                self.lock.unlock();
                return Err(KernError::Failure);
            };

            let needed = actual.wrapping_mul(size_of::<*mut c_void>());
            if needed <= size {
                break (actual, needed);
            }

            self.lock.unlock();

            if let Some(old) = NonNull::new(addr) {
                // SAFETY: `old` came from `kalloc(size)`.
                unsafe { kfree(old, size) };
            }
            size = needed;

            let Some(buffer) = kalloc(size) else {
                return Err(KernError::ResourceShortage);
            };
            addr = buffer.as_ptr();
        };

        // SAFETY: the set is locked and active, so every one of the `actual`
        // links is a live task or thread whose chain stays put during the
        // walk; the references taken here are the ones the port conversion
        // below consumes.
        unsafe {
            match kind {
                Thing::Task => {
                    let mut task =
                        queue_first(&raw mut self.tasks).cast::<Task>();
                    for i in 0..actual {
                        task::reference(task);
                        addr.cast::<*mut Task>().add(i).write(task);
                        task = queue_next(&raw mut (*task).pset_tasks)
                            .cast::<Task>();
                    }
                }
                Thing::Thread => {
                    let mut thread =
                        queue_first(&raw mut self.threads).cast::<Thread>();
                    for i in 0..actual {
                        Thread::reference(thread);
                        addr.cast::<*mut Thread>().add(i).write(thread);
                        thread = queue_next(&raw mut (*thread).pset_threads)
                            .cast::<Thread>();
                    }
                }
            }
            self.lock.unlock();
        }

        if actual == 0 {
            if let Some(old) = NonNull::new(addr) {
                // SAFETY: `old` came from `kalloc(size)`.
                unsafe { kfree(old, size) };
            }
            return Ok((ptr::null_mut(), 0));
        }

        if size_needed < size {
            let Some(buffer) = kalloc(size_needed) else {
                // SAFETY: every slot holds a reference the port conversion
                // below never reached, and `addr` came from `kalloc(size)`.
                unsafe {
                    match kind {
                        Thing::Task => {
                            for i in 0..actual {
                                task::deallocate(
                                    addr.cast::<*mut Task>().add(i).read(),
                                );
                            }
                        }
                        Thing::Thread => {
                            for i in 0..actual {
                                Thread::deallocate(
                                    addr.cast::<*mut Thread>().add(i).read(),
                                );
                            }
                        }
                    }
                    kfree(NonNull::new_unchecked(addr), size);
                }
                return Err(KernError::ResourceShortage);
            };
            // SAFETY: both buffers are live, and `size_needed` is the byte
            // count of the references `addr` holds.
            unsafe {
                ptr::copy_nonoverlapping(addr, buffer.as_ptr(), size_needed);
                kfree(NonNull::new_unchecked(addr), size);
            }
            addr = buffer.as_ptr();
        }

        // SAFETY: every slot holds a task or thread reference, and the
        // conversion consumes it into the port the slot then holds.
        unsafe {
            match kind {
                Thing::Task => {
                    let ports = addr.cast::<*mut Task>();
                    for i in 0..actual {
                        let task = ports.add(i).read();
                        ports.add(i).write(
                            convert_task_to_port(task)
                                .map_or(ptr::null_mut(), IpcPort::as_ptr)
                                .cast::<Task>(),
                        );
                    }
                }
                Thing::Thread => {
                    let ports = addr.cast::<*mut Thread>();
                    for i in 0..actual {
                        let thread = ports.add(i).read();
                        ports.add(i).write(
                            convert_thread_to_port(thread)
                                .map_or(ptr::null_mut(), IpcPort::as_ptr)
                                .cast::<Thread>(),
                        );
                    }
                }
            }
        }

        // `actual` came from a non-negative `c_int`, so the C's `unsigned int`
        // assignment cannot truncate it.
        Ok((addr.cast::<c_void>(), actual as c_uint))
    }
}

/// `pset_sys_bootstrap()` of kern/processor.c: build the default set and the
/// processor records so the scheduler can run.
///
/// # Safety
///
/// `kern/sched_prim.c`'s `sched_init()` is the only caller, and it runs during
/// the single-threaded boot before any other CPU starts.
pub(crate) unsafe fn bootstrap() {
    // SAFETY: single-threaded boot; this is the first initialization of the
    // default set, the per-CPU records and the global list.
    unsafe {
        ProcessorSet::init(default_pset());

        for i in 0..NCPUS {
            // The C indexed `percpu_array` with an `int`; `i` counts at most
            // `NCPUS`, so the narrowing cannot wrap.
            let cpu = i as c_int;
            Processor::init(
                ptr::addr_of_mut!((*percpu_at(cpu)).processor),
                cpu,
            );
        }

        MASTER_PROCESSOR =
            ptr::addr_of_mut!((*percpu_at(master_cpu())).processor);

        queue_init(all_psets());
        (*all_psets_lock()).init();
        queue_enter_tail(
            all_psets(),
            default_pset().cast::<c_void>(),
            offset_of!(ProcessorSet, all_psets),
        );
        *all_psets_count() = 1;
        (*default_pset()).active = 1;
    }
}

/// `processor_set_create()` of kern/processor.c: build a fresh set.
///
/// # Safety
///
/// `host` must be null or the live host privilege object, and no other thread
/// may reach the new set before this returns.
pub(crate) unsafe fn create(
    host: *mut c_void,
) -> Result<*mut ProcessorSet, KernError> {
    if host.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the cache was initialized by `pset_sys_init()`, and the object
    // is unshared until it is linked below.
    let Some(mem) = (unsafe { (*pset_cache()).alloc() }) else {
        return Err(KernError::ResourceShortage);
    };
    let pset = mem.as_ptr().cast::<ProcessorSet>();

    // SAFETY: `pset` is a fresh cache object; the two references are the
    // caller's two out-arguments, as the C took them.
    unsafe {
        ProcessorSet::init(pset);
        (*pset).reference();
        (*pset).reference();
        ipc_host::pset_init(&mut *pset);
        (*pset).active = 1;

        let lock = all_psets_lock();
        (*lock).lock();
        queue_enter_tail(
            all_psets(),
            pset.cast::<c_void>(),
            offset_of!(ProcessorSet, all_psets),
        );
        let count = all_psets_count();
        *count = (*count).wrapping_add(1);
        (*lock).unlock();

        ipc_host::pset_enable(&mut *pset);
    }

    Ok(pset)
}

/// The rest of the processor-set system initialization: the set cache, the
/// control port of every CPU but the master, and the slave set.
///
/// # Safety
///
/// `kern/startup.c` is the only caller; it runs after `bootstrap()` and before
/// any other CPU is started.
pub(crate) unsafe fn system_init() {
    // SAFETY: `pset_cache` is the cache storage this boot step owns, and the
    // initializer only writes the cache's own fields.
    unsafe {
        (*pset_cache()).init(
            b"processor_set",
            size_of::<ProcessorSet>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
    }

    let master = master_processor();

    for i in 0..NCPUS {
        // The C indexed `percpu_array` with an `int`; `i` counts at most
        // `NCPUS`, so the narrowing cannot wrap.
        let cpu = i as c_int;
        // SAFETY: `i` is below `NCPUS`, the length of the C `percpu_array`.
        let processor =
            unsafe { ptr::addr_of_mut!((*percpu_at(cpu)).processor) };
        // SAFETY: `i` is below `NCPUS`, the length of the C `machine_slot`
        // array.
        let is_cpu = unsafe { (*machine::slot(i)).is_cpu } != 0;
        if processor != master && is_cpu {
            // SAFETY: the processor is a live CPU's own record, and no other
            // thread can reach its two port fields yet.
            unsafe { ipc_host::processor_init(&mut *processor) };
        }
    }

    // SAFETY: `realhost` is the live host object and `slave_pset` the pointer
    // this call sets; the set allocator takes the cache just initialized.
    unsafe {
        let result = create(crate::kern::host::realhost().cast::<c_void>());
        SLAVE_PSET = match result {
            Ok(pset) => pset,
            Err(_) => ptr::null_mut(),
        };
    }
}

/// `processor_set_tasks()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live processor set.
pub(crate) unsafe fn tasks(
    pset: *mut ProcessorSet,
) -> Result<(*mut c_void, c_uint), KernError> {
    let Some(pset) = NonNull::new(pset) else {
        return Err(KernError::InvalidArgument);
    };
    // SAFETY: the caller promises a live set.
    unsafe { (*pset.as_ptr()).things(Thing::Task) }
}

/// `processor_set_threads()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live processor set.
pub(crate) unsafe fn threads(
    pset: *mut ProcessorSet,
) -> Result<(*mut c_void, c_uint), KernError> {
    let Some(pset) = NonNull::new(pset) else {
        return Err(KernError::InvalidArgument);
    };
    // SAFETY: the caller promises a live set.
    unsafe { (*pset.as_ptr()).things(Thing::Thread) }
}
