// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The Mach queue package, which `kern/queue.c` used to define.
//!
//! A circular, doubly linked list whose head is a sentinel: an empty
//! queue's head points at itself.  `QueueEntry` is layout-identical to
//! `struct queue_entry` in <kern/queue.h>, so the C macros there and
//! the code here operate on the same queues.

use core::ffi::{c_int, c_void};
use core::ptr;
use core::ptr::NonNull;

/// A queue head or chain link, layout-identical to `struct
/// queue_entry` in <kern/queue.h>.
///
/// The list invariant: following `next` from the head walks the queue
/// and returns to the head, and every entry's `next` and `prev` point
/// at entries of the same list.
#[repr(C)]
pub struct QueueEntry {
    next: *mut QueueEntry,
    prev: *mut QueueEntry,
}

// The C struct is two pointers and nothing else; the layouts must
// agree or the queue.h macros would corrupt Rust-side queues.
const _: () =
    assert!(size_of::<QueueEntry>() == 2 * size_of::<*mut QueueEntry>());
const _: () =
    assert!(align_of::<QueueEntry>() == align_of::<*mut QueueEntry>());

impl QueueEntry {
    /// Self-link this entry, making it an empty queue head.
    /// `queue_init()` in C.
    ///
    /// # Safety
    ///
    /// The entry must not be linked into a queue -- its links are
    /// overwritten -- and must not be moved afterwards, because they
    /// point at its own address.
    pub unsafe fn init_head(&mut self) {
        let this: *mut QueueEntry = self;
        self.next = this;
        self.prev = this;
    }

    /// Whether this head's queue is empty.  `queue_empty()` in C.
    pub fn is_empty(&self) -> bool {
        ptr::eq(self.next, self)
    }

    /// The first entry, or `None` when the queue is empty.
    /// `queue_first()` in C.
    pub fn first(&self) -> Option<NonNull<QueueEntry>> {
        NonNull::new(self.next).filter(|p| !ptr::eq(p.as_ptr(), self))
    }

    /// The last entry, or `None` when the queue is empty.
    /// `queue_last()` in C.
    pub fn last(&self) -> Option<NonNull<QueueEntry>> {
        NonNull::new(self.prev).filter(|p| !ptr::eq(p.as_ptr(), self))
    }

    /// Insert `elt` at the front of the queue, right after this head.
    /// `enqueue_head()` in C.
    ///
    /// # Safety
    ///
    /// `elt` must be valid and not linked into a queue, and nothing
    /// else may access the queue during the call -- the C callers hold
    /// the queue's lock.
    pub unsafe fn push_front(&mut self, elt: NonNull<QueueEntry>) {
        let elt = elt.as_ptr();
        let this: *mut QueueEntry = self;
        // SAFETY: the list invariant puts a valid entry at
        // `(*this).next`, and the caller promises `elt` is valid and
        // unlinked.
        unsafe {
            (*elt).next = (*this).next;
            (*elt).prev = this;
            (*(*this).next).prev = elt;
            (*this).next = elt;
        }
    }

    /// Insert `elt` at the tail of the queue, just before this head.
    /// `enqueue_tail()` in C.
    ///
    /// # Safety
    ///
    /// Same contract as `push_front()`.
    pub unsafe fn push_back(&mut self, elt: NonNull<QueueEntry>) {
        let elt = elt.as_ptr();
        let this: *mut QueueEntry = self;
        // SAFETY: the list invariant puts a valid entry at
        // `(*this).prev`, and the caller promises `elt` is valid and
        // unlinked.
        unsafe {
            (*elt).next = this;
            (*elt).prev = (*this).prev;
            (*(*this).prev).next = elt;
            (*this).prev = elt;
        }
    }

