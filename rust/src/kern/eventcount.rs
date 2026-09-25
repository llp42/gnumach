// SPDX-License-Identifier: CMU-Mach
// Derived from kern/eventcount.c and kern/eventcount.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Eventcounters, which `kern/eventcount.c` used to define for
//! `kern/eventcount.h`.

use crate::arch::i386::percpu::current_thread;
use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::sched_prim::{assert_wait, thread_block, thread_setrun};
use crate::kern::thread::{
    TH_RUN, TH_SCHED_STATE, TH_SUSP, TH_UNINT, TH_WAIT, Thread,
};
use crate::kern::types::KernError;
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{c_int, c_uint};
use core::mem::offset_of;
use core::ptr::{self, NonNull};

/// `MAX_EVCS` in kern/eventcount.c: the eventcounter table's length.
const MAX_EVCS: usize = 10;

/// The `TH_*` combinations the `evc_signal()` state switch names, spelled as
/// single constants because a `|` in a pattern is an or-pattern.
const WAIT: u32 = TH_WAIT;
const WAIT_UNINT: u32 = TH_WAIT | TH_UNINT;
const WAIT_SUSP: u32 = TH_WAIT | TH_SUSP;
const WAIT_SUSP_UNINT: u32 = TH_WAIT | TH_SUSP | TH_UNINT;
const RUN_WAIT: u32 = TH_RUN | TH_WAIT;
const RUN_WAIT_SUSP: u32 = TH_RUN | TH_WAIT | TH_SUSP;
const RUN_WAIT_UNINT: u32 = TH_RUN | TH_WAIT | TH_UNINT;
const RUN_WAIT_SUSP_UNINT: u32 = TH_RUN | TH_WAIT | TH_SUSP | TH_UNINT;

/// `struct evc` of <kern/eventcount.h>: one eventcounter.
#[repr(C)]
pub struct EventCounter {
    /// `count`: pending events, or `-1` while a waiter blocks.
    pub count: c_int,
    pub waiting_thread: *mut Thread,
    /// `ev_id`: the table index.
    pub ev_id: c_uint,
    /// `sanity`: the counter's own address, or null once destroyed.
    pub sanity: *mut EventCounter,
    pub lock: SimpleLock,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<EventCounter>() == 40);
    assert!(align_of::<EventCounter>() == align_of::<*mut EventCounter>());
    assert!(offset_of!(EventCounter, count) == 0);
    assert!(offset_of!(EventCounter, waiting_thread) == 8);
    assert!(offset_of!(EventCounter, ev_id) == 16);
    assert!(offset_of!(EventCounter, sanity) == 24);
    assert!(offset_of!(EventCounter, lock) == 32);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<EventCounter>() == 20);
    assert!(align_of::<EventCounter>() == align_of::<*mut EventCounter>());
    assert!(offset_of!(EventCounter, count) == 0);
    assert!(offset_of!(EventCounter, waiting_thread) == 4);
    assert!(offset_of!(EventCounter, ev_id) == 8);
    assert!(offset_of!(EventCounter, sanity) == 12);
    assert!(offset_of!(EventCounter, lock) == 16);
};

/// `all_eventcounters[MAX_EVCS]` of kern/eventcount.c.
static ALL_EVENTCOUNTERS: SyncCell<[*mut EventCounter; MAX_EVCS]> =
    SyncCell(UnsafeCell::new([ptr::null_mut(); MAX_EVCS]));

/// The table index `id` names, or `None` when it is out of range.
fn slot_of(id: c_uint) -> Option<usize> {
    usize::try_from(id).ok().filter(|index| *index < MAX_EVCS)
}

/// The live counter `ev_id` names, or `None` when it is not registered.
fn counter(ev_id: c_uint) -> Option<NonNull<EventCounter>> {
    let index = slot_of(ev_id)?;
    // SAFETY: the table is written by the boot's single-threaded init path
    // and by `destroy()`, which clears the slot before the counter goes away.
    let ev = unsafe { (*ALL_EVENTCOUNTERS.0.get())[index] };
    let ev = NonNull::new(ev)?;
    // SAFETY: the slot is non-null, and `init()` stored both words before the
    // counter became reachable.
    if unsafe { (*ev.as_ptr()).ev_id } != ev_id
        || unsafe { (*ev.as_ptr()).sanity } != ev.as_ptr()
    {
        return None;
    }
    Some(ev)
}

