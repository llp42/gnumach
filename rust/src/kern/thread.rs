// SPDX-License-Identifier: CMU-Mach
// Derived from kern/thread.h:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Derived from kern/timer.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread record, which `kern/thread.h` declares.
//!
//! This is the full `struct thread` mirror, field for field through
//! `name`, because the scheduler reaches fields late in the record
//! (`timer`, `processor_set`, `bound_processor`) and only the whole
//! layout can be asserted against the C compiler's numbers.  The
//! embedded records that Rust does not operate on yet (the saved IPC
//! state, the statistical timers) are mirrored for their size and
//! alignment alone.
//!
//! `state`, `wake_active` and `active` are an anonymous C bitfield
//! unioned with `event_key`; Rust cannot express either, so the word
//! stays raw in [`StateBits`] with accessors, and the union keeps the
//! same address for the C code that forms `TH_EV_WAKE_ACTIVE(t)`.

use crate::arch::types::VmOffset;
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::Timeout;
use crate::kern::processor::{Processor, ProcessorSet, RunQueue};
use crate::kern::queue::QueueEntry;
use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::mem::offset_of;

/// `size_of(struct thread)` on each kernel; see the module's layout
/// assertions.
#[cfg(target_pointer_width = "64")]
const THREAD_SIZE: usize = 560;
#[cfg(target_pointer_width = "32")]
const THREAD_SIZE: usize = 372;

/// `TASK_NAME_SIZE` in <kern/task.h>, the length of `thread.name`.
pub const TASK_NAME_SIZE: usize = 32;

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

/// The bitfield word of `struct thread`, which C declares as
/// `unsigned state:16; unsigned wake_active:1; unsigned active:1`.
///
/// The word is one `unsigned int`; the bits sit in declaration order
/// from the least significant bit.  C reads and writes the same word
/// through its own members.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateBits(u32);

impl StateBits {
    /// The `state:16` mask.
    const STATE_MASK: u32 = 0xffff;
    /// The `wake_active:1` bit.
    const WAKE_ACTIVE: u32 = 1 << 16;
    /// The `active:1` bit.
    const ACTIVE: u32 = 1 << 17;

    /// The `state` half of the word.
    pub const fn state(self) -> u32 {
        self.0 & Self::STATE_MASK
    }

    /// The `wake_active` bit: someone is waiting for this thread to
    /// become suspended.
    pub const fn wake_active(self) -> bool {
        self.0 & Self::WAKE_ACTIVE != 0
    }

    /// The `active` bit: how alive the thread is.
    pub const fn active(self) -> bool {
        self.0 & Self::ACTIVE != 0
    }

    /// Replaces the `state` half, leaving the flag bits alone; the C
    /// assignment to the 16-bit field.
    pub fn set_state(&mut self, state: u32) {
        self.0 = (self.0 & !Self::STATE_MASK) | (state & Self::STATE_MASK);
    }

    /// Sets or clears `wake_active`.
    pub fn set_wake_active(&mut self, active: bool) {
        if active {
            self.0 |= Self::WAKE_ACTIVE;
        } else {
            self.0 &= !Self::WAKE_ACTIVE;
        }
    }
}

/// The anonymous union of `struct thread` holding the bitfield word
/// and `event_key`.
///
/// The C side forms `TH_EV_WAKE_ACTIVE(t)` as the address of
/// `event_key`; the union's address is the word's address, so both
/// languages wake on the same key.
#[repr(C)]
pub union StateEvent {
    state: StateBits,
    event_key: *mut c_void,
}

/// `struct timer` of <kern/timer.h>: the statistical CPU timer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timer {
    /// `low_bits`: the microsecond count.
    pub low_bits: c_uint,
    /// `high_bits`: the seconds count.
    pub high_bits: c_uint,
    /// `high_bits_check`: a reader's copy of `high_bits`.
    pub high_bits_check: c_uint,
    /// `tstamp`: the last reading's timestamp.
    pub tstamp: c_uint,
}

/// `struct timer_save` of <kern/timer.h>: a saved timer reading.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerSave {
    /// `low`: the saved low half.
    pub low: c_uint,
    /// `high`: the saved high half.
    pub high: c_uint,
}

/// `struct time_value64` of <mach/time_value.h>: 64-bit seconds and
/// nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeValue64 {
    /// `seconds`.
    pub seconds: i64,
    /// `nanoseconds`.
    pub nanoseconds: i64,
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
    /// `option`: the receive options.
    pub option: c_int,
    /// `rcv_size`: the receive buffer size.
    pub rcv_size: c_uint,
    /// `timeout`: the receive timeout.
    pub timeout: c_uint,
    /// `notify`: the notification port name.
    pub notify: c_uint,
    /// `object`: the object being received from.
    pub object: *mut c_void,
    /// `mqueue`: the message queue.
    pub mqueue: *mut c_void,
}

/// The `exception` arm of `thread.saved`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SavedException {
    /// `port`: the exception port.
    pub port: *mut c_void,
    /// `exc`: the exception number.
    pub exc: c_int,
    /// `code`: the exception code.
    pub code: c_int,
    /// `subcode`: the exception subcode.
    pub subcode: c_long,
}

