// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_thread.c and ipc/ipc_thread.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! IPC operations on threads, which `ipc/ipc_thread.c` used to define.
//!
//! An `IpcThreadQueue` is a LIFO stack of threads, not a FIFO queue:
//! `enqueue()` pushes at the front, so a thread that just ran is reused
//! early, which helps locality of reference (the C header's note).  The
//! links live in `struct thread` as `ith_next`/`ith_prev`, read from
//! the [`Thread`] mirror in [`crate::kern::thread`]; the queue itself
//! is Rust.
//!
//! A queue has no lock of its own: the caller holds the message-queue
//! or port lock that protects it.

use crate::kern::thread::Thread;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull};

/// `ipc_thread_t`: a reference to a thread, opaque to this module.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadRef(NonNull<c_void>);

/// `struct ipc_thread_queue`: a LIFO stack of threads.
#[repr(C)]
pub struct IpcThreadQueue {
    base: Option<ThreadRef>,
}

/// The `ith_next`/`ith_prev` pair inside `struct thread`, as one record.
/// `ThreadRef::links()` returns its address.
#[repr(C)]
struct ThreadLinks {
    next: Option<ThreadRef>,
    prev: Option<ThreadRef>,
}

// The C header defines `struct ipc_thread_queue` and embeds it, so the
// mirror must be one pointer; `ThreadLinks` is the mirror's adjacent
// `ith_next`/`ith_prev` pair.
const _: () = assert!(size_of::<IpcThreadQueue>() == size_of::<*mut c_void>());
const _: () =
    assert!(size_of::<IpcThreadQueue>() == size_of::<Option<ThreadRef>>());
const _: () =
    assert!(size_of::<ThreadLinks>() == 2 * size_of::<*mut c_void>());
const _: () = assert!(
    offset_of!(Thread, ith_prev)
        == offset_of!(Thread, ith_next) + size_of::<*mut Thread>()
);

impl ThreadRef {
    /// View a raw thread the caller promises is valid.
    ///
    /// # Safety
    ///
    /// `thread` must point at a valid `struct thread`.
    unsafe fn new(thread: *mut c_void) -> ThreadRef {
        // SAFETY: the caller promises a valid thread.
        ThreadRef(unsafe { NonNull::new_unchecked(thread) })
    }

    /// The raw thread pointer, for the C adapters.
    fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// The thread's IPC links.
    ///
    /// # Safety
    ///
    /// The thread must be valid.
    unsafe fn links(self) -> NonNull<ThreadLinks> {
        // SAFETY: the caller promises a valid thread, so the mirror's
        // ith_next/ith_prev pair is live and adjacent.
        unsafe {
            let thread = self.as_ptr().cast::<Thread>();
            let links = core::ptr::addr_of_mut!((*thread).ith_next);
            NonNull::new_unchecked(links.cast::<ThreadLinks>())
        }
    }

    /// Make the thread unlinked: both links point at itself.
    ///
    /// # Safety
    ///
    /// The thread must be valid, and it must not be linked in any
    /// queue.
    unsafe fn links_init(self) {
        // SAFETY: the caller promises a valid, unlinked thread.
        unsafe {
            let links = self.links();
            (*links.as_ptr()).next = Some(self);
            (*links.as_ptr()).prev = Some(self);
        }
    }
}

impl Default for IpcThreadQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl IpcThreadQueue {
    /// An empty queue; `ipc_thread_queue_init()` uses this.
    pub const fn new() -> Self {
        Self { base: None }
    }

    /// Empty the queue.  `ipc_thread_queue_init()` in C.
    pub fn init(&mut self) {
        *self = Self::new();
    }

    /// The first thread, if any.  `ipc_thread_queue_first()` in C.
    pub fn first(&self) -> Option<ThreadRef> {
        self.base
    }

    /// Push a thread at the front.  `ipc_thread_enqueue()` in C.
    ///
    /// # Safety
    ///
    /// The thread must be valid and not linked in any queue, and the
    /// caller must hold the lock protecting this queue.
    pub unsafe fn enqueue(&mut self, thread: ThreadRef) {
        let Some(first) = self.base else {
            self.base = Some(thread);
            return;
        };

        // SAFETY: the caller promises a valid, unlinked thread, and
        // the queue's first thread is linked.
        unsafe {
            let first_links = first.links();
            let last = (*first_links.as_ptr())
                .prev
                .expect("ipc_thread: linked thread without a predecessor");
            let links = thread.links();

            (*links.as_ptr()).next = Some(first);
            (*links.as_ptr()).prev = Some(last);
            (*first_links.as_ptr()).prev = Some(thread);
            let last_links = last.links();
            (*last_links.as_ptr()).next = Some(thread);
        }

        self.base = Some(thread);
    }

    /// Pop the first thread.  `ipc_thread_dequeue()` in C.
    ///
    /// # Safety
    ///
    /// The queued threads must be valid and linked, and the caller must
    /// hold the lock protecting this queue.
    pub unsafe fn dequeue(&mut self) -> Option<ThreadRef> {
        let first = self.first()?;
        // SAFETY: the caller promises a valid, linked thread.
        unsafe { self.rmqueue_first(first) };
        Some(first)
    }