/// `evc_continue()` of kern/eventcount.c: give the blocked waiter the stack
/// back with `KERN_SUCCESS` as the syscall answer.
unsafe extern "C" fn evc_continue() {
    // SAFETY: `thread_syscall_return()` never returns, and the C passed
    // `KERN_SUCCESS`.
    unsafe { glue::thread_syscall_return(0) }
}

/// `evc_init()` of kern/eventcount.c: zero a counter and register it.
///
/// # Safety
///
/// `ev` must point at writable storage for an [`EventCounter`] that no other
/// thread can see yet.
pub(crate) unsafe fn init(ev: *mut EventCounter) {
    // SAFETY: the caller promises writable, unshared counter storage.
    unsafe {
        (*ev).count = 0;
        (*ev).waiting_thread = ptr::null_mut();
        (*ev).ev_id = 0;
        (*ev).sanity = ptr::null_mut();
        (*ev).lock = SimpleLock::new();
    }

    let table = ALL_EVENTCOUNTERS.0.get();
    // SAFETY: the C registered counters from a single-threaded init path, and
    // every index below `MAX_EVCS` is in bounds.
    unsafe {
        let mut free = None;
        for i in 0..MAX_EVCS {
            if (*table)[i].is_null() {
                free = Some(i);
                break;
            }
        }
        let Some(index) = free else {
            glue::printf(c"Too many eventcounters\n".as_ptr());
            return;
        };
        let Ok(id) = c_uint::try_from(index) else {
            return;
        };

        (*table)[index] = ev;
        (*ev).ev_id = id;
        (*ev).sanity = ev;
    }
}

/// `evc_destroy()` of kern/eventcount.c: signal the counter and drop it from
/// the table.
///
/// # Safety
///
/// `ev` must be a live counter registered by [`init()`].
pub(crate) unsafe fn destroy(ev: *mut EventCounter) {
    // SAFETY: the caller promises a live counter.
    unsafe { signal(ev) };

    // SAFETY: as above; the slot and the sanity word are cleared before the
    // counter goes away, as the C did.
    unsafe {
        (*ev).sanity = ptr::null_mut();
        if let Some(index) = slot_of((*ev).ev_id) {
            let table = ALL_EVENTCOUNTERS.0.get();
            if (*table)[index] == ev {
                (*table)[index] = ptr::null_mut();
            }
        }
        (*ev).ev_id = c_uint::MAX;
    }
}

/// `evc_notify_abort()` of kern/eventcount.c: let go of a dying waiter.
///
/// # Safety
///
/// `thread` must be the live thread the C thread-termination path is about to
/// take off the wait queues.
pub(crate) unsafe fn notify_abort(thread: *mut Thread) {
    // SAFETY: the caller promises a live thread; the C walked the whole table
    // at splsched and took each counter's lock.
    unsafe {
        let s = glue::splsched();
        let table = ALL_EVENTCOUNTERS.0.get();

        for i in 0..MAX_EVCS {
            let ev = (*table)[i];
            if ev.is_null() {
                continue;
            }

            (*ev).lock.lock();
            if (*ev).waiting_thread == thread {
                (*ev).waiting_thread = ptr::null_mut();
                // Removing a waiting thread has to bump the count by one.
                (*ev).count = (*ev).count.wrapping_add(1);
            }
            (*ev).lock.unlock();
        }

        glue::splx(s);
    }
}

