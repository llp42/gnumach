// SPDX-License-Identifier: CMU-Mach
// Derived from kern/queue.c and kern/queue.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The Mach queue package, which `kern/queue.c` used to define.

use core::ffi::{c_int, c_void};
use core::marker::{PhantomData, PhantomPinned};
use core::pin::Pin;
use core::ptr;
use core::ptr::NonNull;

/// A queue head or chain link, layout-identical to `struct queue_entry` in
/// <kern/queue.h>.
#[repr(C)]
pub struct QueueEntry {
    next: *mut QueueEntry,
    prev: *mut QueueEntry,
    _pin: PhantomPinned,
}

const _: () =
    assert!(size_of::<QueueEntry>() == 2 * size_of::<*mut QueueEntry>());
const _: () =
    assert!(align_of::<QueueEntry>() == align_of::<*mut QueueEntry>());
const _: () = assert!(core::mem::offset_of!(QueueEntry, next) == 0);
const _: () = assert!(
    core::mem::offset_of!(QueueEntry, prev) == size_of::<*mut QueueEntry>()
);

impl QueueEntry {
    /// An unlinked entry with null links: the image a C `static` began with,
    /// for storage whose head is `init_head()`ed later.
    pub(crate) const fn unlinked() -> Self {
        Self {
            next: ptr::null_mut(),
            prev: ptr::null_mut(),
            _pin: PhantomPinned,
        }
    }