    /// Remove and return the first entry, or `None` when the queue is
    /// empty.  `dequeue_head()` in C, including its quirk of leaving
    /// the removed entry's links stale.
    ///
    /// # Safety
    ///
    /// The queue must satisfy the list invariant, and nothing else may
    /// access it during the call.
    pub unsafe fn pop_front(&mut self) -> Option<NonNull<QueueEntry>> {
        let elt = self.first()?;
        let this: *mut QueueEntry = self;
        // SAFETY: the list invariant puts a valid entry after `elt`.
        let next = unsafe { (*elt.as_ptr()).next };
        // SAFETY: `next` is valid per the invariant.
        unsafe {
            (*next).prev = this;
            (*this).next = next;
        }
        Some(elt)
    }

    /// Remove and return the last entry, or `None` when the queue is
    /// empty.  `dequeue_tail()` in C, stale links included.
    ///
    /// # Safety
    ///
    /// Same contract as `pop_front()`.
    pub unsafe fn pop_back(&mut self) -> Option<NonNull<QueueEntry>> {
        let elt = self.last()?;
        let this: *mut QueueEntry = self;
        // SAFETY: the list invariant puts a valid entry before `elt`.
        let prev = unsafe { (*elt.as_ptr()).prev };
        // SAFETY: `prev` is valid per the invariant.
        unsafe {
            (*prev).next = this;
            (*this).prev = prev;
        }
        Some(elt)
    }

    /// Remove `elt` from whatever queue it is linked into.
    /// `remqueue()` in C: no membership check, and the removed entry's
    /// links are left stale.
    ///
    /// # Safety
    ///
    /// `elt` must be linked into a queue, and nothing else may access
    /// that queue during the call.
    pub unsafe fn remove(elt: NonNull<QueueEntry>) {
        let elt = elt.as_ptr();
        // SAFETY: the caller promises `elt` is linked into a queue, so
        // its links are valid entries of that list.
        unsafe {
            (*(*elt).next).prev = (*elt).prev;
            (*(*elt).prev).next = (*elt).next;
        }
    }

    /// Insert `elt` right after `pred` in its queue.  `insque()` in C.
    ///
    /// # Safety
    ///
    /// `pred` must be linked into a queue, `elt` must be valid and not
    /// linked, and nothing else may access the queue during the call.
    pub unsafe fn insert_after(
        pred: NonNull<QueueEntry>,
        elt: NonNull<QueueEntry>,
    ) {
        let pred = pred.as_ptr();
        let elt = elt.as_ptr();
        // SAFETY: the caller promises `pred` is linked, so
        // `(*pred).next` is valid, and `elt` is valid and unlinked.
        unsafe {
            (*elt).next = (*pred).next;
            (*elt).prev = pred;
            (*(*pred).next).prev = elt;
            (*pred).next = elt;
        }
    }

    /// Walk the queue from front to back.  `queue_iterate()` in C,
    /// with the same caveat: unlinking the yielded entry invalidates
    /// the iterator.
    pub fn iter(&self) -> QueueIter {
        QueueIter {
            head: NonNull::from(self),
            // SAFETY: the list invariant keeps `next` non-null: on an
            // empty queue it is the head itself.
            next: unsafe { NonNull::new_unchecked(self.next) },
        }
    }
}

/// An iterator over a queue, front to back; see `QueueEntry::iter()`.
pub struct QueueIter {
    head: NonNull<QueueEntry>,
    next: NonNull<QueueEntry>,
}

impl Iterator for QueueIter {
    type Item = NonNull<QueueEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.head {
            return None;
        }
        let cur = self.next;
        // SAFETY: the list invariant puts a valid entry after `cur`,
        // and the head itself is never null.
        self.next = unsafe { NonNull::new_unchecked(cur.as_ref().next) };
        Some(cur)
    }
}

/// Insert `elt` at the head of `que`.
///
/// # Safety
///
/// `que` must be an initialized queue head and `elt` a valid, unlinked
/// entry; nothing else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enqueue_head(
    que: *mut QueueEntry,
    elt: *mut QueueEntry,
) {
    // SAFETY: the caller promises both are valid and non-null.
    let (que, elt) = unsafe { (&mut *que, NonNull::new_unchecked(elt)) };
    // SAFETY: same as above.
    unsafe { que.push_front(elt) };
}

