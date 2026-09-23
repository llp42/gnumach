// SPDX-License-Identifier: CMU-Mach
// Derived from kern/thread.h:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Derived from kern/thread.c:
//   Copyright (c) 1994-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread record, which `kern/thread.h` declares.

use crate::arch::i386::percpu::{current_stack, current_thread};
use crate::arch::types::VmOffset;
use crate::arch::vm_param::KERNEL_STACK_SIZE;
use crate::glue;
use crate::glue::time_value::TimeValue64;
use crate::kern::ipc_mig::mach_msg_abort_rpc;
use crate::kern::ipc_tt::ipc_thread_disable;
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::Timeout;
use crate::kern::policy::{POLICY_FIXEDPRI, POLICY_TIMESHARE, invalid_policy};
use crate::kern::processor::{Processor, ProcessorSet, pset_reference};
use crate::kern::queue::{
    QueueEntry, queue_end, queue_first, queue_init, queue_next,
};
use crate::kern::sched::{
    BASEPRI_SYSTEM, RUN_QUEUE_NULL, RunQueue, invalid_pri,
};
use crate::kern::sched_prim::{
    THREAD_AWAKENED, compute_priority, thread_setrun, thread_wakeup_prim,
};
use crate::kern::syscall_subr::thread_depress_abort;
use crate::kern::timer::{Timer, TimerSave};
use crate::kern::types::KernError;
use crate::utils::string::strncpy;
use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::mem::{MaybeUninit, offset_of};
use core::ptr;

#[cfg(target_pointer_width = "64")]
const THREAD_SIZE: usize = 560;
#[cfg(target_pointer_width = "32")]
const THREAD_SIZE: usize = 372;

/// `TASK_NAME_SIZE` in <kern/task.h>, the length of `thread.name`.
pub const TASK_NAME_SIZE: usize = 32;

/// `i386_DEBUG_STATE` in <mach/i386/thread_status.h>: the debug state, which
/// the current thread can read and write without suspending itself.
const I386_DEBUG_STATE: c_int = 6;
/// `i386_FSGS_BASE_STATE`: the segment bases, writable directly only for the
/// current thread.
const I386_FSGS_BASE_STATE: c_int = 7;
/// `STACK_MARKER` in kern/thread.c: what `stack_init()` fills a fresh kernel
/// stack with when the usage check is on.
const STACK_MARKER: u32 = 0xdead_beef;

/// The [`KernError`] a C `kern_return_t` stands for.
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// `TH_WAIT` in <kern/thread.h>: the thread is queued for waiting.
pub const TH_WAIT: u32 = 0x01;
/// `TH_SUSP`: the thread has been asked to stop.
pub const TH_SUSP: u32 = 0x02;
/// `TH_RUN`: the thread is running or on a run queue.
pub const TH_RUN: u32 = 0x04;
/// `TH_UNINT`: the thread is waiting uninterruptibly.
pub const TH_UNINT: u32 = 0x08;
/// `TH_HALTED`: the thread is halted at a clean point.
pub const TH_HALTED: u32 = 0x10;
/// `TH_IDLE`: the thread is an idle thread.
pub const TH_IDLE: u32 = 0x80;
/// `TH_SCHED_STATE`: the bits the state switches look at.
pub const TH_SCHED_STATE: u32 = TH_WAIT | TH_SUSP | TH_RUN | TH_UNINT;
/// `TH_SWAPPED`: the thread has no kernel stack.
pub const TH_SWAPPED: u32 = 0x0100;
/// `TH_SW_COMING_IN`: the thread waits for a kernel stack.
pub const TH_SW_COMING_IN: u32 = 0x0200;
/// `TH_SWAP_STATE`: the bits `thread_dispatch()` masks off.
pub const TH_SWAP_STATE: u32 = TH_SWAPPED | TH_SW_COMING_IN;

/// A `continuation_t` of <kern/sched_prim.h>, whose null value is
/// `thread_no_continuation`.
pub type Continuation = Option<unsafe extern "C" fn()>;

/// A `void (*)(thread_t)` stack continuation, the third argument of
/// `stack_attach()` of <i386/i386/pcb.h>.
pub type StackResume = Option<unsafe extern "C" fn(*mut Thread)>;

/// The bitfield word of `struct thread`, which C declares as `unsigned
/// state:16; unsigned wake_active:1; unsigned active:1`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateBits(u32);

impl StateBits {
    const STATE_MASK: u32 = 0xffff;
    const WAKE_ACTIVE: u32 = 1 << 16;
    const ACTIVE: u32 = 1 << 17;

    pub const fn state(self) -> u32 {
        self.0 & Self::STATE_MASK
    }

    /// The `wake_active` bit: someone is waiting for this thread to become
    /// suspended.
    pub const fn wake_active(self) -> bool {
        self.0 & Self::WAKE_ACTIVE != 0
    }

    pub const fn active(self) -> bool {
        self.0 & Self::ACTIVE != 0
    }

    /// Replaces the `state` half, leaving the flag bits alone; the C
    /// assignment to the 16-bit field.
    pub fn set_state(&mut self, state: u32) {
        self.0 = (self.0 & !Self::STATE_MASK) | (state & Self::STATE_MASK);
    }

    pub fn set_wake_active(&mut self, active: bool) {
        if active {
            self.0 |= Self::WAKE_ACTIVE;
        } else {
            self.0 &= !Self::WAKE_ACTIVE;
        }
    }

    /// Replaces the `active` bit: how alive the thread is.
    pub fn set_active(&mut self, active: bool) {
        if active {
            self.0 |= Self::ACTIVE;
        } else {
            self.0 &= !Self::ACTIVE;
        }
    }
}

/// The anonymous union of `struct thread` holding the bitfield word and
/// `event_key`.
#[repr(C)]
pub union StateEvent {
    state: StateBits,
    event_key: *mut c_void,
}

/// `struct ipc_kmsg_queue` of <ipc/ipc_kmsg_queue.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IpcKmsgQueue {
    /// `ikmq_base`: the first message, opaque while IPC stays in C.
    pub base: *mut c_void,
}

/// `mach_msg_size_t`/`struct ipc_kmsg *` union of `struct thread`.
#[repr(C)]
pub union ThreadData {
    /// `msize`: the maximum size of a received message.
    pub msize: c_uint,
    /// `kmsg`: the received message.
    pub kmsg: *mut c_void,
}