    /// Views an already-allocated entry as pinned.
    ///
    /// # Safety
    ///
    /// `this` must point at a valid, aligned `QueueEntry`, and its storage
    /// must remain at that address for as long as it is linked into a queue:
    /// the queue's links point at it.
    pub unsafe fn pin_in_place<'a>(
        this: NonNull<QueueEntry>,
    ) -> Pin<&'a mut QueueEntry> {
        // SAFETY: the caller promises validity and address stability.
        unsafe { Pin::new_unchecked(&mut *this.as_ptr()) }
    }

    /// `queue_init()` in C.
    ///
    /// # Safety
    ///
    /// The entry must not be linked into a queue -- its links are overwritten.
    pub unsafe fn init_head(self: Pin<&mut Self>) {
        // SAFETY: the body only writes the link fields; the entry is never
        // moved out of the pin.
        let this = unsafe { self.get_unchecked_mut() };
        let this_ptr: *mut QueueEntry = this;
        this.next = this_ptr;
        this.prev = this_ptr;
        // SAFETY: an empty head is self-linked.
        unsafe { check_head(this_ptr) };
    }

    /// `queue_empty()` in C.
    pub fn is_empty(&self) -> bool {
        ptr::eq(self.next, self)
    }

    /// Whether the entry was initialized: a zeroed C `static` has null links
    /// until `queue_init()` sets them.
    pub fn is_initialized(&self) -> bool {
        !self.next.is_null()
    }

    /// `queue_first()` in C.
    pub fn first(&self) -> Option<NonNull<QueueEntry>> {
        NonNull::new(self.next).filter(|p| !ptr::eq(p.as_ptr(), self))
    }

    /// `queue_last()` in C.
    pub fn last(&self) -> Option<NonNull<QueueEntry>> {
        NonNull::new(self.prev).filter(|p| !ptr::eq(p.as_ptr(), self))
    }

    /// `enqueue_head()` in C.
    ///
    /// # Safety
    ///
    /// `elt` must be valid and not linked into a queue, and nothing else may
    /// access the queue during the call -- the C callers hold the queue's
    /// lock.
    pub unsafe fn push_front(self: Pin<&mut Self>, elt: Pin<&mut QueueEntry>) {
        // SAFETY: the body only writes link fields; neither entry is moved out
        // of its pin.
        let this: *mut QueueEntry = unsafe { self.get_unchecked_mut() };
        // SAFETY: as above.
        let elt: *mut QueueEntry = unsafe { elt.get_unchecked_mut() };
        // SAFETY: the head of an initialized queue is self-consistent.
        unsafe { check_head(this) };
        // SAFETY: the list invariant puts a valid entry at `(*this).next`, and
        // the caller promises `elt` is valid and unlinked.
        unsafe {
            (*elt).next = (*this).next;
            (*elt).prev = this;
            (*(*this).next).prev = elt;
            (*this).next = elt;
        }
        // SAFETY: the insertion keeps the head self-consistent.
        unsafe { check_head(this) };
    }

    /// `enqueue_tail()` in C.
    ///
    /// # Safety
    ///
    /// Same contract as `push_front()`.
    pub unsafe fn push_back(self: Pin<&mut Self>, elt: Pin<&mut QueueEntry>) {
        // SAFETY: the body only writes link fields; neither entry is moved out
        // of its pin.
        let this: *mut QueueEntry = unsafe { self.get_unchecked_mut() };
        // SAFETY: as above.
        let elt: *mut QueueEntry = unsafe { elt.get_unchecked_mut() };
        // SAFETY: the head of an initialized queue is self-consistent.
        unsafe { check_head(this) };
        // SAFETY: the list invariant puts a valid entry at `(*this).prev`, and
        // the caller promises `elt` is valid and unlinked.
        unsafe {
            (*elt).next = this;
            (*elt).prev = (*this).prev;
            (*(*this).prev).next = elt;
            (*this).prev = elt;
        }
        // SAFETY: the insertion keeps the head self-consistent.
        unsafe { check_head(this) };
    }

    /// `dequeue_head()` in C, including its quirk of leaving the removed
    /// entry's links stale.
    ///
    /// # Safety
    ///
    /// The queue must satisfy the list invariant, and nothing else may access
    /// it during the call.
    pub unsafe fn pop_front(
        self: Pin<&mut Self>,
    ) -> Option<NonNull<QueueEntry>> {
        let elt = self.first()?;
        // SAFETY: the body only writes link fields; the head is never moved
        // out of the pin.
        let this: *mut QueueEntry = unsafe { self.get_unchecked_mut() };
        // SAFETY: the head of an initialized queue is self-consistent.
        unsafe { check_head(this) };
        // SAFETY: the list invariant puts a valid entry after `elt`.
        let next = unsafe { (*elt.as_ptr()).next };
        // SAFETY: `next` is valid per the invariant.
        unsafe {
            (*next).prev = this;
            (*this).next = next;
        }
        // SAFETY: the removal keeps the head self-consistent.
        unsafe { check_head(this) };
        Some(elt)
    }

    /// Remove and return the last entry, or `None` when the queue is empty.
    ///
    /// # Safety
    ///
    /// Same contract as `pop_front()`.
    pub unsafe fn pop_back(
        self: Pin<&mut Self>,
    ) -> Option<NonNull<QueueEntry>> {
        let elt = self.last()?;
        // SAFETY: the body only writes link fields; the head is never moved
        // out of the pin.
        let this: *mut QueueEntry = unsafe { self.get_unchecked_mut() };
        // SAFETY: the head of an initialized queue is self-consistent.
        unsafe { check_head(this) };
        // SAFETY: the list invariant puts a valid entry before `elt`.
        let prev = unsafe { (*elt.as_ptr()).prev };
        // SAFETY: `prev` is valid per the invariant.
        unsafe {
            (*prev).next = this;
            (*this).prev = prev;
        }
        // SAFETY: the removal keeps the head self-consistent.
        unsafe { check_head(this) };
        Some(elt)
    }

    /// `remqueue()` in C: no membership check, and the removed entry's links
    /// are left stale.
    ///
    /// # Safety
    ///
    /// `elt` must be linked into a queue, and nothing else may access that
    /// queue during the call.
    pub unsafe fn remove(elt: Pin<&mut QueueEntry>) {
        // SAFETY: the body only writes link fields; `elt` is never moved out
        // of the pin.
        let elt: *mut QueueEntry = unsafe { elt.get_unchecked_mut() };
        // SAFETY: the caller promises `elt` is linked, and the queue it is
        // linked into is self-consistent.
        unsafe { check_linked(elt) };
        // SAFETY: the caller promises `elt` is linked into a queue, so its
        // links are valid entries of that list.
        unsafe {
            (*(*elt).next).prev = (*elt).prev;
            (*(*elt).prev).next = (*elt).next;
        }
    }

    /// Insert `elt` right after `pred` in its queue, as the old `insque()`
    /// did.
    ///
    /// # Safety
    ///
    /// `pred` must be linked into a queue, `elt` must be valid and not linked,
    /// and nothing else may access the queue during the call.
    pub unsafe fn insert_after(
        pred: Pin<&mut QueueEntry>,
        elt: Pin<&mut QueueEntry>,
    ) {
        // SAFETY: the body only writes link fields; neither entry is moved out
        // of its pin.
        let pred: *mut QueueEntry = unsafe { pred.get_unchecked_mut() };
        // SAFETY: as above.
        let elt: *mut QueueEntry = unsafe { elt.get_unchecked_mut() };
        // SAFETY: the caller promises `pred` is linked.
        unsafe { check_linked(pred) };
        // SAFETY: the caller promises `pred` is linked, so `(*pred).next` is
        // valid, and `elt` is valid and unlinked.
        unsafe {
            (*elt).next = (*pred).next;
            (*elt).prev = pred;
            (*(*pred).next).prev = elt;
            (*pred).next = elt;
        }
        // SAFETY: `elt` is now linked into `pred`'s queue.
        unsafe { check_linked(elt) };
    }

    /// `queue_iterate()` in C, with the same caveat: unlinking the yielded
    /// entry invalidates the iterator.
    pub fn iter(&self) -> QueueIter<'_> {
        QueueIter {
            head: NonNull::from(self),
            // SAFETY: the list invariant keeps `next` non-null: on an empty
            // queue it is the head itself.
            next: unsafe { NonNull::new_unchecked(self.next) },
            _marker: PhantomData,
        }
    }
}