/// Insert `elt` at the tail of `que`.
///
/// # Safety
///
/// Same contract as `enqueue_head()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enqueue_tail(
    que: *mut QueueEntry,
    elt: *mut QueueEntry,
) {
    // SAFETY: the caller promises both are valid and non-null.
    let (que, elt) = unsafe { (&mut *que, NonNull::new_unchecked(elt)) };
    // SAFETY: same as above.
    unsafe { que.push_back(elt) };
}

/// Remove and return the head entry of `que`, or null when empty.
///
/// # Safety
///
/// `que` must be an initialized queue head, and nothing else may
/// access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dequeue_head(
    que: *mut QueueEntry,
) -> *mut QueueEntry {
    // SAFETY: the caller promises `que` is a valid head.
    let que = unsafe { &mut *que };
    // SAFETY: same as above.
    unsafe { que.pop_front() }.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// Remove and return the tail entry of `que`, or null when empty.
///
/// # Safety
///
/// Same contract as `dequeue_head()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dequeue_tail(
    que: *mut QueueEntry,
) -> *mut QueueEntry {
    // SAFETY: the caller promises `que` is a valid head.
    let que = unsafe { &mut *que };
    // SAFETY: same as above.
    unsafe { que.pop_back() }.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// Remove `elt` from its queue; `que` is unused, as in C.
///
/// # Safety
///
/// `elt` must be linked into a queue, and nothing else may access that
/// queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remqueue(
    _que: *mut QueueEntry,
    elt: *mut QueueEntry,
) {
    // SAFETY: the caller promises `elt` is linked into a queue.
    let elt = unsafe { NonNull::new_unchecked(elt) };
    // SAFETY: same as above.
    unsafe { QueueEntry::remove(elt) };
}

/// Insert `entry` right after `pred` in its queue.
///
/// # Safety
///
/// `pred` must be linked into a queue and `entry` valid and unlinked;
/// nothing else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn insque(
    entry: *mut QueueEntry,
    pred: *mut QueueEntry,
) {
    // SAFETY: the caller promises both are valid and non-null, with
    // `pred` linked.
    let (entry, pred) = unsafe {
        (NonNull::new_unchecked(entry), NonNull::new_unchecked(pred))
    };
    // SAFETY: same as above.
    unsafe { QueueEntry::insert_after(pred, entry) };
}

/// Initialize `q` as an empty queue head.  `queue_init()` in C.
///
/// # Safety
///
/// `q` must be valid and not linked into a queue, and must not be
/// moved afterwards: the links point at its own address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_init(q: *mut QueueEntry) {
    // SAFETY: the caller promises `q` is valid and unlinked.
    unsafe { (*q).init_head() };
}

/// The raw link after `q`: the first entry, or `q` itself when the
/// queue is empty -- unlike `QueueEntry::first()`, which maps the
/// sentinel to `None`.  `queue_first()` in C.
///
/// # Safety
///
/// `q` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_first(q: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `q` is a valid queue entry.
    unsafe { (*q).next }
}

/// The raw link after `qc`, with the same sentinel semantics as
/// `queue_first()`.  `queue_next()` in C.
///
/// # Safety
///
/// `qc` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_next(qc: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `qc` is a valid queue entry.
    unsafe { (*qc).next }
}

/// The raw link before `qc`, with the same sentinel semantics as
/// `queue_first()`.  `queue_prev()` in C.
///
/// # Safety
///
/// `qc` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_prev(qc: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `qc` is a valid queue entry.
    unsafe { (*qc).prev }
}

/// Whether `qe` is the head `q` itself, as a C boolean.
/// `queue_end()` in C.
///
/// # Safety
///
/// `q` and `qe` must be valid queue entries.  (Only addresses are
/// compared; nothing is dereferenced.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_end(
    q: *mut QueueEntry,
    qe: *mut QueueEntry,
) -> c_int {
    c_int::from(q == qe)
}

/// Whether `q`'s queue is empty, as a C boolean.  `queue_empty()` in C.
///
/// # Safety
///
/// `q` must be a valid queue head.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_empty(q: *mut QueueEntry) -> c_int {
    // SAFETY: the caller promises `q` is a valid queue head.
    c_int::from(unsafe { &*q }.is_empty())
}