/// `evc_wait()` of kern/eventcount.c.
pub(crate) fn wait(ev_id: c_uint) -> Result<(), KernError> {
    let Some(ev) = counter(ev_id) else {
        return Err(KernError::InvalidArgument);
    };

    // SAFETY: `counter()` returned a registered counter; the C took the
    // counter lock at splsched.
    unsafe {
        let s = glue::splsched();
        (*ev.as_ptr()).lock.lock();

        if (*ev.as_ptr()).count > 0 {
            (*ev.as_ptr()).count = (*ev.as_ptr()).count.wrapping_sub(1);
            (*ev.as_ptr()).lock.unlock();
            glue::splx(s);
            return Ok(());
        }

        if (*ev.as_ptr()).waiting_thread.is_null() {
            (*ev.as_ptr()).count = (*ev.as_ptr()).count.wrapping_sub(1);
            (*ev.as_ptr()).waiting_thread = current_thread();
            assert_wait(ptr::null_mut(), 1);
            (*ev.as_ptr()).lock.unlock();
            thread_block(Some(evc_continue));
            return Ok(());
        }

        (*ev.as_ptr()).lock.unlock();
        glue::splx(s);
        Err(KernError::NoSpace)
    }
}

/// `evc_wait_clear()` of kern/eventcount.c: clear the count before blocking.
pub(crate) fn wait_clear(ev_id: c_uint) -> Result<(), KernError> {
    let Some(ev) = counter(ev_id) else {
        return Err(KernError::InvalidArgument);
    };

    // SAFETY: `counter()` returned a registered counter; the C took the
    // counter lock at splsched.
    unsafe {
        let s = glue::splsched();
        (*ev.as_ptr()).lock.lock();

        if (*ev.as_ptr()).waiting_thread.is_null() {
            (*ev.as_ptr()).count = -1;
            (*ev.as_ptr()).waiting_thread = current_thread();
            assert_wait(ptr::null_mut(), 1);
            (*ev.as_ptr()).lock.unlock();
            thread_block(Some(evc_continue));
            return Ok(());
        }

        (*ev.as_ptr()).lock.unlock();
        glue::splx(s);
        Err(KernError::NoSpace)
    }
}

/// `evc_signal()` of kern/eventcount.c: wake a waiter, or record the event.
///
/// # Safety
///
/// `ev` must be null or point at a live counter registered by [`init()`].
pub(crate) unsafe fn signal(ev: *mut EventCounter) {
    // SAFETY: the caller promises a live counter; the sanity word is what the
    // C checked first.
    if unsafe { (*ev).sanity } != ev {
        return;
    }

    // SAFETY: the counter is live; the C took the counter lock at splsched and
    // then the waiting thread's lock.  The C's pre-lock `lock_data` spin is
    // the test-and-test-and-set loop `SimpleLock::lock()` already runs.
    unsafe {
        let s = glue::splsched();
        (*ev).lock.lock();
        (*ev).count = (*ev).count.wrapping_add(1);

        let thread = (*ev).waiting_thread;
        if !thread.is_null() {
            (*ev).waiting_thread = ptr::null_mut();

            loop {
                (*thread).lock.lock();
                let state = (*thread).state();
                match state & TH_SCHED_STATE {
                    WAIT_SUSP_UNINT | WAIT_UNINT | WAIT => {
                        (*thread).set_state((state & !TH_WAIT) | TH_RUN);
                        thread_setrun(thread, 1);
                        (*thread).lock.unlock();
                        break;
                    }
                    RUN_WAIT => {
                        (*thread).lock.unlock();
                        continue;
                    }
                    WAIT_SUSP | RUN_WAIT_SUSP | RUN_WAIT_UNINT
                    | RUN_WAIT_SUSP_UNINT => {
                        (*thread).set_state(state & !TH_WAIT);
                        (*thread).lock.unlock();
                        break;
                    }
                    _ => {
                        // SAFETY: `Panic` does not return; the message and the
                        // function tag are the C `panic()` call's.
                        glue::Panic(
                            c"kern/eventcount.c".as_ptr(),
                            line!() as c_int,
                            c"evc_signal".as_ptr(),
                            c"evc_signal.3".as_ptr(),
                        );
                    }
                }
            }
        }

        (*ev).lock.unlock();
        glue::splx(s);
    }
}