    /// Remove an arbitrary thread.  `ipc_thread_rmqueue()` in C.
    ///
    /// # Safety
    ///
    /// `thread` must be linked in this queue, and the caller must hold
    /// the lock protecting it.
    pub unsafe fn rmqueue(&mut self, thread: ThreadRef) {
        // SAFETY: the caller promises a valid, linked thread.
        unsafe {
            let links = thread.links();
            let next = (*links.as_ptr())
                .next
                .expect("ipc_thread: linked thread without a successor");

            if next == thread {
                self.base = None;
                return;
            }

            let prev = (*links.as_ptr())
                .prev
                .expect("ipc_thread: linked thread without a predecessor");

            if self.base == Some(thread) {
                self.base = Some(next);
            }

            let next_links = next.links();
            (*next_links.as_ptr()).prev = Some(prev);
            let prev_links = prev.links();
            (*prev_links.as_ptr()).next = Some(next);
            thread.links_init();
        }
    }

    /// Remove the first thread.  `ipc_thread_rmqueue_first()` in C; the
    /// caller's macro used to assume `thread` was the first.
    ///
    /// # Safety
    ///
    /// `thread` must be the first thread of this queue, and the caller
    /// must hold the lock protecting it.
    pub unsafe fn rmqueue_first(&mut self, thread: ThreadRef) {
        // SAFETY: the caller promises a valid, linked thread.
        unsafe {
            let links = thread.links();
            let next = (*links.as_ptr())
                .next
                .expect("ipc_thread: linked thread without a successor");

            if next == thread {
                self.base = None;
                return;
            }

            let prev = (*links.as_ptr())
                .prev
                .expect("ipc_thread: linked thread without a predecessor");

            self.base = Some(next);
            let next_links = next.links();
            (*next_links.as_ptr()).prev = Some(prev);
            let prev_links = prev.links();
            (*prev_links.as_ptr()).next = Some(next);
            thread.links_init();
        }
    }
}

/// Enqueue a thread.  `ipc_thread_enqueue()` in C.
///
/// # Safety
///
/// `queue` must be valid, `thread` valid and unlinked, and the caller
/// must hold the lock protecting the queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_enqueue(
    queue: *mut IpcThreadQueue,
    thread: *mut c_void,
) {
    // SAFETY: the caller promises valid pointers.
    unsafe { (*queue).enqueue(ThreadRef::new(thread)) };
}

/// Dequeue a thread.  `ipc_thread_dequeue()` in C.
///
/// # Safety
///
/// `queue` must be valid and its threads linked, and the caller must
/// hold the lock protecting it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_dequeue(
    queue: *mut IpcThreadQueue,
) -> *mut c_void {
    // SAFETY: the caller promises a valid queue.
    let thread = unsafe { (*queue).dequeue() };
    thread.map_or(ptr::null_mut(), ThreadRef::as_ptr)
}

/// Remove a thread.  `ipc_thread_rmqueue()` in C.
///
/// # Safety
///
/// `queue` must be valid, `thread` linked in it, and the caller must
/// hold the lock protecting it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_rmqueue(
    queue: *mut IpcThreadQueue,
    thread: *mut c_void,
) {
    // SAFETY: the caller promises valid pointers.
    unsafe { (*queue).rmqueue(ThreadRef::new(thread)) };
}

/// Make a thread unlinked.  `ipc_thread_links_init()` in C.
///
/// # Safety
///
/// `thread` must be valid and not linked in any queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_links_init(thread: *mut c_void) {
    // SAFETY: the caller promises a valid, unlinked thread.
    unsafe { ThreadRef::new(thread).links_init() };
}

/// Empty a queue.  `ipc_thread_queue_init()` in C.
///
/// # Safety
///
/// `queue` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_queue_init(queue: *mut IpcThreadQueue) {
    // SAFETY: the caller promises a valid queue.
    unsafe { (*queue).init() };
}

/// The first thread of a queue.  `ipc_thread_queue_first()` in C.
///
/// # Safety
///
/// `queue` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_queue_first(
    queue: *mut IpcThreadQueue,
) -> *mut c_void {
    // SAFETY: the caller promises a valid queue.
    let thread = unsafe { (*queue).first() };
    thread.map_or(ptr::null_mut(), ThreadRef::as_ptr)
}

/// Remove the first thread of a queue.  `ipc_thread_rmqueue_first()`
/// in C, where it was `ipc_thread_rmqueue_first_macro()`.
///
/// # Safety
///
/// `queue` must be valid, `thread` its first thread, and the caller
/// must hold the lock protecting it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_rmqueue_first(
    queue: *mut IpcThreadQueue,
    thread: *mut c_void,
) {
    // SAFETY: the caller promises valid pointers.
    unsafe { (*queue).rmqueue_first(ThreadRef::new(thread)) };
}