/// The `saved` union of `struct thread`: what the state selection
/// keeps if the thread's stack is discarded.
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
///
/// The mirror is complete: every C field has its Rust counterpart, in
/// declaration order, so `offset_of!` here is the C compiler's offset
/// and the `const` assertions below pin the total size.  Fields the
/// scheduler does not touch yet are documented as layout members.
#[repr(C)]
pub struct Thread {
    /// `links`: the run-queue or wait-queue links.
    pub links: QueueEntry,
    /// `runq`: the run queue the thread is on, or `RUN_QUEUE_NULL`.
    pub runq: *mut RunQueue,
    /// `task`: the task to which the thread belongs.
    pub task: *mut c_void,
    /// `thread_list`: the task's thread list.
    pub thread_list: QueueEntry,
    /// `state`, `wake_active`, `active` and `event_key`.
    pub state_event: StateEvent,
    /// `pset_threads`: the processor set's thread list.
    pub pset_threads: QueueEntry,
    /// `lock`: the thread lock, taken at splsched.
    pub lock: SimpleLock,
    /// `ref_count`: the number of references to the thread.
    pub ref_count: c_int,
    /// `pcb`: the machine-dependent process control block.
    pub pcb: *mut c_void,
    /// `kernel_stack`: accurate only when the thread is not swapped.
    pub kernel_stack: VmOffset,
    /// `stack_privilege`: the reserved kernel stack.
    pub stack_privilege: VmOffset,
    /// `swap_func`: where the thread starts after swap-in.
    pub swap_func: Continuation,
    /// `wait_event`: the event the thread is waiting on.
    pub wait_event: *mut c_void,
    /// `suspend_count`: internal use only.
    pub suspend_count: c_int,
    /// `wait_result`: the outcome of the wait.
    pub wait_result: c_int,
    /// `priority`: the base priority.
    pub priority: c_int,
    /// `max_priority`: the maximum priority.
    pub max_priority: c_int,
    /// `sched_pri`: the computed priority.
    pub sched_pri: c_int,
    /// `sched_data`: for use by the policy.
    pub sched_data: c_int,
    /// `policy`: the scheduling policy.
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
    /// `user_timer`: the user-mode timer.
    pub user_timer: Timer,
    /// `system_timer`: the system-mode timer.
    pub system_timer: Timer,
    /// `user_timer_save`: the saved user timer.
    pub user_timer_save: TimerSave,
    /// `system_timer_save`: the saved system timer.
    pub system_timer_save: TimerSave,
    /// `cpu_delta`: the CPU usage since the last update.
    pub cpu_delta: c_uint,
    /// `sched_delta`: the weighted CPU usage since the last update.
    pub sched_delta: c_uint,
    /// `creation_time`: the creation timestamp.
    pub creation_time: TimeValue64,
    /// `timer`: the wait timeout.
    pub timer: Timeout,
    /// `depress_timer`: the priority-depression timeout.
    pub depress_timer: Timeout,
    /// `ast`: the pending ASTs; see <kern/ast.h>.
    pub ast: c_int,
    /// `processor_set`: the assigned processor set.
    pub processor_set: *mut ProcessorSet,
    /// `bound_processor`: the processor the thread is bound to.
    pub bound_processor: *mut Processor,
    /// `may_assign`: whether assignment may change (MACH_HOST).
    pub may_assign: c_int,
    /// `assign_active`: someone waits for `may_assign` (MACH_HOST).
    pub assign_active: c_int,
    /// `last_processor`: the processor the thread last ran on.
    pub last_processor: *mut Processor,
    /// `name`: the thread's name.
    pub name: [c_char; TASK_NAME_SIZE],
}

impl Thread {
    /// The `state:16` half of the bitfield word.
    pub fn state(&self) -> u32 {
        // SAFETY: the `state` member shares the low word with
        // `event_key`, and every bit pattern is a valid `StateBits`.
        unsafe { self.state_event.state.state() }
    }

    /// Replaces the `state` half, as the C field assignment does.
    pub fn set_state(&mut self, state: u32) {
        // SAFETY: as `state()`, and the write only touches the low
        // word.
        unsafe { self.state_event.state.set_state(state) };
    }

    /// The `wake_active:1` bit.
    pub fn wake_active(&self) -> bool {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.wake_active() }
    }

    /// Sets or clears the `wake_active` bit.
    pub fn set_wake_active(&mut self, active: bool) {
        // SAFETY: as `state()`.
        unsafe { self.state_event.state.set_wake_active(active) };
    }

    /// The address of `event_key`, which is `TH_EV_WAKE_ACTIVE(t)` in
    /// C: the key a suspended thread's waker waits on.  Only the
    /// address is formed, never the value.
    pub fn wake_active_event(&self) -> *mut c_void {
        // Only the address is formed; the union is never read.
        (&raw const self.state_event.event_key)
            .cast::<c_void>()
            .cast_mut()
    }
}

// `struct thread` is `THREAD_SIZE` bytes on the target; the C compiler
// produced these numbers for the configured kernels.
const _: () = assert!(size_of::<Thread>() == THREAD_SIZE);
const _: () = assert!(align_of::<Thread>() == align_of::<*mut c_void>());

// `StateBits` is the C `unsigned`; the union adds the pointer-sized
// `event_key`, so its alignment is the pointer's.
const _: () = assert!(size_of::<StateBits>() == size_of::<u32>());
const _: () = assert!(
    size_of::<StateEvent>()
        == if size_of::<*mut c_void>() > size_of::<u32>() {
            size_of::<*mut c_void>()
        } else {
            size_of::<u32>()
        }
);

// The offsets the scheduler names, straight from the C compiler.
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

// The embedded records whose sizes the thread layout depends on.
const _: () = assert!(size_of::<Timer>() == 16);
const _: () = assert!(size_of::<TimerSave>() == 8);
const _: () = assert!(size_of::<TimeValue64>() == 16);
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