/// The `receive` arm of `thread.saved`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SavedReceive {
    /// `msg`: the user message header.
    pub msg: *mut c_void,
    pub option: c_int,
    pub rcv_size: c_uint,
    pub timeout: c_uint,
    /// `notify`: the notification port name.
    pub notify: c_uint,
    /// `object`: the object being received from.
    pub object: *mut c_void,
    pub mqueue: *mut c_void,
}

/// The `exception` arm of `thread.saved`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SavedException {
    pub port: *mut c_void,
    pub exc: c_int,
    pub code: c_int,
    pub subcode: c_long,
}

/// The `saved` union of `struct thread`: what the state selection keeps if the
/// thread's stack is discarded.
#[repr(C)]
pub union Saved {
    /// `receive`: a state selection in progress.
    pub receive: SavedReceive,
    /// `exception`: an exception in progress.
    pub exception: SavedException,
    /// `other`: a catch-all.
    pub other: *mut c_void,
}

/// `struct thread` of <kern/thread.h>.
#[repr(C)]
pub struct Thread {
    /// `links`: the run-queue or wait-queue links.
    pub links: QueueEntry,
    /// `runq`: the run queue the thread is on, or `RUN_QUEUE_NULL`.
    pub runq: *mut RunQueue,
    pub task: *mut c_void,
    pub thread_list: QueueEntry,
    /// `state`, `wake_active`, `active` and `event_key`.
    pub state_event: StateEvent,
    pub pset_threads: QueueEntry,
    /// `lock`: the thread lock, taken at splsched.
    pub lock: SimpleLock,
    pub ref_count: c_int,
    /// `pcb`: the machine-dependent process control block.
    pub pcb: *mut c_void,
    /// `kernel_stack`: accurate only when the thread is not swapped.
    pub kernel_stack: VmOffset,
    /// `stack_privilege`: the reserved kernel stack.
    pub stack_privilege: VmOffset,
    /// `swap_func`: where the thread starts after swap-in.
    pub swap_func: Continuation,
    pub wait_event: *mut c_void,
    /// `suspend_count`: internal use only.
    pub suspend_count: c_int,
    pub wait_result: c_int,
    /// `priority`: the base priority.
    pub priority: c_int,
    pub max_priority: c_int,
    /// `sched_pri`: the computed priority.
    pub sched_pri: c_int,
    /// `sched_data`: for use by the policy.
    pub sched_data: c_int,
    pub policy: c_int,
    /// `depress_priority`: the priority when depressed.
    pub depress_priority: c_int,
    /// `cpu_usage`: the decaying CPU usage, in percent.
    pub cpu_usage: c_uint,
    /// `sched_usage`: the load-weighted CPU usage.
    pub sched_usage: c_uint,
    /// `sched_stamp`: the last priority update time.
    pub sched_stamp: c_uint,
    /// `recover`: the page-fault recovery state.
    pub recover: VmOffset,
    /// `vm_privilege`: can the thread use reserved memory.
    pub vm_privilege: c_uint,
    /// `user_stop_count`: the outstanding stops.
    pub user_stop_count: c_int,
    /// `ith_next`: the IPC thread queue's next link.
    pub ith_next: *mut Thread,
    /// `ith_prev`: the IPC thread queue's previous link.
    pub ith_prev: *mut Thread,
    /// `ith_state`: the IPC thread queue state.
    pub ith_state: c_int,
    /// `data`: the received message or its maximum size.
    pub data: ThreadData,
    /// `ith_seqno`: the sequence number of the received message.
    pub ith_seqno: c_uint,
    /// `ith_messages`: messages being destroyed.
    pub ith_messages: IpcKmsgQueue,
    /// `ith_lock_data`: the IPC thread lock.
    pub ith_lock_data: SimpleLock,
    /// `ith_self`: the thread port, not a right.
    pub ith_self: *mut c_void,
    /// `ith_sself`: the thread port, a send right.
    pub ith_sself: *mut c_void,
    /// `ith_exception`: the exception port, a send right.
    pub ith_exception: *mut c_void,
    /// `ith_mig_reply`: the reply port for MIG.
    pub ith_mig_reply: c_uint,
    /// `ith_rpc_reply`: the reply port for kernel RPCs.
    pub ith_rpc_reply: *mut c_void,
    /// `saved`: the state saved when the stack is discarded.
    pub saved: Saved,
    pub user_timer: Timer,
    pub system_timer: Timer,
    pub user_timer_save: TimerSave,
    pub system_timer_save: TimerSave,
    /// `cpu_delta`: the CPU usage since the last update.
    pub cpu_delta: c_uint,
    /// `sched_delta`: the weighted CPU usage since the last update.
    pub sched_delta: c_uint,
    pub creation_time: TimeValue64,
    /// `timer`: the wait timeout.
    pub timer: Timeout,
    /// `depress_timer`: the priority-depression timeout.
    pub depress_timer: Timeout,
    /// `ast`: the pending ASTs; see <kern/ast.h>.
    pub ast: c_int,
    pub processor_set: *mut ProcessorSet,
    pub bound_processor: *mut Processor,
    /// `may_assign`: whether assignment may change (MACH_HOST).
    pub may_assign: c_int,
    /// `assign_active`: someone waits for `may_assign` (MACH_HOST).
    pub assign_active: c_int,
    /// `last_processor`: the processor the thread last ran on.
    pub last_processor: *mut Processor,
    pub name: [c_char; TASK_NAME_SIZE],
}

impl Thread {
    /// `thread_create()` copies this image and fills in the fields that depend
    /// on the run-time task and processor set.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: Every field accepts the all-zero image: the pointers are
        // null, the two unions begin as a null `event_key` and a null `other`,
        // and the locks' atomics start unlocked.
        let mut thread: Thread =
            unsafe { MaybeUninit::zeroed().assume_init() };

