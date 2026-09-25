// SPDX-License-Identifier: CMU-Mach
// Derived from kern/thread.h:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Derived from kern/thread.c:
//   Copyright (c) 1994-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread module's cores, which `kern/thread.c` used to define and
//! `kern/thread.h` declares, and the `struct thread` mirror.

use crate::arch::i386::ast_check::cause_ast_check;
use crate::arch::i386::percpu::{
    cpu_number, current_stack, current_thread, percpu_at,
};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::KERNEL_STACK_SIZE;
use crate::glue;
use crate::glue::time_value::{RpcTimeValue, TimeValue, TimeValue64};
use crate::ipc::IpcSpace;
use crate::ipc::mach_port;
use crate::kern::ast::{AST_BLOCK, AST_HALT, AST_TERMINATE, ast_on};
use crate::kern::ipc_mig::abort_rpc;
use crate::kern::ipc_tt::{
    ipc_thread_disable, ipc_thread_enable, ipc_thread_init,
    ipc_thread_terminate,
};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::{Timeout, read_time_stamp, reset_timeout_check};
use crate::kern::policy::{POLICY_FIXEDPRI, POLICY_TIMESHARE, invalid_policy};
use crate::kern::processor::{
    Processor, ProcessorSet, pset_deallocate, pset_reference,
};
use crate::kern::queue::{
    QueueEntry, dequeue_head, enqueue_tail, queue_end, queue_enter_tail,
    queue_first, queue_init, queue_next, queue_remove_generic,
};
use crate::kern::sched::{
    BASEPRI_SYSTEM, RUN_QUEUE_NULL, RunQueue, SCHED_SCALE, invalid_pri,
};
use crate::kern::sched_prim::{
    TH_RUN_SUSP, TH_RUN_SUSP_UNINT, TH_RUN_WAIT_SUSP, TH_RUN_WAIT_SUSP_UNINT,
    TH_WAIT_SUSP, TH_WAIT_SUSP_UNINT, THREAD_AWAKENED, THREAD_INTERRUPTED,
    assert_wait, clear_wait, compute_priority, thread_setrun, thread_sleep,
    thread_timeout_setup, thread_wakeup_prim,
};
use crate::kern::slab::{CacheInitFlags, KmemCache, kalloc, kfree};
use crate::kern::smp::smp_get_numcpus;
use crate::kern::syscall_subr::thread_depress_abort;
use crate::kern::task::{Task, add_time64, current_task, kernel_task};
use crate::kern::timer::{TIMER_RATE, Timer, TimerSave, thread_read_times};
use crate::kern::types::KernError;
use crate::utils::string::strncpy;
use crate::vm::vm_map::{VmMap, round_page};
use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::mem::{MaybeUninit, offset_of};
use core::ptr::{self, NonNull, with_exposed_provenance_mut};

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
    pub task: *mut Task,
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
    ///
    /// # Safety
    ///
    /// Runs once, from the boot sequence, before any thread exists.
    pub(crate) unsafe fn init() {
        // SAFETY: The boot caller runs this once, before any thread exists,
        // and the globals below are exactly the ones the C `thread_init()`
        // initialized, in the same order.
        unsafe {
            (*ptr::addr_of_mut!(THREAD_CACHE)).init(
                b"thread",
                size_of::<Thread>(),
                0,
                None,
                CacheInitFlags::EMPTY,
            );
            (*ptr::addr_of_mut!(THREAD_STACK_CACHE)).init(
                b"thread_stack",
                KERNEL_STACK_SIZE,
                KERNEL_STACK_SIZE,
                None,
                CacheInitFlags::EMPTY,
            );
            THREAD_TEMPLATE = Self::new();
            queue_init(&raw mut REAPER_QUEUE);
            let reaper_lock = &raw mut REAPER_LOCK;
            (*reaper_lock).init();
            let stack_lock = &raw mut STACK_LOCK_DATA;
            (*stack_lock).init();
            let usage_lock = &raw mut STACK_USAGE_LOCK;
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

    /// The address one `event_key` on, which is `TH_EV_STATE(t)` in C: the
    /// key a thread waiting for its state to become interruptible uses.
    pub fn state_event(&self) -> *mut c_void {
        let event = (&raw const self.state_event.event_key).cast_mut();
        // SAFETY: the union holds one pointer, so one element past
        // `event_key` stays inside `state_event`; the C's `&event_key + 1`.
        unsafe { event.add(1) }.cast::<c_void>()
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

        // SAFETY: as above; `Thread::freeze()` may block.
        unsafe { Thread::freeze(thread) };

        let default_pset = default_pset();
        // SAFETY: as above; the set pointer is the thread's own and
        // `default_pset` is live for the life of the kernel.
        if unsafe { (*thread).processor_set } != default_pset {
            // SAFETY: as above; `Thread::doassign()` may block.
            unsafe { Thread::doassign(thread, default_pset, false) };
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

        // SAFETY: as above; `Thread::halt()` takes its own locks and may
        // block.
        let _ = unsafe { Thread::halt(thread, true) };
        // SAFETY: as above; `ipc_thread_terminate()` takes the thread's IPC
        // lock itself.
        unsafe { ipc_thread_terminate(thread) };
        // SAFETY: as above; the Rust `unfreeze()` takes the thread lock.
        unsafe { Thread::unfreeze(thread) };

        if deallocate_here {
            // SAFETY: as above; the caller's active reference was the last
            // one, and `Thread::deallocate()` may block.
            unsafe { Thread::deallocate(thread) };
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

        // SAFETY: as above; `Thread::halt()` takes its own locks, may block,
        // and reports failure rather than halting.
        if unsafe { Thread::halt(thread, false) }.is_err() {
            return Err(KernError::Aborted);
        }

        // SAFETY: as above; the Rust `abort_rpc()` takes the thread's
        // IPC lock itself.
        unsafe { abort_rpc(thread) };

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
            let _ = Thread::dowait(thread, true);
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
            let _ = Thread::dowait(thread, true);
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
                (*thread).stack_privilege();
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
    pub(crate) unsafe fn stack_alloc_try(
        &mut self,
        resume: StackResume,
    ) -> bool {
        let lock = &raw mut STACK_LOCK_DATA;
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
    pub(crate) unsafe fn stack_alloc(&mut self, resume: StackResume) {
        // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
        // free list is touched only between it and the matching `splx()`.
        let s = unsafe { glue::splsched() };
        let lock = &raw mut STACK_LOCK_DATA;
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
            let fresh =
                unsafe { (*ptr::addr_of_mut!(THREAD_STACK_CACHE)).alloc() };
            let fresh = fresh.map_or(0, |buf| buf.as_ptr().addr());
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
    pub(crate) unsafe fn stack_free(&mut self) {
        let privilege = self.stack_privilege;
        // SAFETY: the caller promises an attached stack, and `stack_detach()`
        // takes it off this thread and returns it.
        let stack = unsafe {
            crate::arch::i386::pcb::stack_detach(ptr::from_mut(self))
        };

        if stack != privilege {
            let lock = &raw mut STACK_LOCK_DATA;
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
    pub(crate) unsafe fn stack_collect() {
        let lock = &raw mut STACK_LOCK_DATA;
        // SAFETY: `splsched()` is the real asm routine; at that level the lock
        // serializes the list, and `stack_finalize()` and `kmem_cache_free()`
        // run with both the lock and the raised level released, so they may
        // block.
        unsafe {
            let mut s = glue::splsched();
            (*lock).lock();
            while STACK_FREE_COUNT > STACK_FREE_LIMIT {
                let stack = STACK_FREE_LIST;
                STACK_FREE_LIST = stack_next(stack);
                STACK_FREE_COUNT -= 1;
                (*lock).unlock();
                glue::splx(s);

                stack_finalize(stack);
                // SAFETY: `thread_stack_cache` is the cache the stack came
                // from, and nothing references it after the finalize.
                if let Some(stack) =
                    NonNull::new(with_exposed_provenance_mut::<u8>(stack))
                {
                    (*ptr::addr_of_mut!(THREAD_STACK_CACHE)).free(stack);
                }

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
    pub(crate) unsafe fn stack_privilege(&mut self) {
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
        let stack = STACK_FREE_LIST;
        if stack != 0 {
            STACK_FREE_LIST = stack_next(stack);
            STACK_FREE_COUNT -= 1;
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
        set_stack_next(stack, STACK_FREE_LIST);
        STACK_FREE_LIST = stack;
        STACK_FREE_COUNT += 1;
    }
}

/// `thread_cache` of kern/thread.c: the `struct thread` slab cache.
#[unsafe(export_name = "thread_cache")]
static mut THREAD_CACHE: KmemCache = KmemCache::zeroed();

/// `thread_stack_cache` of kern/thread.c: the kernel-stack slab cache.
#[unsafe(export_name = "thread_stack_cache")]
static mut THREAD_STACK_CACHE: KmemCache = KmemCache::zeroed();

/// `thread_template` of kern/thread.c: the image `thread_create()` copies.
#[unsafe(export_name = "thread_template")]
static mut THREAD_TEMPLATE: Thread =
    unsafe { MaybeUninit::zeroed().assume_init() };

/// `reaper_queue` of kern/thread.c: the threads waiting for the reaper.
#[unsafe(export_name = "reaper_queue")]
static mut REAPER_QUEUE: QueueEntry = QueueEntry::unlinked();

/// `reaper_lock` of kern/thread.c: protects `reaper_queue`.
#[unsafe(export_name = "reaper_lock")]
static mut REAPER_LOCK: SimpleLock = SimpleLock::new();

/// `stack_lock_data` of kern/thread.c: protects the cached-stack free list,
/// at splsched.
#[unsafe(export_name = "stack_lock_data")]
static mut STACK_LOCK_DATA: SimpleLock = SimpleLock::new();

/// `stack_free_list` and `stack_free_count` of kern/thread.c: the cached
/// stacks, at splsched.
#[unsafe(export_name = "stack_free_list")]
static mut STACK_FREE_LIST: VmOffset = 0;
#[unsafe(export_name = "stack_free_count")]
static mut STACK_FREE_COUNT: c_uint = 0;

/// `stack_free_limit` of kern/thread.c: the cached-stack high-water mark.
#[unsafe(export_name = "stack_free_limit")]
static mut STACK_FREE_LIMIT: c_uint = 1;

/// `thread_deallocate_stack` of kern/thread.c: how many stacks the
/// deallocator freed.
#[unsafe(export_name = "thread_deallocate_stack")]
static mut THREAD_DEALLOCATE_STACK: c_uint = 0;

/// `stack_check_usage` of kern/thread.c: whether stack usage is tracked.
#[unsafe(export_name = "stack_check_usage")]
static mut STACK_CHECK_USAGE: c_int = 0;

/// `stack_usage_lock` of kern/thread.c: protects `stack_max_usage`.
#[unsafe(export_name = "stack_usage_lock")]
static mut STACK_USAGE_LOCK: SimpleLock = SimpleLock::new();

/// `stack_max_usage` of kern/thread.c: the largest kernel-stack usage seen.
#[unsafe(export_name = "stack_max_usage")]
static mut STACK_MAX_USAGE: VmSize = 0;

/// `thread_collect_allowed` of kern/thread.c: whether the collector may run.
#[unsafe(export_name = "thread_collect_allowed")]
static mut THREAD_COLLECT_ALLOWED: c_int = 1;

/// `thread_collect_last_tick` and `thread_collect_max_rate` of kern/thread.c:
/// the last tick the collector ran and the minimum interval, in ticks.
#[unsafe(export_name = "thread_collect_last_tick")]
static mut THREAD_COLLECT_LAST_TICK: c_uint = 0;
#[unsafe(export_name = "thread_collect_max_rate")]
static mut THREAD_COLLECT_MAX_RATE: c_uint = 0;

/// `MACH_PORT_NULL` of <mach/port.h>: no port name.
const MACH_PORT_NULL: c_uint = 0;

/// `THREAD_BASIC_INFO` and `THREAD_SCHED_INFO` of <mach/thread_info.h>.
const THREAD_BASIC_INFO: c_int = 1;
const THREAD_SCHED_INFO: c_int = 2;
/// `TH_USAGE_SCALE`: the scale of the `cpu_usage` field.
const TH_USAGE_SCALE: c_uint = 1000;
/// The `TH_STATE_*` run states of <mach/thread_info.h>.
const TH_STATE_RUNNING: c_int = 1;
const TH_STATE_STOPPED: c_int = 2;
const TH_STATE_WAITING: c_int = 3;
const TH_STATE_UNINTERRUPTIBLE: c_int = 4;
const TH_STATE_HALTED: c_int = 5;
/// `TH_FLAGS_SWAPPED` and `TH_FLAGS_IDLE` of <mach/thread_info.h>.
const TH_FLAGS_SWAPPED: c_int = 0x1;
const TH_FLAGS_IDLE: c_int = 0x2;

/// `struct thread_basic_info` of <mach/thread_info.h>.
#[repr(C)]
struct ThreadBasicInfo {
    user_time: RpcTimeValue,
    system_time: RpcTimeValue,
    cpu_usage: c_int,
    base_priority: c_int,
    cur_priority: c_int,
    run_state: c_int,
    flags: c_int,
    suspend_count: c_int,
    sleep_time: c_int,
    creation_time: RpcTimeValue,
    user_time64: TimeValue64,
    system_time64: TimeValue64,
    creation_time64: TimeValue64,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<ThreadBasicInfo>() == 128);
    assert!(offset_of!(ThreadBasicInfo, user_time) == 0);
    assert!(offset_of!(ThreadBasicInfo, system_time) == 16);
    assert!(offset_of!(ThreadBasicInfo, cpu_usage) == 32);
    assert!(offset_of!(ThreadBasicInfo, run_state) == 44);
    assert!(offset_of!(ThreadBasicInfo, sleep_time) == 56);
    assert!(offset_of!(ThreadBasicInfo, creation_time) == 64);
    assert!(offset_of!(ThreadBasicInfo, user_time64) == 80);
    assert!(offset_of!(ThreadBasicInfo, system_time64) == 96);
    assert!(offset_of!(ThreadBasicInfo, creation_time64) == 112);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<ThreadBasicInfo>() == 100);
    assert!(offset_of!(ThreadBasicInfo, user_time) == 0);
    assert!(offset_of!(ThreadBasicInfo, system_time) == 8);
    assert!(offset_of!(ThreadBasicInfo, cpu_usage) == 16);
    assert!(offset_of!(ThreadBasicInfo, run_state) == 28);
    assert!(offset_of!(ThreadBasicInfo, sleep_time) == 40);
    assert!(offset_of!(ThreadBasicInfo, creation_time) == 44);
    assert!(offset_of!(ThreadBasicInfo, user_time64) == 52);
    assert!(offset_of!(ThreadBasicInfo, system_time64) == 68);
    assert!(offset_of!(ThreadBasicInfo, creation_time64) == 84);
};

/// `struct thread_sched_info` of <mach/thread_info.h>.
#[repr(C)]
struct ThreadSchedInfo {
    policy: c_int,
    data: c_int,
    base_priority: c_int,
    max_priority: c_int,
    cur_priority: c_int,
    depressed: c_int,
    depress_priority: c_int,
    last_processor: c_int,
}

/// The C's `sizeof(struct T) / sizeof(natural_t)`, the `THREAD_*_COUNT` of
/// <mach/thread_info.h>.  Every count is a few dozen, so the narrowing cannot
/// lose a bit.
const fn info_count(size: usize) -> c_uint {
    (size / size_of::<c_int>()) as c_uint
}

const THREAD_BASIC_INFO_COUNT: c_uint =
    info_count(size_of::<ThreadBasicInfo>());
const THREAD_BASIC_INFO_LEGACY_COUNT: usize =
    offset_of!(ThreadBasicInfo, user_time64) / size_of::<c_int>();
const THREAD_SCHED_INFO_COUNT: c_uint =
    info_count(size_of::<ThreadSchedInfo>());

const _: () = assert!(size_of::<ThreadSchedInfo>() == 32);

/// `default_pset` of <kern/processor.h> as a typed pointer.
pub(crate) fn default_pset() -> *mut ProcessorSet {
    ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>()
}

/// The stack accounting `host_stack_usage()` and
/// `processor_set_stack_usage()` report.
pub(crate) struct StackUsage {
    /// The number of stacks counted.
    pub total: c_uint,
    /// The VM space they reserve, equal to the resident space.
    pub space: VmSize,
    /// The largest usage seen, when `stack_check_usage` is on.
    pub maxusage: VmSize,
    /// The address of the thread with the largest stack.
    pub maxstack: VmOffset,
}

impl Thread {
    /// `thread_create()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `parent_task` must be null or point at a live task, and the caller
    /// must hold no locks: the routine allocates and may block.
    pub(crate) unsafe fn create(
        parent_task: *mut Task,
    ) -> Result<*mut Thread, KernError> {
        if parent_task.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: `thread_init()` built the cache before any thread existed,
        // and the allocation may block, as the caller permits.
        let Some(buf) =
            (unsafe { (*ptr::addr_of_mut!(THREAD_CACHE)).alloc() })
        else {
            return Err(KernError::ResourceShortage);
        };
        let new_thread = buf.as_ptr().cast::<Thread>();

        // SAFETY: the storage is fresh and unshared; every field below is
        // written before the thread is visible to anything else.
        unsafe {
            new_thread.write(ptr::read(ptr::addr_of!(THREAD_TEMPLATE)));
            glue::record_time_stamp(ptr::addr_of_mut!(
                (*new_thread).creation_time
            ));
            (*new_thread).task = parent_task;

            let cur_thread = current_thread();
            let cur_task = current_task();
            if !cur_thread.is_null()
                && cur_task != kernel_task
                && parent_task == cur_task
                && (*cur_thread).vm_privilege != 0
            {
                (*new_thread).vm_privilege = 1;
            }
            (*new_thread).lock.init();
            (*new_thread).sched_stamp = glue::sched_tick;
            thread_timeout_setup(new_thread);
            glue::pcb_init(parent_task, new_thread);
            ipc_thread_init(new_thread);
        }

        // SAFETY: the caller promises a live task; the lock covers the set
        // pointer, and the reference keeps the set alive past the unlock.
        let mut pset = unsafe {
            (*parent_task).lock.lock();
            let pset = (*parent_task).processor_set;
            pset_reference(pset);
            (*parent_task).lock.unlock();
            pset
        };

        // SAFETY: the set is live and referenced; the load average and the
        // new thread's fields are the C's, under the same conditions.
        unsafe {
            let scale = SCHED_SCALE.unsigned_abs();
            let divisor = if (*pset).load_average >= c_long::from(SCHED_SCALE)
            {
                u32::try_from((*pset).load_average).unwrap_or(u32::MAX)
            } else {
                scale
            };
            (*new_thread).cpu_usage = (TIMER_RATE * scale) / divisor;
            (*new_thread).sched_usage = TIMER_RATE * scale;
        }

        // SAFETY: the task is live; the loop holds the new set's lock and the
        // task lock, and the reference keeps whichever set it settles on
        // alive.
        loop {
            unsafe {
                (*pset).lock.lock();
                (*parent_task).lock.lock();

                let mut cur_pset = (*parent_task).processor_set;
                if (*cur_pset).active == 0 {
                    cur_pset = default_pset();
                }

                if cur_pset != pset {
                    pset_reference(cur_pset);
                    (*parent_task).lock.unlock();
                    (*pset).lock.unlock();
                    pset_deallocate(pset);
                    pset = cur_pset;
                    continue;
                }
            }
            break;
        }

        // SAFETY: both sets and the task are locked, and the thread is not
        // visible yet, so its fields are safe to set without its lock.
        unsafe {
            (*new_thread).priority = (*parent_task).priority;
            (*new_thread).max_priority = (*parent_task).max_priority;
            if (*pset).max_priority > (*new_thread).max_priority {
                (*new_thread).max_priority = (*pset).max_priority;
            }
            if (*new_thread).max_priority > (*new_thread).priority {
                (*new_thread).priority = (*new_thread).max_priority;
            }
            compute_priority(new_thread, 1);
            (*new_thread).suspend_count =
                (*parent_task).suspend_count.wrapping_add(1);

            (*pset).add_thread(new_thread);
            if (*pset).empty != 0 {
                (*new_thread).suspend_count =
                    (*new_thread).suspend_count.wrapping_add(1);
            }

            ptr::copy_nonoverlapping(
                (*parent_task).name.as_ptr(),
                (*new_thread).name.as_mut_ptr(),
                TASK_NAME_SIZE,
            );

            (*parent_task).ref_count =
                (*parent_task).ref_count.wrapping_add(1);
            (*parent_task).thread_count =
                (*parent_task).thread_count.wrapping_add(1);
            queue_enter_tail(
                ptr::addr_of_mut!((*parent_task).thread_list),
                new_thread.cast::<c_void>(),
                offset_of!(Thread, thread_list),
            );

            (*new_thread).set_active(true);

            if !(*parent_task).active() {
                (*parent_task).lock.unlock();
                (*pset).lock.unlock();
                let _ = Thread::terminate(new_thread);
                Thread::deallocate(new_thread);
                return Err(KernError::Failure);
            }
            (*parent_task).lock.unlock();
            (*pset).lock.unlock();
        }

        // SAFETY: the thread is live and active, and its IPC state is built.
        unsafe { ipc_thread_enable(new_thread) };

        Ok(new_thread)
    }

    /// `thread_deallocate()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or a live thread the caller holds a reference
    /// to, and the caller must hold no locks: the teardown may block.
    pub(crate) unsafe fn deallocate(thread: *mut Thread) {
        if thread.is_null() {
            return;
        }

        // SAFETY: the caller promises a live thread; only the thread lock is
        // needed for the common case, and splsched keeps the count from
        // reaching zero underneath it.
        unsafe {
            let s = glue::splsched();
            (*thread).lock.lock();
            (*thread).ref_count = (*thread).ref_count.wrapping_sub(1);
            if (*thread).ref_count > 0 {
                (*thread).lock.unlock();
                glue::splx(s);
                return;
            }

            (*thread).ref_count = 1;
            (*thread).lock.unlock();
            glue::splx(s);
        }

        // SAFETY: the caller promises a live thread; the task and set locks
        // dominate the thread lock, so they are taken first, in the C order.
        unsafe {
            let mut pset = (*thread).processor_set;
            (*pset).lock.lock();

            while pset != (*thread).processor_set {
                (*pset).lock.unlock();
                pset = (*thread).processor_set;
                (*pset).lock.lock();
            }

            let task = (*thread).task;
            (*task).lock.lock();

            let s = glue::splsched();
            (*thread).lock.lock();

            (*thread).ref_count = (*thread).ref_count.wrapping_sub(1);
            if (*thread).ref_count > 0 {
                (*thread).lock.unlock();
                glue::splx(s);
                (*task).lock.unlock();
                (*pset).lock.unlock();
                return;
            }

            reset_timeout_check(ptr::addr_of_mut!((*thread).timer));
            reset_timeout_check(ptr::addr_of_mut!((*thread).depress_timer));
            (*thread).depress_priority = -1;

            let mut user_time = TimeValue64::default();
            let mut system_time = TimeValue64::default();
            thread_read_times(
                thread,
                ptr::addr_of_mut!(user_time),
                ptr::addr_of_mut!(system_time),
            );
            add_time64(&mut (*task).total_user_time, user_time);
            add_time64(&mut (*task).total_system_time, system_time);

            (*task).thread_count = (*task).thread_count.wrapping_sub(1);
            queue_remove_generic(
                ptr::addr_of_mut!((*task).thread_list),
                thread.cast::<c_void>(),
                offset_of!(Thread, thread_list),
            );

            (*pset).remove_thread(thread);

            (*thread).lock.unlock();
            glue::splx(s);
            (*task).lock.unlock();
            (*pset).lock.unlock();
            pset_deallocate(pset);
        }

        // SAFETY: the checks are the C's; a live thread is never the current
        // one here, and an unreferenced thread is suspended.
        unsafe {
            if thread == current_thread() {
                glue::Panic(
                    c"kern/thread.c".as_ptr(),
                    line!() as c_int,
                    c"thread_deallocate".as_ptr(),
                    c"thread deallocating itself".as_ptr(),
                )
            }
            if (*thread).state() & !(TH_RUN | TH_HALTED | TH_SWAPPED)
                != TH_SUSP
            {
                glue::Panic(
                    c"kern/thread.c".as_ptr(),
                    line!() as c_int,
                    c"thread_deallocate".as_ptr(),
                    c"unstopped thread destroyed!".as_ptr(),
                )
            }
        }

        // SAFETY: the thread holds the task reference it took at creation,
        // and it is not running; `task_deallocate()` may block.
        unsafe { crate::kern::task::deallocate((*thread).task) };

        // SAFETY: the thread is dead at splsched; its stack, if any, is
        // detached and the count is the C's.
        unsafe {
            if (*thread).state() & TH_SWAPPED == 0 {
                let s = glue::splsched();
                (*thread).stack_free();
                glue::splx(s);
                THREAD_DEALLOCATE_STACK =
                    THREAD_DEALLOCATE_STACK.wrapping_add(1);
            }
        }

        // SAFETY: the thread is dead; the event count takes its own lock.
        unsafe { glue::evc_notify_abort(thread) };
        // SAFETY: as above; `pcb_terminate()` releases the machine state.
        unsafe { glue::pcb_terminate(thread) };

        // SAFETY: the thread came from `THREAD_CACHE`, and the entry check
        // makes its pointer non-null, so the free is sound.
        unsafe {
            (*ptr::addr_of_mut!(THREAD_CACHE))
                .free(NonNull::new_unchecked(thread.cast::<u8>()));
        }
    }

    /// `thread_terminate()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and the caller must
    /// hold no locks: the routine waits and may block.
    pub(crate) unsafe fn terminate(
        thread: *mut Thread,
    ) -> Result<(), KernError> {
        let cur_thread = current_thread();

        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the caller promises a live thread; the IPC lock is taken by
        // the routine itself.
        unsafe { ipc_thread_disable(thread) };

        if thread == cur_thread {
            // SAFETY: the current thread is live; the lock protects the state
            // word and the AST slot.
            unsafe {
                let s = glue::splsched();
                (*thread).lock.lock();
                if (*thread).active() {
                    (*thread).set_active(false);
                    (*thread).ast |= AST_TERMINATE;
                }
                (*thread).lock.unlock();
                // The AST slot holds `usize` reasons; the constant is one bit
                // and cannot lose anything.
                ast_on(cpu_number(), AST_TERMINATE as usize);
                glue::splx(s);
            }
            return Ok(());
        }

        // SAFETY: the caller promises a live thread, and the current thread
        // is live; the locks are taken in address order, as the C did.
        unsafe {
            let cur_task = current_task();
            (*cur_task).lock.lock();
            let s = glue::splsched();
            if thread.addr() < cur_thread.addr() {
                (*thread).lock.lock();
                (*cur_thread).lock.lock();
            } else {
                (*cur_thread).lock.lock();
                (*thread).lock.lock();
            }

            if !(*cur_task).active() || !(*cur_thread).active() {
                (*cur_thread).lock.unlock();
                (*thread).lock.unlock();
                glue::splx(s);
                (*cur_task).lock.unlock();
                let _ = Thread::terminate(cur_thread);
                return Err(KernError::Failure);
            }

            (*cur_thread).lock.unlock();
            (*cur_task).lock.unlock();

            if !(*thread).active() {
                (*thread).lock.unlock();
                glue::splx(s);
                return Err(KernError::Failure);
            }

            (*thread).set_active(false);
            (*thread).lock.unlock();
            glue::splx(s);
        }

        // SAFETY: the thread is live; the routines take the locks they need
        // and may block, as the C's did.
        unsafe {
            Thread::freeze(thread);
            let default_pset = default_pset();
            if (*thread).processor_set != default_pset {
                Thread::doassign(thread, default_pset, false);
            }
            let _ = Thread::halt(thread, true);
            Thread::unfreeze(thread);
            ipc_thread_terminate(thread);
            Thread::deallocate(thread);
        }
        Ok(())
    }

    /// `thread_terminate_release()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` and `task` must be null or point at live objects, and the
    /// caller must hold no locks: the routine deallocates and may block.
    pub(crate) unsafe fn terminate_release(
        thread: *mut Thread,
        task: *mut Task,
        thread_name: c_uint,
        reply_port: c_uint,
        address: VmOffset,
        size: VmSize,
    ) -> Result<(), KernError> {
        if task.is_null() || thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the caller promises a live task; the port routines take the
        // space's own locks, and the C ignored their results.
        unsafe {
            let space = IpcSpace::new((*task).itk_space);
            let _ = mach_port::deallocate(space, thread_name);
            if reply_port != MACH_PORT_NULL {
                let _ = mach_port::destroy(space, reply_port);
            }
        }

        if address != 0 || size != 0 {
            // SAFETY: the task is live, so its map is; `vm_deallocate()`
            // takes the map's own locks and reports failure rather than
            // panicking.
            unsafe {
                let map = (*task).map.cast::<VmMap>();
                if let Some(map) = map.as_mut() {
                    let _ = crate::vm::vm_user::deallocate(map, address, size);
                }
            }
        }

        // SAFETY: the caller's contract for `thread`.
        unsafe { Thread::terminate(thread) }
    }

    /// `thread_halt()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be a live thread other than the current one, and the
    /// caller must hold no locks: the routine waits and may block.
    pub(crate) unsafe fn halt(
        thread: *mut Thread,
        must_halt: bool,
    ) -> Result<(), KernError> {
        let cur_thread = current_thread();

        if thread == cur_thread {
            // SAFETY: `Panic` does not return; the file, function and message
            // tags are the C `panic()` macro's, and the line is this file's.
            unsafe {
                glue::Panic(
                    c"kern/thread.c".as_ptr(),
                    line!() as c_int,
                    c"thread_halt".as_ptr(),
                    c"thread_halt: trying to halt current thread.".as_ptr(),
                )
            }
        }

        // SAFETY: the caller promises a live thread, and the current thread
        // is live; the locks are taken at splsched in address order.
        let mut s = unsafe {
            let s = glue::splsched();

            if must_halt {
                (*thread).lock.lock();

                if (*thread).state() & TH_HALTED != 0 {
                    (*thread).suspend_count =
                        (*thread).suspend_count.wrapping_add(1);
                    (*thread).lock.unlock();
                    glue::splx(s);
                    return Ok(());
                }
            } else {
                if thread.addr() < cur_thread.addr() {
                    (*thread).lock.lock();
                    (*cur_thread).lock.lock();
                } else {
                    (*cur_thread).lock.lock();
                    (*thread).lock.lock();
                }

                if (*thread).state() & TH_HALTED != 0 {
                    (*thread).suspend_count =
                        (*thread).suspend_count.wrapping_add(1);
                    (*cur_thread).lock.unlock();
                    (*thread).lock.unlock();
                    glue::splx(s);
                    return Ok(());
                }

                if (*cur_thread).ast & AST_HALT != 0 {
                    thread_wakeup_prim(
                        (*cur_thread).wake_active_event(),
                        0,
                        THREAD_INTERRUPTED,
                    );
                    (*thread).lock.unlock();
                    (*cur_thread).lock.unlock();
                    glue::splx(s);
                    return Err(KernError::Failure);
                }

                (*cur_thread).lock.unlock();
            }

            s
        };

        // SAFETY: the thread lock is held here in either arm; the state and
        // the wait follow the C's.
        unsafe {
            (*thread).suspend_count = (*thread).suspend_count.wrapping_add(1);
            (*thread).set_state((*thread).state() | TH_SUSP);

            while (*thread).ast & AST_HALT != 0
                && (*thread).state() & TH_HALTED == 0
            {
                (*thread).set_wake_active(true);
                thread_sleep(
                    (*thread).wake_active_event(),
                    ptr::addr_of_mut!((*thread).lock),
                    c_int::from(true),
                );

                if (*thread).state() & TH_HALTED != 0 {
                    glue::splx(s);
                    return Ok(());
                }
                if (*cur_thread).wait_result != THREAD_AWAKENED && !must_halt {
                    glue::splx(s);
                    Thread::release(thread);
                    return Err(KernError::Failure);
                }
                (*thread).lock.lock();
            }

            (*thread).ast |= AST_HALT;

            loop {
                (*thread).lock.unlock();
                glue::splx(s);

                let ret = Thread::dowait(thread, must_halt);

                if ret.is_err() {
                    s = glue::splsched();
                    (*thread).lock.lock();
                    (*thread).ast &= !AST_HALT;
                    thread_wakeup_prim(
                        (*thread).wake_active_event(),
                        0,
                        THREAD_INTERRUPTED,
                    );
                    (*thread).lock.unlock();
                    glue::splx(s);

                    Thread::release(thread);
                    return Err(KernError::Failure);
                }

                clear_wait(thread, THREAD_INTERRUPTED, c_int::from(true));

                if (*thread).state() & TH_HALTED != 0 {
                    return Ok(());
                }

                let swap_func = (*thread).swap_func;
                // The C compared the stored continuation against the clean
                // point routines by address; the bindings give the function
                // items the pointer type that comparison needs.
                let continue_fn: unsafe extern "C" fn() =
                    glue::mach_msg_continue;
                let receive_continue_fn: unsafe extern "C" fn() =
                    glue::mach_msg_receive_continue;
                let has_cleanup = (swap_func.is_some_and(|f| {
                    core::ptr::fn_addr_eq(f, continue_fn)
                        || core::ptr::fn_addr_eq(f, receive_continue_fn)
                })) && glue::mach_msg_interrupt(thread) != 0;
                let exception_return_fn: unsafe extern "C" fn() -> ! =
                    glue::thread_exception_return;
                let bootstrap_return_fn: unsafe extern "C" fn() =
                    glue::thread_bootstrap_return;
                let at_clean_point = has_cleanup
                    || swap_func.is_some_and(|f| {
                        core::ptr::fn_addr_eq(f, exception_return_fn)
                    })
                    || swap_func.is_some_and(|f| {
                        core::ptr::fn_addr_eq(f, bootstrap_return_fn)
                    });

                if at_clean_point {
                    s = glue::splsched();
                    (*thread).lock.lock();
                    (*thread).set_state((*thread).state() | TH_HALTED);
                    (*thread).ast &= !AST_HALT;
                    (*thread).lock.unlock();
                    glue::splx(s);
                    return Ok(());
                }

                s = glue::splsched();
                (*thread).lock.lock();
                if (*thread).state() & TH_SCHED_STATE != TH_SUSP {
                    glue::Panic(
                        c"kern/thread.c".as_ptr(),
                        line!() as c_int,
                        c"thread_halt".as_ptr(),
                        c"thread_halt".as_ptr(),
                    )
                }
                (*thread).set_state((*thread).state() | TH_RUN | TH_UNINT);
                thread_setrun(thread, 0);
            }
        }
    }

    /// `thread_halt_self()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// Runs on the current thread, which must be at a clean kernel point with
    /// no lock held; `continuation` runs when the thread is released again.
    pub(crate) unsafe fn halt_self(continuation: Continuation) {
        let thread = current_thread();

        // SAFETY: the current thread is live; it queues itself for the reaper
        // and only the IPC teardown and the reaper touch it from here on.
        unsafe {
            if (*thread).ast & AST_TERMINATE != 0 {
                ipc_thread_terminate(thread);

                Thread::hold(thread);

                let s = glue::splsched();
                (*ptr::addr_of_mut!(REAPER_LOCK)).lock();
                enqueue_tail(
                    ptr::addr_of_mut!(REAPER_QUEUE),
                    ptr::addr_of_mut!((*thread).links),
                );
                (*ptr::addr_of_mut!(REAPER_LOCK)).unlock();

                (*thread).lock.lock();
                (*thread).set_state((*thread).state() | TH_HALTED);
                (*thread).lock.unlock();
                glue::splx(s);

                thread_wakeup_prim(
                    ptr::addr_of_mut!(REAPER_QUEUE).cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
                glue::thread_block(Some(walking_zombie));
            } else {
                let s = glue::splsched();
                (*thread).lock.lock();
                (*thread).set_state((*thread).state() | TH_HALTED);
                (*thread).ast &= !AST_HALT;
                (*thread).lock.unlock();
                glue::splx(s);
                glue::thread_block(continuation);
            }
        }
    }

    /// `thread_dowait()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be a live thread other than the current one, and the
    /// caller must hold no locks: the routine waits and may block.
    pub(crate) unsafe fn dowait(
        thread: *mut Thread,
        must_halt: bool,
    ) -> Result<(), KernError> {
        if thread == current_thread() {
            // SAFETY: `Panic` does not return; the file, function and message
            // tags are the C `panic()` macro's, and the line is this file's.
            unsafe {
                glue::Panic(
                    c"kern/thread.c".as_ptr(),
                    line!() as c_int,
                    c"thread_dowait".as_ptr(),
                    c"thread_dowait".as_ptr(),
                )
            }
        }

        let mut need_wakeup = false;

        // SAFETY: the caller promises a live thread; the lock protects the
        // state and the wait fields, and is released across each block.
        unsafe {
            let s = glue::splsched();
            (*thread).lock.lock();

            let mut result = Ok(());
            loop {
                let mut need_wait = false;
                match (*thread).state() & TH_SCHED_STATE {
                    TH_SUSP | TH_WAIT_SUSP => (),
                    TH_RUN_SUSP => {
                        if glue::rem_runq(thread) != RUN_QUEUE_NULL {
                            (*thread).set_state((*thread).state() & !TH_RUN);
                            need_wakeup = (*thread).wake_active();
                            (*thread).set_wake_active(false);
                            break;
                        }
                        if !(*thread).last_processor.is_null() {
                            cause_ast_check((*thread).last_processor);
                        }
                        need_wait = true;
                    }
                    TH_RUN_SUSP_UNINT
                    | TH_RUN_WAIT_SUSP
                    | TH_RUN_WAIT_SUSP_UNINT
                    | TH_WAIT_SUSP_UNINT => need_wait = true,
                    _ => (),
                }
                if !need_wait {
                    break;
                }

                (*thread).set_wake_active(true);
                thread_sleep(
                    (*thread).wake_active_event(),
                    ptr::addr_of_mut!((*thread).lock),
                    c_int::from(true),
                );
                (*thread).lock.lock();
                if (*current_thread()).wait_result != THREAD_AWAKENED
                    && !must_halt
                {
                    result = Err(KernError::Failure);
                    break;
                }
            }

            (*thread).lock.unlock();
            glue::splx(s);

            if need_wakeup {
                thread_wakeup_prim(
                    (*thread).wake_active_event(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            result
        }
    }

    /// `thread_suspend()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and the caller must
    /// hold no locks: the routine waits and may block.
    pub(crate) unsafe fn suspend(
        thread: *mut Thread,
    ) -> Result<(), KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        let mut hold = false;
        let mut spl = unsafe { glue::splsched() };

        // SAFETY: the caller promises a live thread; the lock is held across
        // the stop count and state update, and released around the block.
        unsafe {
            (*thread).lock.lock();
            while (*thread).state() & TH_UNINT != 0 {
                assert_wait((*thread).state_event(), c_int::from(true));
                (*thread).lock.unlock();
                glue::thread_block(None);
                (*thread).lock.lock();
            }

            let stop_count = (*thread).user_stop_count;
            (*thread).user_stop_count = stop_count.wrapping_add(1);
            if stop_count == 0 {
                hold = true;
                (*thread).suspend_count =
                    (*thread).suspend_count.wrapping_add(1);
                (*thread).set_state((*thread).state() | TH_SUSP);
            }
            (*thread).lock.unlock();
            glue::splx(spl);
        }

        if hold {
            if thread == current_thread() {
                // SAFETY: the current thread is live; the AST slot takes the
                // write and the level is restored.
                unsafe {
                    spl = glue::splsched();
                    ast_on(cpu_number(), AST_BLOCK);
                    glue::splx(spl);
                }
            } else {
                // SAFETY: the caller's contract; the wait may block.
                let _ = unsafe { Thread::dowait(thread, true) };
            }
        }
        Ok(())
    }

    /// `thread_info()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must be null or point at a live thread, and `out` must be
    /// writable for the `count` `integer_t`s the caller passes.
    pub(crate) unsafe fn info(
        thread: *mut Thread,
        flavor: c_int,
        out: *mut c_int,
        count: c_uint,
    ) -> Result<c_uint, KernError> {
        if thread.is_null() {
            return Err(KernError::InvalidArgument);
        }

        match flavor {
            THREAD_BASIC_INFO => {
                // The count is a `natural_t`, and both targets widen it to
                // `usize`.
                if (count as usize) < THREAD_BASIC_INFO_LEGACY_COUNT {
                    return Err(KernError::InvalidArgument);
                }

                let basic = out.cast::<ThreadBasicInfo>();

                // SAFETY: the caller promises a live thread; the lock covers
                // the scheduler fields and the timers.
                unsafe {
                    let s = glue::splsched();
                    (*thread).lock.lock();

                    if (*thread).state() & TH_RUN == 0
                        && (*thread).sched_stamp != glue::sched_tick
                    {
                        glue::update_priority(thread);
                    }

                    let mut user_time = TimeValue64::default();
                    let mut system_time = TimeValue64::default();
                    thread_read_times(
                        thread,
                        ptr::addr_of_mut!(user_time),
                        ptr::addr_of_mut!(system_time),
                    );
                    ptr::addr_of_mut!((*basic).user_time)
                        .write(RpcTimeValue::from(TimeValue::from(user_time)));
                    ptr::addr_of_mut!((*basic).system_time).write(
                        RpcTimeValue::from(TimeValue::from(system_time)),
                    );

                    ptr::addr_of_mut!((*basic).base_priority)
                        .write((*thread).priority);
                    ptr::addr_of_mut!((*basic).cur_priority)
                        .write((*thread).sched_pri);

                    let mut creation_time = TimeValue64::default();
                    read_time_stamp(
                        ptr::addr_of!((*thread).creation_time),
                        ptr::addr_of_mut!(creation_time),
                    );
                    ptr::addr_of_mut!((*basic).creation_time).write(
                        RpcTimeValue::from(TimeValue::from(creation_time)),
                    );

                    if count == THREAD_BASIC_INFO_COUNT {
                        // The C wrote `user_time` into `system_time64`; the
                        // copy is kept.
                        ptr::addr_of_mut!((*basic).user_time64)
                            .write(user_time);
                        ptr::addr_of_mut!((*basic).system_time64)
                            .write(user_time);
                        ptr::addr_of_mut!((*basic).creation_time64)
                            .write(creation_time);
                    }

                    let usage =
                        (*thread).cpu_usage / (TIMER_RATE / TH_USAGE_SCALE);
                    // The C stored the `unsigned` quotient into an
                    // `integer_t`; the `as` keeps the low bits as the C
                    // conversion did.
                    ptr::addr_of_mut!((*basic).cpu_usage)
                        .write((usage * 3 / 5) as c_int);

                    let state = (*thread).state();
                    let mut flags = 0;
                    if state & TH_SWAPPED != 0 {
                        flags |= TH_FLAGS_SWAPPED;
                    }
                    if state & TH_IDLE != 0 {
                        flags |= TH_FLAGS_IDLE;
                    }

                    let run_state = if state & TH_HALTED != 0 {
                        TH_STATE_HALTED
                    } else if state & TH_RUN != 0 {
                        TH_STATE_RUNNING
                    } else if state & TH_UNINT != 0 {
                        TH_STATE_UNINTERRUPTIBLE
                    } else if state & TH_SUSP != 0 {
                        TH_STATE_STOPPED
                    } else if state & TH_WAIT != 0 {
                        TH_STATE_WAITING
                    } else {
                        0
                    };

                    ptr::addr_of_mut!((*basic).run_state).write(run_state);
                    ptr::addr_of_mut!((*basic).flags).write(flags);
                    ptr::addr_of_mut!((*basic).suspend_count)
                        .write((*thread).user_stop_count);
                    let sleep_time = if run_state == TH_STATE_RUNNING {
                        0
                    } else {
                        // The C stored the `unsigned` difference into an
                        // `integer_t`; the `as` keeps the low bits.
                        glue::sched_tick.wrapping_sub((*thread).sched_stamp)
                            as c_int
                    };
                    ptr::addr_of_mut!((*basic).sleep_time).write(sleep_time);

                    (*thread).lock.unlock();
                    glue::splx(s);
                }

                if count > THREAD_BASIC_INFO_COUNT {
                    Ok(THREAD_BASIC_INFO_COUNT)
                } else {
                    Ok(count)
                }
            }
            THREAD_SCHED_INFO => {
                if count < THREAD_SCHED_INFO_COUNT - 1 {
                    return Err(KernError::InvalidArgument);
                }

                let sched = out.cast::<ThreadSchedInfo>();

                // SAFETY: the caller promises a live thread; the lock covers
                // every field below.
                unsafe {
                    let s = glue::splsched();
                    (*thread).lock.lock();

                    ptr::addr_of_mut!((*sched).policy).write((*thread).policy);
                    let data = if (*thread).policy == POLICY_FIXEDPRI {
                        (*thread).sched_data.wrapping_mul(glue::tick) / 1000
                    } else {
                        0
                    };
                    ptr::addr_of_mut!((*sched).data).write(data);
                    ptr::addr_of_mut!((*sched).base_priority)
                        .write((*thread).priority);
                    ptr::addr_of_mut!((*sched).max_priority)
                        .write((*thread).max_priority);
                    ptr::addr_of_mut!((*sched).cur_priority)
                        .write((*thread).sched_pri);
                    ptr::addr_of_mut!((*sched).depressed)
                        .write(c_int::from((*thread).depress_priority >= 0));
                    ptr::addr_of_mut!((*sched).depress_priority)
                        .write((*thread).depress_priority);

                    let last_processor = if (*thread).last_processor.is_null()
                    {
                        0
                    } else {
                        (*(*thread).last_processor).slot_num
                    };
                    ptr::addr_of_mut!((*sched).last_processor)
                        .write(last_processor);

                    (*thread).lock.unlock();
                    glue::splx(s);
                }

                Ok(THREAD_SCHED_INFO_COUNT)
            }
            _ => Err(KernError::InvalidArgument),
        }
    }

    /// `thread_freeze()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread, and the caller must hold no
    /// locks: the wait may block.
    pub(crate) unsafe fn freeze(thread: *mut Thread) {
        // SAFETY: the caller promises a live thread; the lock is released
        // across the wait, whose key is `assign_active`.
        unsafe {
            let s = glue::splsched();
            (*thread).lock.lock();
            while (*thread).may_assign == 0 {
                (*thread).assign_active = 1;
                thread_sleep(
                    ptr::addr_of_mut!((*thread).assign_active)
                        .cast::<c_void>(),
                    ptr::addr_of_mut!((*thread).lock),
                    0,
                );
                (*thread).lock.lock();
            }
            (*thread).may_assign = 0;
            (*thread).lock.unlock();
            glue::splx(s);
        }
    }

    /// `thread_doassign()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// `thread` must point at a live thread and `new_pset` at a live
    /// processor set, and the caller must hold no locks: the routine waits
    /// and may block.
    pub(crate) unsafe fn doassign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
        release_freeze: bool,
    ) {
        let mut new_pset = new_pset;

        // SAFETY: the caller promises live objects; every lock below is
        // taken at splsched in the C order.
        unsafe {
            let pset = (*thread).processor_set;
            if pset == new_pset {
                if release_freeze {
                    Thread::unfreeze(thread);
                }
                return;
            }

            Thread::hold(thread);
            if thread != current_thread() {
                let _ = Thread::dowait(thread, true);
            }

            loop {
                if pset.addr() < new_pset.addr() {
                    (*pset).lock.lock();
                    (*new_pset).lock.lock();
                } else {
                    (*new_pset).lock.lock();
                    (*pset).lock.lock();
                }

                if (*new_pset).active == 0 {
                    (*pset).lock.unlock();
                    (*new_pset).lock.unlock();
                    new_pset = default_pset();
                    continue;
                }
                break;
            }

            pset_reference(new_pset);

            let s = glue::splsched();
            (*thread).lock.lock();

            Thread::change_psets(thread, pset, new_pset);

            let old_empty = (*pset).empty;
            let new_empty = (*new_pset).empty;

            (*pset).lock.unlock();

            let mut recompute_pri = false;
            if (*thread).policy & (*new_pset).policies == 0 {
                (*thread).policy = POLICY_TIMESHARE;
                recompute_pri = true;
            }

            if (*thread).max_priority < (*new_pset).max_priority {
                (*thread).max_priority = (*new_pset).max_priority;
                if (*thread).priority < (*thread).max_priority {
                    (*thread).priority = (*thread).max_priority;
                    recompute_pri = true;
                } else if (*thread).depress_priority >= 0
                    && (*thread).depress_priority < (*thread).max_priority
                {
                    (*thread).depress_priority = (*thread).max_priority;
                }
            }

            (*new_pset).lock.unlock();

            if recompute_pri {
                compute_priority(thread, 1);
            }

            if release_freeze {
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
            }

            (*thread).lock.unlock();
            glue::splx(s);

            pset_deallocate(pset);

            if old_empty != 0 {
                Thread::release(thread);
            }
            if new_empty == 0 {
                Thread::release(thread);
            }

            if thread == current_thread() {
                let s = glue::splsched();
                ast_on(cpu_number(), AST_BLOCK);
                glue::splx(s);
            }
        }
    }

    /// `thread_assign()` of kern/thread.c, the `MACH_HOST` arm both
    /// configured builds take.
    ///
    /// # Safety
    ///
    /// `thread` must be null or a live thread the caller holds an extra
    /// reference to, `new_pset` must be null or a live processor set, and the
    /// caller must hold no locks.
    pub(crate) unsafe fn assign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
    ) -> Result<(), KernError> {
        if thread.is_null() || new_pset.is_null() {
            return Err(KernError::InvalidArgument);
        }

        // SAFETY: the caller's contract; both routines may block.
        unsafe {
            Thread::freeze(thread);
            Thread::doassign(thread, new_pset, true);
        }
        Ok(())
    }

    /// `thread_collect_scan()` of kern/thread.c: walk every processor set's
    /// threads and let the machine layer release what it can.
    ///
    /// # Safety
    ///
    /// `kern/thread.c`'s collector calls this with nothing locked, as the C
    /// did.
    pub(crate) unsafe fn collect_scan() {
        let mut prev_thread: *mut Thread = ptr::null_mut();
        let mut prev_pset: *mut ProcessorSet = ptr::null_mut();

        // SAFETY: `all_psets` and its lock are the C globals; the walk keeps
        // a reference on both the set and the thread between iterations.
        unsafe {
            let all_psets = ptr::addr_of_mut!(glue::all_psets);
            let all_psets_lock = ptr::addr_of_mut!(glue::all_psets_lock);

            (*all_psets_lock).lock();
            let mut pset_entry = queue_first(all_psets);
            while queue_end(all_psets, pset_entry) == 0 {
                let pset = pset_entry.cast::<ProcessorSet>();
                (*pset).lock.lock();

                let threads = ptr::addr_of_mut!((*pset).threads);
                let mut thread_entry = queue_first(threads);
                while queue_end(threads, thread_entry) == 0 {
                    let thread = thread_entry.cast::<Thread>();

                    let s = glue::splsched();
                    (*thread).lock.lock();

                    if (*thread).state() & (TH_RUN | TH_SWAPPED) == TH_SWAPPED
                    {
                        (*thread).ref_count =
                            (*thread).ref_count.wrapping_add(1);
                        (*thread).lock.unlock();
                        glue::splx(s);
                        (*pset).ref_count = (*pset).ref_count.wrapping_add(1);
                        (*pset).lock.unlock();
                        (*all_psets_lock).unlock();

                        crate::arch::i386::pcb::pcb_collect(thread);

                        if !prev_thread.is_null() {
                            Thread::deallocate(prev_thread);
                        }
                        prev_thread = thread;

                        if !prev_pset.is_null() {
                            pset_deallocate(prev_pset);
                        }
                        prev_pset = pset;

                        (*all_psets_lock).lock();
                        (*pset).lock.lock();
                    } else {
                        (*thread).lock.unlock();
                        glue::splx(s);
                    }

                    thread_entry =
                        queue_next(ptr::addr_of_mut!((*thread).pset_threads));
                }
                (*pset).lock.unlock();
                pset_entry = queue_next(ptr::addr_of_mut!((*pset).all_psets));
            }
            (*all_psets_lock).unlock();

            if !prev_thread.is_null() {
                Thread::deallocate(prev_thread);
            }
            if !prev_pset.is_null() {
                pset_deallocate(prev_pset);
            }
        }
    }

    /// `thread_stats()` of kern/thread.c.
    ///
    /// # Safety
    ///
    /// Reached from the debugger; the queue walk takes no locks, as the C
    /// did, so every link must be a live thread for the walk.
    pub(crate) unsafe fn stats() {
        let pset = default_pset();
        // SAFETY: `default_pset` is the live global set the bootstrap built,
        // and the walk reads its thread list as the C did.
        unsafe {
            let list = ptr::addr_of_mut!((*pset).threads);
            let mut total = 0;
            let mut rpcreply = 0;
            let mut thread = queue_first(list).cast::<Thread>();
            while queue_end(list, thread.cast::<QueueEntry>()) == 0 {
                total += 1;
                if !(*thread).ith_rpc_reply.is_null() {
                    rpcreply += 1;
                }
                thread = queue_next(ptr::addr_of_mut!((*thread).pset_threads))
                    .cast::<Thread>();
            }

            // SAFETY: `printf` is the C variadic; each format string takes
            // the one `int` argument passed, as the C did.
            glue::printf(c"%d total threads.\n".as_ptr(), total);
            glue::printf(c"%d using rpc_reply.\n".as_ptr(), rpcreply);
        }
    }
}

/// `walking_zombie()` of kern/thread.c, the private continuation of a
/// terminating thread.
unsafe extern "C" fn walking_zombie() {
    // SAFETY: `Panic` does not return; the file, function and message tags
    // are the C `panic()` macro's.
    unsafe {
        glue::Panic(
            c"kern/thread.c".as_ptr(),
            line!() as c_int,
            c"walking_zombie".as_ptr(),
            c"the zombie walks!".as_ptr(),
        )
    }
}

/// `reaper_thread_continue()` of kern/thread.c: the reaper's loop, which the
/// `reaper_thread()` continuation runs forever.
///
/// # Safety
///
/// Runs as the reaper kernel thread, which `kernel_thread()` starts once.
pub(crate) unsafe extern "C" fn reaper_thread_continue() {
    loop {
        // SAFETY: the reaper runs alone; the lock protects the queue, and
        // both waits release it.
        unsafe {
            let mut s = glue::splsched();
            (*ptr::addr_of_mut!(REAPER_LOCK)).lock();

            let mut entry = dequeue_head(ptr::addr_of_mut!(REAPER_QUEUE));
            while !entry.is_null() {
                (*ptr::addr_of_mut!(REAPER_LOCK)).unlock();
                glue::splx(s);

                let thread = entry.cast::<Thread>();
                let _ = Thread::dowait(thread, true);
                Thread::deallocate(thread);

                s = glue::splsched();
                (*ptr::addr_of_mut!(REAPER_LOCK)).lock();
                entry = dequeue_head(ptr::addr_of_mut!(REAPER_QUEUE));
            }

            assert_wait(ptr::addr_of_mut!(REAPER_QUEUE).cast::<c_void>(), 0);
            (*ptr::addr_of_mut!(REAPER_LOCK)).unlock();
            glue::splx(s);
            glue::thread_block(Some(reaper_thread_continue));
        }
    }
}

/// `kernel_thread()` of kern/thread.c.
///
/// # Safety
///
/// `task` must point at a live task, `name` must be a NUL-terminated string,
/// `start` must be a continuation, and the caller must hold no locks: the
/// routine may block.
pub(crate) unsafe fn kernel_thread(
    task: *mut Task,
    _name: *const c_char,
    start: Continuation,
    arg: *mut c_void,
) -> *mut Thread {
    // SAFETY: the caller's contract; `create()` reports a shortage rather
    // than returning a null thread.
    let thread = match unsafe { Thread::create(task) } {
        Ok(thread) => thread,
        Err(_) => return ptr::null_mut(),
    };

    // SAFETY: the thread is live and holds the extra reference the C
    // released here; the swap-in may block and resumes it.
    unsafe {
        Thread::deallocate(thread);
        (*thread).start(start);
        (*thread).saved.other = arg;
        let _ = crate::kern::thread_swap::doswapin(thread);
        (*thread).max_priority = BASEPRI_SYSTEM;
        (*thread).priority = BASEPRI_SYSTEM;
        (*thread).sched_pri = BASEPRI_SYSTEM;
        let _ = Thread::resume(thread);
    }
    thread
}

/// `consider_thread_collect()` of kern/thread.c.
///
/// # Safety
///
/// The pageout daemon calls this with nothing locked, as the C did.
pub(crate) unsafe fn consider_collect() {
    // The C's `hz / 1` and the usual arithmetic conversions reinterpret the
    // signed tick rate as unsigned; `hz` is positive and set before the
    // pageout daemon can run.
    let hz = unsafe { glue::hz }.unsigned_abs();
    let mut max_rate = unsafe { THREAD_COLLECT_MAX_RATE };
    if max_rate == 0 {
        max_rate = hz;
        // SAFETY: this is the collector's own state, and it runs on one
        // thread.
        unsafe { THREAD_COLLECT_MAX_RATE = max_rate };
    }

    let last_tick = unsafe { THREAD_COLLECT_LAST_TICK };
    let deadline = last_tick.wrapping_add(max_rate / hz);
    if unsafe { THREAD_COLLECT_ALLOWED } != 0
        && unsafe { glue::sched_tick } > deadline
    {
        // SAFETY: as above.
        unsafe {
            THREAD_COLLECT_LAST_TICK = glue::sched_tick;
        }
        unsafe { Thread::collect_scan() };
    }
}

/// `stack_usage()` of kern/thread.c: how much of `stack` the marker fill no
/// longer covers.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel
/// stack object.
unsafe fn stack_usage(stack: VmOffset) -> VmSize {
    // SAFETY: the caller promises a whole stack object, and
    // `KERNEL_STACK_SIZE` is a whole number of `unsigned int`s, so the slice
    // covers exactly the object.
    let words = unsafe {
        core::slice::from_raw_parts(
            ptr::with_exposed_provenance::<u32>(stack),
            KERNEL_STACK_SIZE / size_of::<u32>(),
        )
    };
    let used = words
        .iter()
        .position(|word| *word != STACK_MARKER)
        .unwrap_or(words.len());
    KERNEL_STACK_SIZE - used * size_of::<u32>()
}

/// `stack_finalize()` of kern/thread.c: account for a stack about to be
/// released.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel
/// stack object.
pub(crate) unsafe fn stack_finalize(stack: VmOffset) {
    if unsafe { STACK_CHECK_USAGE } == 0 {
        return;
    }

    let used = unsafe { stack_usage(stack) };

    // SAFETY: `stack_usage_lock` is the lock `thread_init()` built for the
    // accounting pair.
    unsafe {
        let lock = ptr::addr_of_mut!(STACK_USAGE_LOCK);
        (*lock).lock();
        if used > STACK_MAX_USAGE {
            STACK_MAX_USAGE = used;
        }
        (*lock).unlock();
    }
}

/// `stack_statistics()` of kern/thread.c: walk the cached stacks, raising
/// `maxusage`.
///
/// # Safety
///
/// The caller must hold no lock: the routine takes `stack_lock_data` at
/// splsched.
unsafe fn stack_statistics(mut maxusage: VmSize) -> (c_uint, VmSize) {
    // SAFETY: `stack_lock_data` is the lock `thread_init()` built; the free
    // list holds only whole cache objects.
    unsafe {
        let s = glue::splsched();
        let lock = ptr::addr_of_mut!(STACK_LOCK_DATA);
        (*lock).lock();

        if STACK_CHECK_USAGE != 0 {
            let mut stack = STACK_FREE_LIST;
            while stack != 0 {
                let usage = stack_usage(stack);
                if usage > maxusage {
                    maxusage = usage;
                }
                stack = stack_next(stack);
            }
        }

        let total = STACK_FREE_COUNT;
        (*lock).unlock();
        glue::splx(s);
        (total, maxusage)
    }
}

/// `stack_init()` of kern/thread.c: fill a fresh stack with the usage marker
/// when the check is on.
///
/// # Safety
///
/// `stack` must be the base address of a live `KERNEL_STACK_SIZE` kernel
/// stack object.
pub(crate) unsafe fn stack_init(stack: VmOffset) {
    if unsafe { STACK_CHECK_USAGE } == 0 {
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

/// `host_stack_usage()` of kern/thread.c.
///
/// # Safety
///
/// `host` must be null or the live host the MIG stub converted.
pub(crate) unsafe fn host_stack_usage(
    host: *mut c_void,
) -> Result<StackUsage, KernError> {
    if host.is_null() {
        return Err(KernError::InvalidHost);
    }

    // SAFETY: `stack_usage_lock` is the lock `thread_init()` built for the
    // accounting pair.
    let maxusage = unsafe {
        let lock = ptr::addr_of_mut!(STACK_USAGE_LOCK);
        (*lock).lock();
        let maxusage = STACK_MAX_USAGE;
        (*lock).unlock();
        maxusage
    };

    // SAFETY: the caller holds no lock, as `stack_statistics()` requires.
    let (total, maxusage) = unsafe { stack_statistics(maxusage) };

    // The C multiplied the `unsigned` count by a `vm_size_t`; the count fits
    // `usize` on both targets and the product wraps in the C too.
    let space = (total as usize).wrapping_mul(round_page(KERNEL_STACK_SIZE));
    Ok(StackUsage {
        total,
        space,
        maxusage,
        maxstack: 0,
    })
}

/// `processor_set_stack_usage()` of kern/thread.c.
///
/// # Safety
///
/// `pset` must be null or point at a live processor set, and the caller must
/// hold no locks: the routine allocates.
pub(crate) unsafe fn processor_set_stack_usage(
    pset: *mut ProcessorSet,
) -> Result<StackUsage, KernError> {
    if pset.is_null() {
        return Err(KernError::InvalidArgument);
    }

    let mut size: VmSize = 0;
    let mut addr: Option<NonNull<u8>> = None;
    let mut actual: c_uint;
    let mut size_needed: usize;

    loop {
        // SAFETY: the caller promises a live set; the lock is taken here and
        // the break below leaves it held.
        unsafe {
            (*pset).lock.lock();
            if (*pset).active == 0 {
                (*pset).lock.unlock();
                return Err(KernError::InvalidArgument);
            }

            // The C read the `int` count into an `unsigned int`; it is the
            // number of threads and never negative.
            actual = (*pset).thread_count as c_uint;
            // The `unsigned int` count widens on both targets.
            size_needed = actual as usize * size_of::<VmOffset>();
            if size_needed <= size {
                break;
            }

            (*pset).lock.unlock();
        }

        if let Some(old) = addr {
            // SAFETY: the old buffer is the live allocation of `size` bytes
            // made above.
            unsafe { kfree(old, size) };
        }
        size = size_needed;
        // SAFETY: `kalloc_init()` ran during the boot this routine follows.
        let Some(buf) = kalloc(size) else {
            return Err(KernError::ResourceShortage);
        };
        addr = Some(buf);
    }

    let Some(buf) = addr else {
        // The count was zero on the first look, so nothing was allocated.
        // SAFETY: the set lock was left held by the break above.
        unsafe { (*pset).lock.unlock() };
        return Ok(StackUsage {
            total: 0,
            space: 0,
            maxusage: 0,
            maxstack: 0,
        });
    };

    let threads = buf.as_ptr().cast::<VmOffset>();
    // SAFETY: the set lock is held, so every queue entry is a live thread,
    // and the references taken here keep them alive.  An address is the same
    // width as the thread pointers the C stored.
    unsafe {
        let list = ptr::addr_of_mut!((*pset).threads);
        let mut entry = queue_first(list);
        for i in 0..actual as usize {
            let thread = entry.cast::<Thread>();
            Thread::reference(thread);
            threads.add(i).write(thread.addr());
            entry = queue_next(ptr::addr_of_mut!((*thread).pset_threads));
        }
        (*pset).lock.unlock();
    }

    let mut total: c_uint = 0;
    let mut maxusage: VmSize = 0;
    let mut maxstack: VmOffset = 0;
    let ncpus = smp_get_numcpus();

    for i in 0..actual as usize {
        // SAFETY: every slot holds a referenced live thread, and the
        // reference is dropped at the end of the iteration.
        let thread = unsafe {
            ptr::with_exposed_provenance_mut::<Thread>(threads.add(i).read())
        };
        let mut stack: VmOffset = 0;

        // SAFETY: the thread is live; the state read is the C's unlocked
        // one, and a swapped thread's stack is not looked at.
        unsafe {
            if (*thread).state() & TH_SWAPPED == 0 {
                stack = (*thread).kernel_stack;

                for cpu in 0..ncpus {
                    let percpu = percpu_at(c_int::from(cpu));
                    if (*percpu).active_thread == thread {
                        stack = (*percpu).active_stack;
                        break;
                    }
                }
            }

            if stack != 0 {
                total = total.wrapping_add(1);

                if STACK_CHECK_USAGE != 0 {
                    let usage = stack_usage(stack);

                    if usage > maxusage {
                        maxusage = usage;
                        maxstack = thread.addr();
                    }
                }
            }

            Thread::deallocate(thread);
        }
    }

    if size != 0 {
        // SAFETY: the buffer is the live allocation of `size` bytes, and
        // every reference it held was dropped above.
        unsafe { kfree(buf, size) };
    }

    // The C multiplied the `unsigned` count by a `vm_size_t`; the count fits
    // `usize` on both targets and the product wraps in the C too.
    let space = (total as usize).wrapping_mul(round_page(KERNEL_STACK_SIZE));
    Ok(StackUsage {
        total,
        space,
        maxusage,
        maxstack,
    })
}