/// Compile-time-selected queue invariant checks; see `queue_debug`.
#[cfg(queue_debug)]
unsafe fn check_head(head: *const QueueEntry) {
    // SAFETY: the caller promises `head` is an initialized queue head and that
    // nothing else mutates the queue concurrently.
    unsafe {
        let next = (*head).next;
        let prev = (*head).prev;
        assert!(core::ptr::eq((*next).prev, head), "queue: head->next->prev");
        assert!(core::ptr::eq((*prev).next, head), "queue: head->prev->next");
    }
}

#[cfg(not(queue_debug))]
unsafe fn check_head(_head: *const QueueEntry) {}

#[cfg(queue_debug)]
unsafe fn check_linked(elt: *const QueueEntry) {
    // SAFETY: the caller promises `elt` is linked and that nothing else
    // mutates its queue concurrently.
    unsafe {
        let next = (*elt).next;
        let prev = (*elt).prev;
        assert!(core::ptr::eq((*next).prev, elt), "queue: elt->next->prev");
        assert!(core::ptr::eq((*prev).next, elt), "queue: elt->prev->next");
    }
}

#[cfg(not(queue_debug))]
unsafe fn check_linked(_elt: *const QueueEntry) {}

#[cfg(queue_debug)]
unsafe fn check_chain(head: *const QueueEntry, off: usize) {
    // SAFETY: the caller promises `head` is the initialized head of a queue
    // whose links are the chain field `off` bytes inside each container, and
    // that nothing else mutates it concurrently.
    unsafe {
        let this = head.cast_mut();
        let next = (*this).next;
        let prev = (*this).prev;
        if next == this {
            assert!(prev == this, "queue: empty chain head");
        } else {
            assert!(
                core::ptr::eq((*chain_of(next.cast(), off)).prev, head),
                "queue: head->next chain->prev"
            );
        }
        if prev == this {
            assert!(next == this, "queue: empty chain head");
        } else {
            assert!(
                core::ptr::eq((*chain_of(prev.cast(), off)).next, head),
                "queue: head->prev chain->next"
            );
        }
    }
}

#[cfg(not(queue_debug))]
unsafe fn check_chain(_head: *const QueueEntry, _off: usize) {}

/// An iterator over a queue, front to back; see `QueueEntry::iter()`.
pub struct QueueIter<'a> {
    head: NonNull<QueueEntry>,
    next: NonNull<QueueEntry>,
    _marker: PhantomData<&'a QueueEntry>,
}

impl<'a> Iterator for QueueIter<'a> {
    type Item = NonNull<QueueEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.head {
            return None;
        }
        let cur = self.next;
        // SAFETY: the list invariant puts a valid entry after `cur`, and the
        // head itself is never null.
        self.next = unsafe { NonNull::new_unchecked(cur.as_ref().next) };
        Some(cur)
    }
}

/// Insert `elt` at the head of `que`.
///
/// # Safety
///
/// `que` must be an initialized queue head and `elt` a valid, unlinked entry;
/// nothing else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enqueue_head(
    que: *mut QueueEntry,
    elt: *mut QueueEntry,
) {
    // SAFETY: the caller promises both are valid, non-null, and stable.
    let que = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(que)) };
    // SAFETY: as above.
    let elt = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(elt)) };
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
    // SAFETY: the caller promises both are valid, non-null, and stable.
    let que = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(que)) };
    // SAFETY: as above.
    let elt = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(elt)) };
    // SAFETY: same as above.
    unsafe { que.push_back(elt) };
}