        thread.runq = RUN_QUEUE_NULL;
        thread.ref_count = 2;
        thread.set_state(TH_SUSP | TH_SWAPPED);
        thread.swap_func = Some(glue::thread_bootstrap_return);
        thread.max_priority = BASEPRI_SYSTEM;
        thread.policy = POLICY_TIMESHARE;
        thread.depress_priority = -1;
        thread.user_stop_count = 1;
        thread.may_assign = 1;
        thread
    }

    /// `thread_init()` in C.
    fn init() {
        // SAFETY: The boot caller runs this once, before any thread exists,
        // and the C globals below are exactly the ones the C `thread_init()`
        // initialized, in the same order.
        unsafe {
            glue::kmem_cache_init(
                &raw mut glue::thread_cache,
                c"thread".as_ptr(),
                size_of::<Thread>(),
                0,
                None,
                0,
            );
            glue::kmem_cache_init(
                &raw mut glue::thread_stack_cache,
                c"thread_stack".as_ptr(),
                KERNEL_STACK_SIZE,
                KERNEL_STACK_SIZE,
                None,
                0,
            );
            glue::thread_template = Self::new();
            queue_init(&raw mut glue::reaper_queue);
            let reaper_lock = &raw mut glue::reaper_lock;
            (*reaper_lock).init();
            let stack_lock = &raw mut glue::stack_lock_data;
            (*stack_lock).init();
            let usage_lock = &raw mut glue::stack_usage_lock;
            (*usage_lock).init();
            glue::pcb_module_init();
        }
    }

    /// `thread_timer_delta()` of <kern/sched.h>.
    ///
    /// # Safety
    ///
    /// `thread` must be live, and the caller must hold its lock at splsched,
    /// as `update_priority()` and the quantum expiry do.
    pub unsafe fn timer_delta(thread: *mut Thread) {
        // SAFETY: the caller's contract; the thread lock serializes the timer
        // records and the two accounting fields.
        let delta = unsafe {
            let system =
                (*thread).system_timer_save.delta(&(*thread).system_timer);
            let user = (*thread).user_timer_save.delta(&(*thread).user_timer);
            system.wrapping_add(user)
        };
        // SAFETY: as above; `processor_set` is the thread's own set, whose
        // `sched_load` the caller's lock covers.
        let load = unsafe { (*(*thread).processor_set).sched_load };
        // The C multiplies an `unsigned` by a `long` and stores the product
        // into an `unsigned`, so only the low 32 bits survive; the truncating
        // cast is exact for that.
        let scaled = delta.wrapping_mul(load as c_uint);
        // SAFETY: as above.
        unsafe {
            (*thread).cpu_delta = (*thread).cpu_delta.wrapping_add(delta);
            (*thread).sched_delta = (*thread).sched_delta.wrapping_add(scaled);
        }
    }

    pub fn state(&self) -> u32 {
        // SAFETY: the `state` member shares the low word with `event_key`, and
        // every bit pattern is a valid `StateBits`.
        unsafe { self.state_event.state.state() }
    }

    pub fn set_state(&mut self, state: u32) {
        // SAFETY: as `state()`, and the write only touches the low word.
        unsafe { self.state_event.state.set_state(state) };
    }

    pub fn wake_active(&self) -> bool {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.wake_active() }
    }

    pub fn set_wake_active(&mut self, active: bool) {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.set_wake_active(active) };
    }

    /// The `active` bit: whether the thread is alive.
    pub fn active(&self) -> bool {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.active() }
    }

    pub fn set_active(&mut self, active: bool) {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.set_active(active) };
    }

    /// The address of `event_key`, which is `TH_EV_WAKE_ACTIVE(t)` in C: the
    /// key a suspended thread's waker waits on.
    pub fn wake_active_event(&self) -> *mut c_void {
        (&raw const self.state_event.event_key)
            .cast::<c_void>()
            .cast_mut()
    }
}

impl Thread {
    /// `thread_reference()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread.
    pub unsafe fn reference(thread: *mut Thread) {
        let s = unsafe { glue::splsched() };
        // SAFETY: the caller promises a live thread; the lock protects
        // `ref_count`, and splsched keeps `thread_deallocate()` from reaching
        // zero under it.
        unsafe {
            (*thread).lock.lock();
            (*thread).ref_count = (*thread).ref_count.wrapping_add(1);
            (*thread).lock.unlock();
            glue::splx(s);
        }
    }

    /// `thread_hold()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread.
    pub unsafe fn hold(thread: *mut Thread) {
        let s = unsafe { glue::splsched() };
        // SAFETY: the caller promises a live thread; the lock protects
        // `suspend_count` and the state word.
        unsafe {
            (*thread).lock.lock();
            (*thread).suspend_count = (*thread).suspend_count.wrapping_add(1);
            let state = (*thread).state();
            (*thread).set_state(state | TH_SUSP);
            (*thread).lock.unlock();
            glue::splx(s);
        }
    }

    /// `thread_release()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread.
    pub unsafe fn release(thread: *mut Thread) {
        let s = unsafe { glue::splsched() };
        // SAFETY: the caller promises a live thread; the lock protects
        // `suspend_count` and the state word, and `thread_setrun()` expects
        // the thread lock held.
        unsafe {
            (*thread).lock.lock();
            (*thread).suspend_count = (*thread).suspend_count.wrapping_sub(1);
            if (*thread).suspend_count == 0 {
                let state = (*thread).state() & !(TH_SUSP | TH_HALTED);
                (*thread).set_state(state);
                if state & (TH_WAIT | TH_RUN) == 0 {
                    (*thread).set_state(state | TH_RUN);
                    thread_setrun(thread, 1);
                }
            }
            (*thread).lock.unlock();
            glue::splx(s);
        }
    }

