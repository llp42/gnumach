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
use crate::glue;
use crate::kern::ipc_host;
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
    QueueEntry, queue_end, queue_enter_tail, queue_first, queue_init,
    queue_next, queue_remove_generic,
};
use crate::kern::sched::{
    BASEPRI_SYSTEM, NRQS, RunQueue, SCHED_SCALE, invalid_pri,
};
use crate::kern::slab::CacheInitFlags;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_long, c_uint, c_void};
use core::mem::offset_of;
use core::ptr;
use core::slice;

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
        // SAFETY: `self` is a live processor, and the C routine takes the
        // machine lock it needs itself.
        kern_error(unsafe { glue::processor_shutdown(self) })
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
        // SAFETY: `master_processor` is the live C global, which
        // `pset_sys_bootstrap()` points at the master slot.
        let is_master = ptr::eq(self, unsafe { glue::master_processor });

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
            // SAFETY: `min_quantum` is the live C global <kern/sched.h>
            // declares and kern/sched_prim.c sets; `pset_sys_bootstrap` runs
            // after it is set.
            let min_quantum = glue::min_quantum;
            (*pset).set_quantum = min_quantum;
            (*pset).quantum_adj_index = 0;
            init_lock(&raw mut (*pset).quantum_adj_lock);
            for quantum in (*pset).machine_quantum.iter_mut() {
                *quantum = min_quantum;
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

        // SAFETY: the lock is the C global for the list, and the C order is
        // `all_psets_lock` before the set's `ref_lock`.
        let all_psets_lock = ptr::addr_of_mut!(glue::all_psets_lock);
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

        let is_default = ptr::from_mut(self).cast::<c_void>()
            == ptr::addr_of_mut!(glue::default_pset);
        if is_default
            || self.thread_count > 0
            || self.task_count > 0
            || self.processor_count > 0
        {
            // SAFETY: `Panic` does not return; the message and the function
            // tag are the C `panic()` call's.
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

        // SAFETY: the set is linked into `all_psets` and both locks are held;
        // the removal keeps the list consistent.
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

        // SAFETY: the set came from `pset_cache` and nothing references it any
        // more; `.addr()` is the address the allocator handed out.
        unsafe {
            (*ptr::addr_of_mut!(glue::pset_cache))
                .free(ptr::NonNull::from_mut(self).cast::<u8>())
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

    /// `quantum_set()` of kern/processor.c.
    pub fn quantum_set(&mut self) {
        let ncpus = self.processor_count;
        let runq_count = self.runq.count;

        // SAFETY: `min_quantum` is the live C global <kern/sched.h> declares
        // and kern/sched_prim.c sets.
        let min_quantum = unsafe { glue::min_quantum };

        for i in 1..=ncpus {
            let quantum =
                min_quantum.wrapping_mul(ncpus).wrapping_add(i / 2) / i;
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
    /// Panics through [`glue::Panic`] when `processor` does not belong to this
    /// set, as the C `panic()` did.
    pub unsafe fn remove_processor(&mut self, processor: *mut Processor) {
        // SAFETY: the caller promises a live processor linked into this set,
        // and the check below is the C's own guard against a wrong one.
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

/// `processor_init()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must point at writable storage for a [`Processor`] that no other
/// thread can see yet, as `pset_sys_bootstrap()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_init(pr: *mut Processor, slot_num: c_int) {
    // SAFETY: the caller's contract.
    unsafe { Processor::init(pr, slot_num) };
}

/// `pset_init()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at writable storage for a full `struct processor_set`
/// that no other thread can see yet; `pset_sys_bootstrap()` and
/// `processor_set_create()` are the callers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_init(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ProcessorSet::init(pset) };
}

/// The rest of the processor-set system initialization: the set cache, the
/// control port of every CPU but the master, and the slave set.
fn system_init() {
    // SAFETY: `pset_cache` is the C cache storage this boot step owns, and the
    // initializer only writes the cache's own fields.
    unsafe {
        (*ptr::addr_of_mut!(glue::pset_cache)).init(
            b"processor_set",
            size_of::<ProcessorSet>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
    }

    // SAFETY: `pset_sys_bootstrap()` ran during the boot and pointed this at
    // the master slot.
    let master = unsafe { glue::master_processor };

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
            unsafe { ipc_host::ipc_processor_init(processor) };
        }
    }

    // SAFETY: `realhost` is the live host object and `slave_pset` the C
    // pointer this call sets; the set allocator takes the cache just
    // initialized.
    unsafe {
        glue::processor_set_create(
            ptr::addr_of_mut!(glue::realhost),
            ptr::addr_of_mut!(glue::slave_pset),
            ptr::addr_of_mut!(glue::slave_pset),
        );
    }
}

/// `pset_sys_init()` of kern/processor.c.
///
/// # Safety
///
/// `kern/startup.c` is the only caller; it runs this during boot after
/// `pset_sys_bootstrap()` and before any other CPU is started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_sys_init() {
    system_init();
}

/// `processor_start()` of kern/processor.c.
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

/// `processor_exit()` of kern/processor.c.
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

/// `processor_control()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `info` must be
/// readable for `count` integers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_control(
    pr: *mut Processor,
    info: *mut c_int,
    count: c_uint,
) -> c_int {
    let Some(pr) = ptr::NonNull::new(pr) else {
        return c_int::from(KernError::InvalidArgument);
    };

    let info: &[c_int] = if count == 0 {
        &[]
    } else {
        // SAFETY: the caller promises `count` readable integers.
        unsafe { slice::from_raw_parts(info, count as usize) }
    };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*pr.as_ptr()).control(info) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_get_assignment()` of kern/processor.c.
///
/// # Safety
///
/// `pr` must be null or point at a live `struct processor`, and `pset` must be
/// a valid out-parameter.
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
            // SAFETY: the caller passed the out-parameter the C signature
            // requires.
            unsafe { *pset = assignment };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `pset_reference()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_reference(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).reference() };
}

