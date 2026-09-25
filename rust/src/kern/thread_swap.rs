// SPDX-License-Identifier: CMU-Mach
// Derived from kern/thread_swap.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread swapper, which `kern/thread_swap.c` used to define and
//! `kern/thread_swap.h` declares.

use crate::arch::i386::percpu::current_thread;
use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_continue,
    thread_setrun, thread_wakeup_prim,
};
use crate::kern::thread::{
    TH_RUN, TH_SW_COMING_IN, TH_SWAP_STATE, TH_SWAPPED, Thread,
};
use core::ffi::{c_int, c_void};
use core::pin::Pin;
use core::ptr::NonNull;

/// `KERN_SUCCESS` in <mach/kern_return.h>.
const KERN_SUCCESS: c_int = 0;

/// `swapper_lock_data` of kern/thread_swap.c: guards `swapin_queue`.
static SWAPPER_LOCK: SimpleLock = SimpleLock::new();

/// `swapin_queue` of kern/thread_swap.c: the threads waiting for a stack, and
/// the event the swapin thread sleeps on.
#[unsafe(export_name = "swapin_queue")]
static mut SWAPIN_QUEUE: QueueEntry = QueueEntry::unlinked();

/// A pinned view of the swapin queue head.
///
/// # Safety
///
/// The caller must hold `SWAPPER_LOCK`, except during [`swapper_init()`], and
/// the returned reference must not outlive the critical section: the static
/// must not have two live `&mut` views.
unsafe fn swapin_queue<'a>() -> Pin<&'a mut QueueEntry> {
    // SAFETY: `SWAPIN_QUEUE` is a static, so it is valid, aligned and never
    // moves.
    unsafe {
        QueueEntry::pin_in_place(NonNull::new_unchecked(&raw mut SWAPIN_QUEUE))
    }
}

/// The swapin queue's address, which doubles as the wakeup event.
fn swapin_event() -> *mut c_void {
    (&raw mut SWAPIN_QUEUE).cast::<c_void>()
}

/// `swapper_init()` in C.
///
/// # Safety
///
/// Must be called once during boot, before any thread is queued for swapin;
/// `setup_main()` is the only caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swapper_init() {
    // SAFETY: the boot caller is single-threaded here, and `SWAPIN_QUEUE` is
    // an unlinked static.
    unsafe {
        swapin_queue().init_head();
    }
    SWAPPER_LOCK.init();
}

/// `thread_swapin()` in C.
///
/// # Safety
///
/// `thread` must be a live thread whose lock the caller holds, at splsched, as
/// the scheduler's swap path is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_swapin(thread: *mut Thread) {
    // SAFETY: the caller holds the thread lock, so the state read is stable
    // and `thread` is live and non-null.
    let state = unsafe { (*thread).state() };
    match state & TH_SWAP_STATE {
        TH_SWAPPED => {
            // SAFETY: as above; this is the C assignment to the 16-bit `state`
            // field.
            unsafe {
                (*thread)
                    .set_state((state & !TH_SWAP_STATE) | TH_SW_COMING_IN);
            }
            // SAFETY: the swapper lock serializes the queue, and the thread's
            // `links` field is free while the thread is swapped out.
            unsafe {
                SWAPPER_LOCK.lock();
                let links = NonNull::new_unchecked(&raw mut (*thread).links);
                swapin_queue().push_back(QueueEntry::pin_in_place(links));
                SWAPPER_LOCK.unlock();
            }
            // SAFETY: the event is the queue head's fixed address, the key the
            // swapin thread registers with `assert_wait()`.
            unsafe {
                thread_wakeup_prim(swapin_event(), 0, THREAD_AWAKENED);
            }
        }
        TH_SW_COMING_IN => (),
        _ => {
            // SAFETY: `Panic` halts the kernel and never returns; the
            // arguments are the C `panic()` macro's.
            unsafe {
                glue::Panic(
                    c"kern/thread_swap.c".as_ptr(),
                    line!() as c_int,
                    c"thread_swapin".as_ptr(),
                    c"thread_swapin".as_ptr(),
                )
            }
        }
    }
}

/// `thread_doswapin()` of kern/thread_swap.c, the body behind the adapter
/// below.
///
/// # Safety
///
/// `thread` must be a live thread with `TH_SWAP_STATE` set that no lock
/// protects, because the stack allocation can block; the caller must hold no
/// spin lock.
pub(crate) unsafe fn doswapin(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract; the Rust `stack_alloc()` may block and
    // resumes the thread through `thread_continue` once it has a stack.
    unsafe { (*thread).stack_alloc(Some(thread_continue)) };

    // SAFETY: `thread` is live and not locked; the spl level and the thread
    // lock guard the state and the run queue, in the C order.
    unsafe {
        let s = glue::splsched();
        (*thread).lock.lock();
        (*thread).set_state((*thread).state() & !TH_SWAP_STATE);
        if (*thread).state() & TH_RUN != 0 {
            thread_setrun(thread, c_int::from(true));
        }
        (*thread).lock.unlock();
        glue::splx(s);
    }
    KERN_SUCCESS
}

/// `thread_doswapin()` in C.
///
/// # Safety
///
/// `thread` must be a live thread queued for swapin, and the caller must hold
/// no spin lock, because the stack allocation can block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_doswapin(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { doswapin(thread) }
}

/// `swapin_thread_continue()` of kern/thread_swap.c, which C kept private.
///
/// # Safety
///
/// Runs as the swapin kernel thread.
unsafe extern "C" fn swapin_thread_continue() -> ! {
    loop {
        // SAFETY: the continuation runs in thread context; the swapper lock
        // and the spl level guard the queue, and `doswapin()` blocks only with
        // both released.
        unsafe {
            let mut s = glue::splsched();
            SWAPPER_LOCK.lock();

            while let Some(elt) = swapin_queue().pop_front() {
                SWAPPER_LOCK.unlock();
                glue::splx(s);

                // SAFETY: `links` is the first field of `struct thread`, so a
                // popped link is its thread; every entry on this queue was
                // pushed that way.
                let thread = elt.as_ptr().cast::<Thread>();
                let kr = doswapin(thread);

                s = glue::splsched();
                SWAPPER_LOCK.lock();

                if kr != KERN_SUCCESS {
                    // SAFETY: the failed `doswapin()` left the thread
                    // unqueued, so its links may go back on the queue.
                    swapin_queue().push_front(QueueEntry::pin_in_place(elt));
                    break;
                }
            }

            // SAFETY: the event is the queue head's fixed address, and the
            // lock is released before blocking, as in C.
            assert_wait(swapin_event(), 0);
            SWAPPER_LOCK.unlock();
            glue::splx(s);
            thread_block(Some(swapin_thread_continuation));
        }
    }
}

/// The `void (*)(void)` continuation `thread_block()` resumes.
///
/// # Safety
///
/// Runs as the swapin kernel thread's continuation after a block; it never
/// returns.
unsafe extern "C" fn swapin_thread_continuation() {
    // SAFETY: the swapin thread's own loop, which never returns.
    unsafe { swapin_thread_continue() }
}

/// `swapin_thread()` in C.
///
/// # Safety
///
/// Started by `kernel_thread()` as the "swapin" thread; it reserves its stack
/// and never returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swapin_thread() -> ! {
    // SAFETY: the kernel thread starts here with `current_thread()` pointing
    // at itself, and `stack_privilege()` reserves the stack it already runs
    // on.
    unsafe {
        let thread = current_thread();
        (*thread).vm_privilege = 1;
        (*thread).stack_privilege();
        swapin_thread_continue()
    }
}