    /// `thread_resume()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    pub unsafe fn resume(thread: *mut Thread) -> Result<(), KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        let s = unsafe { glue::splsched() };
        // SAFETY: the null check above and the caller's contract; the lock
        // protects both stop counts and the state word, and `thread_setrun()`
        // expects the thread lock held.
        unsafe {
            (*thread).lock.lock();
            let result = if (*thread).user_stop_count > 0 {
                (*thread).user_stop_count =
                    (*thread).user_stop_count.wrapping_sub(1);
                if (*thread).user_stop_count == 0 {
                    (*thread).suspend_count =
                        (*thread).suspend_count.wrapping_sub(1);
                    if (*thread).suspend_count == 0 {
                        let state = (*thread).state() & !(TH_SUSP | TH_HALTED);
                        (*thread).set_state(state);
                        if state & (TH_WAIT | TH_RUN) == 0 {
                            (*thread).set_state(state | TH_RUN);
                            thread_setrun(thread, 1);
                        }
                    }
                }
                Ok(())
            } else {
                Err(KernError::Failure)
            };
            (*thread).lock.unlock();
            glue::splx(s);
            result
        }
    }

    /// `thread_force_terminate()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread that is not the current thread;
    /// `task_terminate()` is the C caller.
    pub unsafe fn force_terminate(thread: *mut Thread) {
        // SAFETY: the caller's contract; `ipc_thread_disable()` takes the
        // thread's IPC lock itself.
        unsafe { ipc_thread_disable(thread) };

        // SAFETY: as above; `thread_freeze()` may block.
        unsafe { glue::thread_freeze(thread) };

        let default_pset =
            ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
        // SAFETY: as above; the set pointer is the thread's own and
        // `default_pset` is live for the life of the kernel.
        if unsafe { (*thread).processor_set } != default_pset {
            // SAFETY: as above; `thread_doassign()` may block.
            unsafe { glue::thread_doassign(thread, default_pset, 0) };
        }

        // SAFETY: as above; the lock protects the state word.
        let deallocate_here = unsafe {
            let s = glue::splsched();
            (*thread).lock.lock();
            let active = (*thread).active();
            (*thread).set_active(false);
            (*thread).lock.unlock();
            glue::splx(s);
            active
        };

        // SAFETY: as above; `thread_halt()` takes its own locks and may block.
        unsafe { glue::thread_halt(thread, 1) };
        // SAFETY: as above; `ipc_thread_terminate()` takes the thread's IPC
        // lock itself.
        unsafe { glue::ipc_thread_terminate(thread) };
        // SAFETY: as above; the Rust `unfreeze()` takes the thread lock.
        unsafe { Thread::unfreeze(thread) };

        if deallocate_here {
            // SAFETY: as above; the caller's active reference was the last
            // one, and `thread_deallocate()` may block.
            unsafe { glue::thread_deallocate(thread) };
        }
    }

    /// `thread_abort()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    pub unsafe fn abort(thread: *mut Thread) -> Result<(), KernError> {
        if thread.is_null() || thread == current_thread() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the check above and the caller's contract; the event count
        // takes the thread lock itself.
        unsafe { glue::evc_notify_abort(thread) };

        // SAFETY: as above; `thread_halt()` takes its own locks, may block,
        // and reports failure rather than halting.
        if unsafe { glue::thread_halt(thread, 0) } != 0 {
            return Err(KernError::Aborted);
        }

        // SAFETY: as above; the Rust `mach_msg_abort_rpc()` takes the thread's
        // IPC lock itself.
        unsafe { mach_msg_abort_rpc(thread) };

        // SAFETY: as above; `release()` takes the thread lock.
        unsafe { Thread::release(thread) };

        if unsafe { (*thread).depress_priority } != -1 {
            // SAFETY: as above; the Rust `thread_depress_abort()` takes the
            // thread lock itself.
            unsafe { thread_depress_abort(thread) };
        }

        Ok(())
    }

    /// `thread_start()` of kern/thread.c.
    pub fn start(&mut self, start: Continuation) {
        self.swap_func = start;
    }

    /// `thread_unfreeze()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread.
    pub unsafe fn unfreeze(thread: *mut Thread) {
        let s = unsafe { glue::splsched() };
        // SAFETY: the caller promises a live thread; the lock protects both
        // assignment fields.
        unsafe {
            (*thread).lock.lock();
            (*thread).may_assign = 1;
            if (*thread).assign_active != 0 {
                (*thread).assign_active = 0;
                thread_wakeup_prim(
                    ptr::addr_of_mut!((*thread).assign_active)
                        .cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            (*thread).lock.unlock();
            glue::splx(s);
        }
    }

    /// `thread_get_assignment()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    pub unsafe fn assignment(
        thread: *mut Thread,
    ) -> Result<*mut ProcessorSet, KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the check above and the caller's contract; the set pointer
        // is the thread's own assignment.
        let pset = unsafe { (*thread).processor_set };
        // SAFETY: as above; `pset_reference()` takes the set's reference lock.
        unsafe { pset_reference(pset) };
        Ok(pset)
    }
}

/// Compute the `sched_data` quantum a fixed-priority request yields: `data`
/// milliseconds, rounded up to whole `tick`s.
///
/// # Panics
///
/// Panics if the kernel's `tick` global is zero: the conversion divides by it.
fn fixedpri_quantum(data: c_int) -> c_int {
    // SAFETY: `tick` is the `int tick` of <kern/mach_clock.h>, initialized
    // before any thread can call a policy setter and never written after.
    let tick = unsafe { glue::tick };
    let temp = data.wrapping_mul(1000);
    let temp = if temp % tick != 0 {
        temp.wrapping_add(tick)
    } else {
        temp
    };
    temp / tick
}

impl Thread {
    /// `thread_get_state()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread; `old_state` must be
    /// writable for the words `*old_state_count` names, and `old_state_count`
    /// must be valid for a read and a write.
    pub unsafe fn get_status(
        thread: *mut Thread,
        flavor: c_int,
        old_state: *mut c_uint,
        old_state_count: *mut c_uint,
    ) -> Result<(), KernError> {
        if flavor == I386_DEBUG_STATE && thread == current_thread() {
            // SAFETY: the caller promises the state buffers, and `thread` is
            // the current thread.
            let code = unsafe {
                glue::thread_getstatus(
                    thread,
                    flavor,
                    old_state,
                    old_state_count,
                )
            };
            return kern_error(code);
        }

        if thread.is_null() || thread == current_thread() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the checks above and the caller's contract; the suspend and
        // the wait take the thread lock themselves.
        unsafe {
            Thread::hold(thread);
            let _ = glue::thread_dowait(thread, 1);
        }

        // SAFETY: as above; `thread` is live and held.
        let code = unsafe {
            glue::thread_getstatus(thread, flavor, old_state, old_state_count)
        };

        // SAFETY: as above; `thread` is the one just held.
        unsafe { Thread::release(thread) };

        kern_error(code)
    }