/// `pset_deallocate()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set` that the
/// caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_deallocate(pset: *mut ProcessorSet) {
    let Some(pset) = ptr::NonNull::new(pset) else {
        return;
    };

    // SAFETY: the caller's contract.
    unsafe { (*pset.as_ptr()).deallocate() };
}

/// `pset_add_thread()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at a live
/// thread that is not linked into a set; the caller must hold both locks, as
/// the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_thread(thread) };
}

/// `pset_remove_thread()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `thread` at a live
/// thread linked into it; the caller must hold both locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_thread(
    pset: *mut ProcessorSet,
    thread: *mut Thread,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_thread(thread) };
}

/// `quantum_set()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn quantum_set(pset: *mut ProcessorSet) {
    // SAFETY: the caller promises a live set.
    unsafe { (*pset).quantum_set() };
}

/// `pset_add_processor()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor` at a
/// live processor that is not linked into a set; the caller must hold both
/// locks, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_add_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).add_processor(processor) };
}

/// `pset_remove_processor()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` and `processor` at a
/// live processor linked into it; the caller must hold both locks, as the C
/// requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pset_remove_processor(
    pset: *mut ProcessorSet,
    processor: *mut Processor,
) {
    // SAFETY: the caller's contract.
    unsafe { (*pset).remove_processor(processor) };
}

/// `thread_change_psets()` of kern/processor.c.
///
/// # Safety
///
/// `thread` must point at a live thread linked into the live set `old_pset`,
/// and `new_pset` at a live set; the caller must hold the locks of both sets
/// and of the thread, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_change_psets(
    thread: *mut Thread,
    old_pset: *mut ProcessorSet,
    new_pset: *mut ProcessorSet,
) {
    // SAFETY: the caller's contract.
    unsafe { Thread::change_psets(thread, old_pset, new_pset) };
}

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

/// `processor_info()` of kern/processor.c.
///
/// # Safety
///
/// `processor` must be null or point at a live `struct processor`; `host` and
/// `count` must be valid out-parameters, and `info` must be writable for the
/// `processor_basic_info` that `*count` reports.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_info(
    processor: *mut Processor,
    flavor: c_int,
    host: *mut *mut c_void,
    info: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    let Some(processor) = ptr::NonNull::new(processor) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a valid `count` out-parameter.
    let capacity = unsafe { *count };

    // SAFETY: the caller promises a live processor.
    match unsafe { (*processor.as_ptr()).info(flavor, capacity) } {
        Ok(basic) => {
            // SAFETY: the count check inside `info()` guarantees the caller's
            // buffer is at least a `processor_basic_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorBasicInfo>(), basic);
                *count = PROCESSOR_BASIC_INFO_COUNT;
                *host = ptr::addr_of_mut!(glue::realhost);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `processor_set_info()` of kern/processor.c.
///
/// # Safety
///
/// `pset` must be null or point at a live `struct processor_set`; `host` and
/// `count` must be valid out-parameters, and `info` must be writable for the
/// record the flavor and `*count` call for.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_info(
    pset: *mut ProcessorSet,
    flavor: c_int,
    host: *mut *mut c_void,
    info: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    let Some(pset) = ptr::NonNull::new(pset) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a valid `count` out-parameter.
    let capacity = unsafe { *count };

    // SAFETY: the caller promises a live set.
    match unsafe { (*pset.as_ptr()).info(flavor, capacity) } {
        Ok(ProcessorSetInfo::Basic(basic)) => {
            // SAFETY: the count check inside `info()` guarantees the caller's
            // buffer is at least a `processor_set_basic_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorSetBasicInfo>(), basic);
                *count = PROCESSOR_SET_BASIC_INFO_COUNT;
                *host = ptr::addr_of_mut!(glue::realhost);
            }
            0
        }
        Ok(ProcessorSetInfo::Sched(sched)) => {
            // SAFETY: the flavor's count check guarantees the caller's buffer
            // is at least a `processor_set_sched_info`, and the caller
            // promises the other two out-parameters.
            unsafe {
                ptr::write(info.cast::<ProcessorSetSchedInfo>(), sched);
                *count = PROCESSOR_SET_SCHED_INFO_COUNT;
                *host = ptr::addr_of_mut!(glue::realhost);
            }
            0
        }
        Err(KernError::InvalidArgument) => {
            // SAFETY: the caller promises a valid `host` out-parameter.
            unsafe {
                *host = ptr::null_mut();
            }
            c_int::from(KernError::InvalidArgument)
        }
        Err(error) => c_int::from(error),
    }
}