/// The chain slot `off` bytes inside a container.
///
/// The generic `queue.h` macros store container pointers in the links,
/// so a neighbour's chain is found by adding the field offset to the
/// container address.
fn chain_of(container: *mut c_void, off: usize) -> *mut QueueEntry {
    container
        .cast::<u8>()
        .wrapping_add(off)
        .cast::<QueueEntry>()
}

/// Insert the container `elt` at the tail of `head`, chaining through
/// the `QueueEntry` field `off` bytes inside each container.  The
/// function form of the old `queue_enter()` macro: links store
/// container pointers, so `off` is how neighbours' chains are found.
///
/// # Safety
///
/// `head` must be an initialized queue head and `elt` a valid,
/// unlinked container with a `QueueEntry` at `off` bytes; every link
/// in the queue must be a container with the same offset (or the head
/// itself).  Nothing else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_enter_tail(
    head: *mut QueueEntry,
    elt: *mut c_void,
    off: usize,
) {
    let chain = chain_of(elt, off);
    // SAFETY: the caller promises `head` is a valid head and the queue
    // satisfies the container-links invariant for `off`.
    let prev = unsafe { (*head).prev };
    if prev == head {
        // SAFETY: as above.
        unsafe { (*head).next = elt.cast() };
    } else {
        // SAFETY: `prev` is a container whose chain is at `off`.
        unsafe { (*chain_of(prev.cast(), off)).next = elt.cast() };
    }
    // SAFETY: `chain` is the chain slot of the valid container `elt`.
    unsafe {
        (*chain).prev = prev;
        (*chain).next = head;
        (*head).prev = elt.cast();
    }
}

/// Insert the container `elt` at the head of `head`; the function form
/// of the old `queue_enter_first()` macro.  Same layout rules as
/// `queue_enter_tail()`.
///
/// # Safety
///
/// Same contract as `queue_enter_tail()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_enter_head(
    head: *mut QueueEntry,
    elt: *mut c_void,
    off: usize,
) {
    let chain = chain_of(elt, off);
    // SAFETY: the caller promises `head` is a valid head and the queue
    // satisfies the container-links invariant for `off`.
    let next = unsafe { (*head).next };
    if next == head {
        // SAFETY: as above.
        unsafe { (*head).prev = elt.cast() };
    } else {
        // SAFETY: `next` is a container whose chain is at `off`.
        unsafe { (*chain_of(next.cast(), off)).prev = elt.cast() };
    }
    // SAFETY: `chain` is the chain slot of the valid container `elt`.
    unsafe {
        (*chain).next = next;
        (*chain).prev = head;
        (*head).next = elt.cast();
    }
}

/// Remove the container `elt` from the queue headed by `head`; the
/// function form of the old `queue_remove()` macro.  No membership
/// check, like `remqueue()`.
///
/// # Safety
///
/// `elt` must be linked into `head`'s queue, with a `QueueEntry` at
/// `off` bytes, and every link in the queue must be a container with
/// the same offset (or the head).  Nothing else may access the queue
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_remove_generic(
    head: *mut QueueEntry,
    elt: *mut c_void,
    off: usize,
) {
    let chain = chain_of(elt, off);
    // SAFETY: the caller promises `elt` is linked into this queue.
    let (next, prev) = unsafe { ((*chain).next, (*chain).prev) };
    if next == head {
        // SAFETY: the caller promises `head` is a valid head.
        unsafe { (*head).prev = prev };
    } else {
        // SAFETY: `next` is a container whose chain is at `off`.
        unsafe { (*chain_of(next.cast(), off)).prev = prev };
    }
    if prev == head {
        // SAFETY: the caller promises `head` is a valid head.
        unsafe { (*head).next = next };
    } else {
        // SAFETY: `prev` is a container whose chain is at `off`.
        unsafe { (*chain_of(prev.cast(), off)).next = next };
    }
}