    /// `thread_set_state()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and `new_state` must
    /// be readable for `new_state_count` words.
    pub unsafe fn set_status(
        thread: *mut Thread,
        flavor: c_int,
        new_state: *mut c_uint,
        new_state_count: c_uint,
    ) -> Result<(), KernError> {
        if thread == current_thread()
            && (flavor == I386_DEBUG_STATE || flavor == I386_FSGS_BASE_STATE)
        {
            // SAFETY: the caller promises the state buffer, and `thread` is
            // the current thread.
            let code = unsafe {
                glue::thread_setstatus(
                    thread,
                    flavor,
                    new_state,
                    new_state_count,
                )
            };
            return kern_error(code);
        }

        if thread.is_null() || thread == current_thread() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the checks above and the caller's contract; the suspend and
        // the wait take the thread lock themselves.
        unsafe {
            Thread::hold(thread);
            let _ = glue::thread_dowait(thread, 1);
        }

        // SAFETY: as above; `thread` is live and held.
        let code = unsafe {
            glue::thread_setstatus(thread, flavor, new_state, new_state_count)
        };

        // SAFETY: as above; `thread` is the one just held.
        unsafe { Thread::release(thread) };

        kern_error(code)
    }

    /// `thread_priority()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    pub unsafe fn priority(
        thread: *mut Thread,
        priority: c_int,
        set_max: bool,
    ) -> Result<(), KernError> {
        if thread.is_null() || invalid_pri(priority) {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // thread lock is taken under it.
        let s = unsafe { glue::splsched() };
        // SAFETY: the checks above and the caller's contract; the thread lock
        // protects every field below.
        let result = unsafe {
            (*thread).lock.lock();
            let result = if priority < (*thread).max_priority {
                Err(KernError::Failure)
            } else {
                if (*thread).depress_priority >= 0 {
                    (*thread).depress_priority = priority;
                } else {
                    (*thread).priority = priority;
                    compute_priority(thread, 1);
                }
                if set_max {
                    (*thread).max_priority = priority;
                }
                Ok(())
            };
            (*thread).lock.unlock();
            result
        };
        // SAFETY: `s` is the level `splsched()` returned.
        unsafe { glue::splx(s) };

        result
    }

    /// `thread_set_own_priority()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must be the current thread and hold no thread lock:
    /// `splsched()` raises the level and this takes the current thread's lock.
    pub unsafe fn set_own_priority(priority: c_int) {
        let thread = current_thread();
        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // thread lock is taken under it.
        let s = unsafe { glue::splsched() };
        // SAFETY: `thread` is the live current thread, and the lock protects
        // the priority fields.
        unsafe {
            (*thread).lock.lock();
            if priority < (*thread).max_priority {
                (*thread).max_priority = priority;
            }
            (*thread).priority = priority;
            compute_priority(thread, 1);
            (*thread).lock.unlock();
            // SAFETY: `s` is the level `splsched()` returned.
            glue::splx(s);
        }
    }

    /// `thread_max_priority()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` and `pset` must be null or point at live objects.
    pub unsafe fn max_priority(
        thread: *mut Thread,
        pset: *mut ProcessorSet,
        max_priority: c_int,
    ) -> Result<(), KernError> {
        if thread.is_null() || pset.is_null() || invalid_pri(max_priority) {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // thread lock is taken under it.
        let s = unsafe { glue::splsched() };
        // SAFETY: the checks above and the caller's contract; the thread lock
        // protects every field below.
        let result = unsafe {
            (*thread).lock.lock();
            let result = if pset != (*thread).processor_set {
                Err(KernError::Failure)
            } else {
                (*thread).max_priority = max_priority;
                if max_priority > (*thread).priority {
                    (*thread).priority = max_priority;
                    compute_priority(thread, 1);
                } else if (*thread).depress_priority >= 0
                    && max_priority > (*thread).depress_priority
                {
                    (*thread).depress_priority = max_priority;
                }
                Ok(())
            };
            (*thread).lock.unlock();
            result
        };
        // SAFETY: `s` is the level `splsched()` returned.
        unsafe { glue::splx(s) };

        result
    }

    /// `thread_policy()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    ///
    /// # Panics
    ///
    /// Panics through [`fixedpri_quantum()`] if the kernel's `tick` global is
    /// zero, which the C divides by in the same case.
    pub unsafe fn policy(
        thread: *mut Thread,
        policy: c_int,
        data: c_int,
    ) -> Result<(), KernError> {
        if thread.is_null() || invalid_policy(policy) {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // thread lock is taken under it.
        let s = unsafe { glue::splsched() };
        // SAFETY: the checks above and the caller's contract; the thread lock
        // protects every field below.
        let result = unsafe {
            (*thread).lock.lock();
            let result = if policy == (*thread).policy {
                if policy == POLICY_FIXEDPRI {
                    (*thread).sched_data = fixedpri_quantum(data);
                }
                Ok(())
            } else {
                let pset = (*thread).processor_set;
                if ((*pset).policies & policy) == 0 {
                    Err(KernError::Failure)
                } else {
                    (*thread).policy = policy;
                    if policy == POLICY_FIXEDPRI {
                        (*thread).sched_data = fixedpri_quantum(data);
                    }
                    compute_priority(thread, 1);
                    Ok(())
                }
            };
            (*thread).lock.unlock();
            result
        };
        // SAFETY: `s` is the level `splsched()` returned.
        unsafe { glue::splx(s) };

        result
    }

    /// `thread_wire()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread.
    pub unsafe fn wire(
        thread: *mut Thread,
        wired: bool,
    ) -> Result<(), KernError> {
        if thread.is_null() || thread != current_thread() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // thread lock is taken under it.
        let s = unsafe { glue::splsched() };
        // SAFETY: the checks above and the caller's contract; the thread lock
        // protects the privilege fields, and the Rust `stack_privilege()`
        // compares the thread with the current one, which the check above
        // already found equal.
        unsafe {
            (*thread).lock.lock();
            if wired {
                (*thread).vm_privilege = 1;
                stack_privilege(thread);
            } else {
                (*thread).vm_privilege = 0;
                (*thread).stack_privilege = 0;
            }
            (*thread).lock.unlock();
            glue::splx(s);
        }

        Ok(())
    }

    /// `thread_set_name()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and `name` must be
    /// readable up to `TASK_NAME_SIZE - 1` bytes or a NUL inside them.
    pub unsafe fn set_name(
        thread: *mut Thread,
        name: *const c_char,
    ) -> Result<(), KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the caller's contract; `name` is the fixed-size array in the
        // thread record, one byte longer than the copy.
        unsafe {
            strncpy((*thread).name.as_mut_ptr(), name, TASK_NAME_SIZE - 1);
            (*thread).name[TASK_NAME_SIZE - 1] = 0;
        }
        Ok(())
    }

    /// `thread_get_name()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and `name` must be
    /// writable for `TASK_NAME_SIZE` bytes.
    pub unsafe fn get_name(
        thread: *mut Thread,
        name: *mut c_char,
    ) -> Result<(), KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the caller promises a live thread and a buffer of
        // `TASK_NAME_SIZE` bytes; the C copied the whole array and `strncpy()`
        // NUL-pads it.
        unsafe {
            strncpy(name, (*thread).name.as_ptr(), TASK_NAME_SIZE);
        }
        Ok(())
    }
}