/// Remove and return the head entry of `que`, or null when empty.
///
/// # Safety
///
/// `que` must be an initialized queue head that stays at its address while its
/// entries are linked, and nothing else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dequeue_head(
    que: *mut QueueEntry,
) -> *mut QueueEntry {
    // SAFETY: the caller promises `que` is a valid, stable head.
    let que = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(que)) };
    // SAFETY: same as above.
    unsafe { que.pop_front() }.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// Remove `elt` from its queue; `que` is unused, as in C.
///
/// # Safety
///
/// `elt` must be linked into a queue that stays at its address while linked,
/// and nothing else may access that queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remqueue(
    _que: *mut QueueEntry,
    elt: *mut QueueEntry,
) {
    // SAFETY: the caller promises `elt` is linked and stable.
    let elt = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(elt)) };
    // SAFETY: same as above.
    unsafe { QueueEntry::remove(elt) };
}

/// `queue_init()` in C.
///
/// # Safety
///
/// `q` must be valid, must stay at its address while linked, and must not be
/// linked into a queue: the links are overwritten.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_init(q: *mut QueueEntry) {
    // SAFETY: the caller promises `q` is valid, stable, and unlinked.
    let q = unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(q)) };
    // SAFETY: same as above.
    unsafe { q.init_head() };
}

/// `queue_first()` in C.
///
/// # Safety
///
/// `q` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_first(q: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `q` is a valid queue entry.
    unsafe { (*q).next }
}

/// `queue_next()` in C.
///
/// # Safety
///
/// `qc` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_next(qc: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `qc` is a valid queue entry.
    unsafe { (*qc).next }
}

/// `queue_prev()` in C.
///
/// # Safety
///
/// `qc` must be a valid queue entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_prev(qc: *mut QueueEntry) -> *mut QueueEntry {
    // SAFETY: the caller promises `qc` is a valid queue entry.
    unsafe { (*qc).prev }
}

/// `queue_end()` in C.
///
/// # Safety
///
/// `q` and `qe` must be valid queue entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_end(
    q: *mut QueueEntry,
    qe: *mut QueueEntry,
) -> c_int {
    c_int::from(q == qe)
}

/// `queue_empty()` in C.
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
fn chain_of(container: *mut c_void, off: usize) -> *mut QueueEntry {
    container
        .cast::<u8>()
        .wrapping_add(off)
        .cast::<QueueEntry>()
}

/// Insert the container `elt` at the tail of `head`, chaining through the
/// `QueueEntry` field `off` bytes inside each container; what the old
/// `queue_enter()` macro expanded to.
///
/// # Safety
///
/// `head` must be an initialized queue head and `elt` a valid, unlinked
/// container with a `QueueEntry` at `off` bytes; every link in the queue must
/// be a container with the same offset (or the head itself), and every link
/// must stay at its address while linked.
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
    // SAFETY: as above.
    unsafe { check_chain(head, off) };
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
    // SAFETY: the insertion keeps the head consistent.
    unsafe { check_chain(head, off) };
}

/// Insert the container `elt` at the head of `head`; what the old
/// `queue_enter_first()` macro expanded to.
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
    // SAFETY: as above.
    unsafe { check_chain(head, off) };
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
    // SAFETY: the insertion keeps the head consistent.
    unsafe { check_chain(head, off) };
}

/// Remove the container `elt` from the queue headed by `head`; what the old
/// `queue_remove()` macro expanded to.
///
/// # Safety
///
/// `elt` must be linked into `head`'s queue, with a `QueueEntry` at `off`
/// bytes, and every link in the queue must be a container with the same offset
/// (or the head) that stays at its address while linked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_remove_generic(
    head: *mut QueueEntry,
    elt: *mut c_void,
    off: usize,
) {
    let chain = chain_of(elt, off);
    // SAFETY: the caller promises `head` is a valid head and the queue
    // satisfies the container-links invariant for `off`.
    unsafe { check_chain(head, off) };
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
    // SAFETY: the removal keeps the head consistent.
    unsafe { check_chain(head, off) };
}