impl Thread {
    /// `stack_alloc_try()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must hold this thread's lock at splsched, as the swap path
    /// does, and `resume` must be a stack continuation.
    #[must_use]
    unsafe fn stack_alloc_try(&mut self, resume: StackResume) -> bool {
        let lock = &raw mut glue::stack_lock_data;
        // SAFETY: the caller is at splsched with the thread locked, so no
        // other path can hold `stack_lock_data`; `thread_init()` built the
        // lock, and every entry on the list is a cache object `stack_free()`
        // pushed.
        let stack = unsafe {
            (*lock).lock();
            let stack = stack_free_pop();
            (*lock).unlock();
            stack
        };

        let stack = if stack != 0 {
            stack
        } else {
            self.stack_privilege
        };

        if stack == 0 {
            return false;
        }

        // SAFETY: `stack` is a whole cache object, or this thread's private
        // one, and the C `stack_attach()` writes only this thread's fields and
        // the stack's first frame.
        unsafe { glue::stack_attach(ptr::from_mut(self), stack, resume) };

        true
    }

    /// `stack_alloc()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must hold no spin lock, because the cache allocation may
    /// block, and `resume` must be a stack continuation.
    unsafe fn stack_alloc(&mut self, resume: StackResume) {
        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // free list is touched only between it and the matching `splx()`.
        let s = unsafe { glue::splsched() };
        let lock = &raw mut glue::stack_lock_data;
        // SAFETY: as in `stack_alloc_try()`.
        let stack = unsafe {
            (*lock).lock();
            let stack = stack_free_pop();
            (*lock).unlock();
            stack
        };
        // SAFETY: `s` is the level `splsched()` returned.
        unsafe { glue::splx(s) };

        let stack = if stack == 0 {
            // SAFETY: `thread_stack_cache` is the cache `thread_init()` built,
            // and the allocation may block: this function's contract is that
            // no lock is held.
            let fresh = unsafe {
                glue::kmem_cache_alloc(&raw mut glue::thread_stack_cache)
            };
            // SAFETY: `stack_init()` marks the fresh object when the usage
            // check is on, exactly as the C called it.
            unsafe { stack_init(fresh) };
            fresh
        } else {
            stack
        };

        // SAFETY: as in `stack_alloc_try()`.
        unsafe { glue::stack_attach(ptr::from_mut(self), stack, resume) };
    }

    /// `stack_free()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must hold this thread's lock at splsched, and a stack must
    /// be attached: the C walks the returned stack's link word.
    unsafe fn stack_free(&mut self) {
        let privilege = self.stack_privilege;
        // SAFETY: the caller promises an attached stack, and `stack_detach()`
        // takes it off this thread and returns it.
        let stack = unsafe {
            crate::arch::i386::pcb::stack_detach(ptr::from_mut(self))
        };

        if stack != privilege {
            let lock = &raw mut glue::stack_lock_data;
            // SAFETY: the caller is at splsched with the thread lock held; the
            // detached stack is on no list.
            unsafe {
                (*lock).lock();
                stack_free_push(stack);
                (*lock).unlock();
            }
        }
    }

    /// `stack_collect()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must hold no spin lock.
    unsafe fn stack_collect() {
        let lock = &raw mut glue::stack_lock_data;
        // SAFETY: `splsched()` is the real asm routine; at that level the lock
        // serializes the list, and `stack_finalize()` and `kmem_cache_free()`
        // run with both the lock and the raised level released, so they may
        // block.
        unsafe {
            let mut s = glue::splsched();
            (*lock).lock();
            while glue::stack_free_count > glue::stack_free_limit {
                let stack = glue::stack_free_list;
                glue::stack_free_list = stack_next(stack);
                glue::stack_free_count -= 1;
                (*lock).unlock();
                glue::splx(s);

                glue::stack_finalize(stack);
                glue::kmem_cache_free(
                    &raw mut glue::thread_stack_cache,
                    stack,
                );

                s = glue::splsched();
                (*lock).lock();
            }
            (*lock).unlock();
            glue::splx(s);
        }
    }

    /// `stack_privilege()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// The caller must be running on `self`; the C halts the kernel otherwise.
    unsafe fn stack_privilege(&mut self) {
        if current_thread() != ptr::from_mut(self) {
            // SAFETY: `Panic` halts the kernel and never returns; the
            // arguments are the C `panic()` macro's.
            unsafe {
                glue::Panic(
                    c"kern/thread.c".as_ptr(),
                    line!() as c_int,
                    c"stack_privilege".as_ptr(),
                    c"stack_privilege".as_ptr(),
                )
            }
        }

        if self.stack_privilege == 0 {
            self.stack_privilege = current_stack();
        }
    }
}

impl Default for Thread {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(size_of::<Thread>() == THREAD_SIZE);
const _: () = assert!(align_of::<Thread>() == align_of::<*mut c_void>());

const _: () = assert!(size_of::<StateBits>() == size_of::<u32>());
const _: () = assert!(
    size_of::<StateEvent>()
        == if size_of::<*mut c_void>() > size_of::<u32>() {
            size_of::<*mut c_void>()
        } else {
            size_of::<u32>()
        }
);

const _: () = assert!(KERNEL_STACK_SIZE.is_multiple_of(size_of::<VmOffset>()));

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(Thread, links) == 0);
    assert!(offset_of!(Thread, runq) == 16);
    assert!(offset_of!(Thread, state_event) == 48);
    assert!(offset_of!(Thread, pset_threads) == 56);
    assert!(offset_of!(Thread, lock) == 72);
    assert!(offset_of!(Thread, swap_func) == 104);
    assert!(offset_of!(Thread, wait_event) == 112);
    assert!(offset_of!(Thread, wait_result) == 124);
    assert!(offset_of!(Thread, sched_pri) == 136);
    assert!(offset_of!(Thread, sched_stamp) == 160);
    assert!(offset_of!(Thread, timer) == 392);
    assert!(offset_of!(Thread, depress_timer) == 440);
    assert!(offset_of!(Thread, processor_set) == 496);
    assert!(offset_of!(Thread, bound_processor) == 504);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(Thread, links) == 0);
    assert!(offset_of!(Thread, runq) == 8);
    assert!(offset_of!(Thread, state_event) == 24);
    assert!(offset_of!(Thread, pset_threads) == 28);
    assert!(offset_of!(Thread, lock) == 36);
    assert!(offset_of!(Thread, swap_func) == 56);
    assert!(offset_of!(Thread, wait_event) == 60);
    assert!(offset_of!(Thread, wait_result) == 68);
    assert!(offset_of!(Thread, sched_pri) == 80);
    assert!(offset_of!(Thread, sched_stamp) == 104);
    assert!(offset_of!(Thread, timer) == 268);
    assert!(offset_of!(Thread, depress_timer) == 292);
    assert!(offset_of!(Thread, processor_set) == 320);
    assert!(offset_of!(Thread, bound_processor) == 324);
};

const _: () = assert!(size_of::<IpcKmsgQueue>() == size_of::<*mut c_void>());
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<SavedReceive>() == 40);
    assert!(size_of::<SavedException>() == 24);
    assert!(size_of::<Saved>() == 40);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<SavedReceive>() == 28);
    assert!(size_of::<SavedException>() == 16);
    assert!(size_of::<Saved>() == 28);
};
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ThreadData>() == 8);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<ThreadData>() == 4);

/// The free-list link word of a stack object: `stack_next()` of kern/thread.c,
/// a `vm_offset_t` in the last word of the `KERNEL_STACK_SIZE` region.
///
/// # Safety
///
/// `stack` must be the base address of a live `thread_stack_cache` object.
unsafe fn stack_next(stack: VmOffset) -> VmOffset {
    // SAFETY: the caller promises a whole cache object, so its last word is
    // readable; `with_exposed_provenance()` rebuilds the pointer from the
    // address the cache handed out.
    unsafe {
        ptr::with_exposed_provenance::<VmOffset>(stack)
            .add(KERNEL_STACK_SIZE / size_of::<VmOffset>() - 1)
            .read()
    }
}

/// Store `next` as the free-list link of a stack object, the C's
/// `stack_next(stack) = next`.
///
/// # Safety
///
/// `stack` must be the base address of a live `thread_stack_cache` object.
unsafe fn set_stack_next(stack: VmOffset, next: VmOffset) {
    // SAFETY: as `stack_next()`; the write stays inside the object.
    unsafe {
        ptr::with_exposed_provenance_mut::<VmOffset>(stack)
            .add(KERNEL_STACK_SIZE / size_of::<VmOffset>() - 1)
            .write(next);
    }
}

/// Remove the first stack from the free list, or return zero when the list is
/// empty.
///
/// # Safety
///
/// The caller must hold `stack_lock_data` at splsched, and every entry on the
/// list must be a live cache object.
unsafe fn stack_free_pop() -> VmOffset {
    // SAFETY: the caller holds the lock, so the head and the count are stable;
    // `stack_free_push()` built every link.
    unsafe {
        let stack = glue::stack_free_list;
        if stack != 0 {
            glue::stack_free_list = stack_next(stack);
            glue::stack_free_count -= 1;
        }
        stack
    }
}

/// Return `stack` to the free list.
///
/// # Safety
///
/// The caller must hold `stack_lock_data` at splsched, and `stack` must be a
/// live cache object on no list.
unsafe fn stack_free_push(stack: VmOffset) {
    // SAFETY: as `stack_free_pop()`; `stack` supplies a free link word.
    unsafe {
        set_stack_next(stack, glue::stack_free_list);
        glue::stack_free_list = stack;
        glue::stack_free_count += 1;
    }
}

/// `thread_init()` of kern/thread.c.
///
/// # Safety
///
/// Must be called once during boot, before the first thread is created;
/// `setup_main()` is the only caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_init() {
    Thread::init();
}

/// `thread_timer_delta()` of kern/sched.h, which used to be a macro.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds, at splsched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_timer_delta(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::timer_delta(thread) };
}

/// `thread_assign_default()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or a live thread the caller holds an extra reference
/// to, and the caller must hold no locks: `thread_assign()` may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_assign_default(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract is `thread_assign()`'s own, and the
    // default set is a live `struct processor_set` for the life of the kernel.
    let assigned = unsafe {
        glue::thread_assign(
            thread,
            ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>(),
        )
    };

    match kern_error(assigned) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `stack_alloc_try()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread whose lock the caller holds at
/// splsched, and `resume` must be a stack continuation.
#[unsafe(no_mangle)]
#[must_use]
pub unsafe extern "C" fn stack_alloc_try(
    thread: *mut Thread,
    resume: StackResume,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { (*thread).stack_alloc_try(resume) })
}

/// `stack_alloc()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread, the caller must hold no spin lock
/// because the allocation may block, and `resume` must be a stack
/// continuation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_alloc(
    thread: *mut Thread,
    resume: StackResume,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*thread).stack_alloc(resume) };
    0
}

/// `stack_free()` of kern/sched_prim.h.
///
/// # Safety
///
/// `thread` must point at a live thread whose lock the caller holds at
/// splsched, with a stack attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_free(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).stack_free() };
}

/// `stack_collect()` of kern/thread.h.
#[unsafe(no_mangle)]
pub extern "C" fn stack_collect() {
    // SAFETY: `stack_collect()` takes its own splsched level and drops it
    // around each release; the caller holds no lock.
    unsafe { Thread::stack_collect() };
}

/// `stack_privilege()` of kern/thread.h.
///
/// # Safety
///
/// `thread` must be the current thread; the C halts the kernel otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_privilege(thread: *mut Thread) {
    // SAFETY: the caller's contract; the core compares `thread` with
    // `current_thread()` and halts when they differ.
    unsafe { (*thread).stack_privilege() };
}

/// `stack_init()` of kern/thread.c.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel stack
/// object: the cache allocator's, or one the machine-dependent code is about
/// to install.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stack_init(stack: VmOffset) {
    // SAFETY: `stack_check_usage` is the `boolean_t` global kern/thread.c
    // defines and a debugger may set; the read is a race-free load of an
    // initialized `int`.
    if unsafe { glue::stack_check_usage } == 0 {
        return;
    }

    // SAFETY: the caller promises a whole stack object, and
    // `KERNEL_STACK_SIZE` is a whole number of `unsigned int`s, so the slice
    // covers exactly the object.
    let words = unsafe {
        core::slice::from_raw_parts_mut(
            ptr::with_exposed_provenance_mut::<u32>(stack),
            KERNEL_STACK_SIZE / size_of::<u32>(),
        )
    };
    for word in words {
        *word = STACK_MARKER;
    }
}

/// `thread_reference()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_reference(thread: *mut Thread) {
    let Some(thread) = ptr::NonNull::new(thread) else {
        return;
    };
    // SAFETY: the caller's contract.
    unsafe { Thread::reference(thread.as_ptr()) };
}

/// `thread_force_terminate()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread that is not the current thread, as
/// `task_terminate()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_force_terminate(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::force_terminate(thread) };
}

/// `thread_hold()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_hold(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::hold(thread) };
}

/// `thread_release()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_release(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::release(thread) };
}

/// `thread_resume()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_resume(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `resume()` checks the null the C checked.
    match unsafe { Thread::resume(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_abort()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread; the routine takes the
/// thread's locks itself and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_abort(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; `abort()` checks the null and
    // current-thread arguments the C checked.
    match unsafe { Thread::abort(thread) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_start()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread; the C stores `start` in its
/// `swap_func` field.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_start(
    thread: *mut Thread,
    start: Continuation,
) {
    // SAFETY: the caller's contract.
    unsafe { (*thread).start(start) };
}

/// `thread_unfreeze()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_unfreeze(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { Thread::unfreeze(thread) };
}

/// `thread_get_assignment()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `pset` must be valid
/// for a write; the MIG server passes the address of its own
/// `processor_set_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_assignment(
    thread: *mut Thread,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    // SAFETY: the caller's contract; `assignment()` checks the null the C
    // checked.
    match unsafe { Thread::assignment(thread) } {
        Ok(assignment) => {
            // SAFETY: the caller promises the out-parameter.
            unsafe { *pset = assignment };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `thread_get_state()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread; `old_state` must be
/// writable for the words `*old_state_count` names, and `old_state_count` must
/// be valid for a read and a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_state(
    thread: *mut Thread,
    flavor: c_int,
    old_state: *mut c_uint,
    old_state_count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract; `get_status()` checks the null and
    // current-thread arguments the C checked.
    match unsafe {
        Thread::get_status(thread, flavor, old_state, old_state_count)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_set_state()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `new_state` must be
/// readable for `new_state_count` words.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_state(
    thread: *mut Thread,
    flavor: c_int,
    new_state: *mut c_uint,
    new_state_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract; `set_status()` checks the null and
    // current-thread arguments the C checked.
    match unsafe {
        Thread::set_status(thread, flavor, new_state, new_state_count)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_priority()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_priority(
    thread: *mut Thread,
    priority: c_int,
    set_max: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `set_max` is the C boolean.
    match unsafe { Thread::priority(thread, priority, set_max != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_set_own_priority()` of kern/thread.c.
///
/// # Safety
///
/// The caller must be the current thread and hold no thread lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_own_priority(priority: c_int) {
    // SAFETY: the caller's contract.
    unsafe { Thread::set_own_priority(priority) };
}

/// `thread_max_priority()` of kern/thread.c.
///
/// # Safety
///
/// `thread` and `pset` must be null or point at live objects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_max_priority(
    thread: *mut Thread,
    pset: *mut ProcessorSet,
    max_priority: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `max_priority()` checks the null
    // arguments the C checked.
    match unsafe { Thread::max_priority(thread, pset, max_priority) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_policy()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_policy(
    thread: *mut Thread,
    policy: c_int,
    data: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `policy()` checks the null argument the C
    // checked.
    match unsafe { Thread::policy(thread, policy, data) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_wire()` of kern/thread.c.
///
/// # Safety
///
/// `host` must be null or a live `struct host`; `thread` must be null or point
/// at a live thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_wire(
    host: *mut c_void,
    thread: *mut Thread,
    wired: c_int,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    // SAFETY: the caller's contract; `wire()` checks the null and
    // current-thread arguments the C checked.
    match unsafe { Thread::wire(thread, wired != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_stats()` of kern/thread.c.
///
/// # Safety
///
/// Reached from the debugger; the queue walk takes no locks, exactly as the C
/// did, so every link must be a live thread for the walk.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_stats() {
    let pset = ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
    // SAFETY: `default_pset` is the live global set the bootstrap built, and
    // the walk reads its thread list as the C did.
    let (total, rpcreply) = unsafe {
        let list = ptr::addr_of_mut!((*pset).threads);
        let mut total = 0;
        let mut rpcreply = 0;
        let mut thread = queue_first(list).cast::<Thread>();
        while queue_end(list, thread.cast::<QueueEntry>()) == 0 {
            total += 1;
            if !(*thread).ith_rpc_reply.is_null() {
                rpcreply += 1;
            }
            thread =
                queue_next(&raw mut (*thread).pset_threads).cast::<Thread>();
        }
        (total, rpcreply)
    };

    // SAFETY: `printf` is the C variadic; each format string takes the one
    // `int` argument passed, as the C did.
    unsafe {
        glue::printf(c"%d total threads.\n".as_ptr(), total);
        glue::printf(c"%d using rpc_reply.\n".as_ptr(), rpcreply);
    }
}

/// `thread_set_name()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `name` must be
/// readable up to `TASK_NAME_SIZE - 1` bytes or a NUL inside them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_name(
    thread: *mut Thread,
    name: *const c_char,
) -> c_int {
    // SAFETY: the caller's contract; `set_name()` checks the null argument the
    // C checked.
    match unsafe { Thread::set_name(thread, name) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_get_name()` of kern/thread.c.
///
/// # Safety
///
/// `thread` must be null or point at a live thread, and `name` must be
/// writable for `TASK_NAME_SIZE` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_name(
    thread: *mut Thread,
    name: *mut c_char,
) -> c_int {
    // SAFETY: the caller's contract; `get_name()` checks the null argument the
    // C checked.
    match unsafe { Thread::get_name(thread, name) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}
