// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Simple doubly-linked list, which `kern/list.h` declares.

use core::marker::{PhantomData, PhantomPinned};
use core::pin::Pin;
use core::ptr::NonNull;

/// A list head or node, layout-identical to `struct list` of <kern/list.h>.
#[repr(C)]
pub struct List {
    prev: Option<NonNull<List>>,
    next: Option<NonNull<List>>,
    _pin: PhantomPinned,
}

const _: () = assert!(size_of::<List>() == 2 * size_of::<*mut List>());
const _: () = assert!(align_of::<List>() == align_of::<*mut List>());
const _: () = assert!(core::mem::offset_of!(List, prev) == 0);
const _: () =
    assert!(core::mem::offset_of!(List, next) == size_of::<*mut List>());

impl List {
    /// `init_head()` makes it a head.
    #[must_use]
    pub const fn unlinked() -> Self {
        Self {
            prev: None,
            next: None,
            _pin: PhantomPinned,
        }
    }

    /// View an already-allocated node as pinned.
    ///
    /// # Safety
    ///
    /// `this` must point at a valid, aligned `List`, and its storage must
    /// remain at that address for as long as it is linked into a list: the
    /// list's links point at it.
    pub unsafe fn pin_in_place<'a>(this: NonNull<List>) -> Pin<&'a mut List> {
        // SAFETY: the caller promises validity and address stability.
        unsafe { Pin::new_unchecked(&mut *this.as_ptr()) }
    }

    /// `list_init()` in C.
    ///
    /// # Safety
    ///
    /// The node must not be linked into a list: its links are overwritten.
    pub unsafe fn init_head(self: Pin<&mut Self>) {
        // SAFETY: the body only writes the link fields; the node is never
        // moved out of the pin.
        let this = unsafe { self.get_unchecked_mut() };
        // SAFETY: the caller promises a valid, unlinked node.
        unsafe { List::init_head_at(NonNull::from(&mut *this)) };
    }

    /// `list_init()` in C.
    ///
    /// # Safety
    ///
    /// `this` must point at valid, aligned storage for a `List` that is not
    /// linked into a list and stays at its address while linked.
    pub unsafe fn init_head_at(this: NonNull<List>) {
        // SAFETY: the caller promises valid, unlinked storage.
        unsafe {
            (*this.as_ptr()).prev = Some(this);
            (*this.as_ptr()).next = Some(this);
        }
    }

    /// `list_node_unlinked()` in C.
    pub fn is_unlinked(&self) -> bool {
        self.prev.is_none()
    }

    pub fn is_linked(&self) -> bool {
        !self.is_unlinked()
    }

    /// `list_empty()` in C.
    pub fn is_empty(&self) -> bool {
        let this = NonNull::from(self);
        self.next == Some(this)
    }

    /// `list_singular()` in C.
    pub fn is_singular(&self) -> bool {
        let this = NonNull::from(self);
        self.next != Some(this) && self.next == self.prev
    }

    /// `list_first()` in C.
    pub fn first(&self) -> Option<NonNull<List>> {
        self.next.filter(|node| *node != NonNull::from(self))
    }

    /// `list_for_each()` in C, over this head's nodes.
    ///
    /// Every linked node must outlive the head, and nothing may mutate the
    /// list while the iterator runs.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            head: NonNull::from(self),
            next: self.next,
            _marker: PhantomData,
        }
    }

    /// `list_last()` in C.
    pub fn last(&self) -> Option<NonNull<List>> {
        self.prev.filter(|node| *node != NonNull::from(self))
    }

    /// `list_insert_head()` in C.
    ///
    /// # Safety
    ///
    /// `node` must be valid, unlinked and kept at its address while linked;
    /// nothing else may access the list during the call.
    pub unsafe fn insert_head(&mut self, node: NonNull<List>) {
        // SAFETY: the caller promises `node` is valid and unlinked.
        unsafe { List::add(Some(NonNull::from(&mut *self)), self.next, node) };
    }

    /// `list_insert_tail()` in C.
    ///
    /// # Safety
    ///
    /// Same contract as `insert_head()`.
    pub unsafe fn insert_tail(&mut self, node: NonNull<List>) {
        // SAFETY: the caller promises `node` is valid and unlinked.
        unsafe { List::add(self.prev, Some(NonNull::from(&mut *self)), node) };
    }

    /// `list_insert_before()` in C.
    ///
    /// # Safety
    ///
    /// `next` must be linked into a list, `node` valid and unlinked, and both
    /// must stay at their addresses while linked; nothing else may access the
    /// list during the call.
    pub unsafe fn insert_before(next: NonNull<List>, node: NonNull<List>) {
        // SAFETY: the caller promises `next` is linked.
        let prev = unsafe { (*next.as_ptr()).prev };
        // SAFETY: as above.
        unsafe { List::add(prev, Some(next), node) };
    }

    /// `list_insert_after()` in C.
    ///
    /// # Safety
    ///
    /// Same contract as `insert_before()`.
    pub unsafe fn insert_after(prev: NonNull<List>, node: NonNull<List>) {
        // SAFETY: the caller promises `prev` is linked.
        let next = unsafe { (*prev.as_ptr()).next };
        // SAFETY: as above.
        unsafe { List::add(Some(prev), next, node) };
    }

    /// `list_add()` in C.
    ///
    /// # Safety
    ///
    /// `prev` and `next` must be the endpoints of one gap in a list, `node`
    /// valid and unlinked, and all three must stay at their addresses while
    /// linked.
    unsafe fn add(
        prev: Option<NonNull<List>>,
        next: Option<NonNull<List>>,
        node: NonNull<List>,
    ) {
        // SAFETY: the caller promises both neighbours are valid.
        unsafe {
            if let Some(next) = next {
                (*next.as_ptr()).prev = Some(node);
            }
            if let Some(prev) = prev {
                (*prev.as_ptr()).next = Some(node);
            }
            (*node.as_ptr()).prev = prev;
            (*node.as_ptr()).next = next;
        }
    }

    /// `list_remove()` in C: the node's links are left stale and it must be
    /// re-initialized before it is linked again.
    ///
    /// # Safety
    ///
    /// `node` must be linked into a list and nothing else may access that list
    /// during the call.
    pub unsafe fn remove(node: NonNull<List>) {
        // SAFETY: the caller promises `node` is linked, so its links are valid
        // nodes of one list.
        unsafe {
            if let Some(prev) = (*node.as_ptr()).prev {
                (*prev.as_ptr()).next = (*node.as_ptr()).next;
            }
            if let Some(next) = (*node.as_ptr()).next {
                (*next.as_ptr()).prev = (*node.as_ptr()).prev;
            }
        }
    }

    /// `list_set_head()` in C.
    ///
    /// # Safety
    ///
    /// `old_head` must be a valid head or a stale one, `new_head` must be
    /// valid and unlinked, and the nodes of the list must stay at their
    /// addresses and not be accessed through `old_head` again.
    pub unsafe fn set_head(new_head: NonNull<List>, old_head: NonNull<List>) {
        // SAFETY: the caller promises both heads are valid.
        unsafe {
            if (*old_head.as_ptr()).is_empty() {
                (*new_head.as_ptr()).prev = Some(new_head);
                (*new_head.as_ptr()).next = Some(new_head);
                return;
            }
            (*new_head.as_ptr()).prev = (*old_head.as_ptr()).prev;
            (*new_head.as_ptr()).next = (*old_head.as_ptr()).next;
            if let Some(next) = (*new_head.as_ptr()).next {
                (*next.as_ptr()).prev = Some(new_head);
            }
            if let Some(prev) = (*new_head.as_ptr()).prev {
                (*prev.as_ptr()).next = Some(new_head);
            }
        }
    }

    /// `list_concat()` in C.
    ///
    /// # Safety
    ///
    /// Both must be valid heads, their nodes must stay at their addresses, and
    /// `list2` must not be used again.
    pub unsafe fn concat(list1: NonNull<List>, list2: NonNull<List>) {
        // SAFETY: the caller promises both heads are valid.
        unsafe {
            if (*list2.as_ptr()).is_empty() {
                return;
            }
            let last1 = (*list1.as_ptr()).prev;
            let first2 = (*list2.as_ptr()).next;
            let last2 = (*list2.as_ptr()).prev;
            if let Some(last1) = last1 {
                (*last1.as_ptr()).next = first2;
            }
            if let Some(first2) = first2 {
                (*first2.as_ptr()).prev = last1;
            }
            if let Some(last2) = last2 {
                (*last2.as_ptr()).next = Some(list1);
            }
            (*list1.as_ptr()).prev = last2;
        }
    }

    /// `list_split()` in C.
    ///
    /// # Safety
    ///
    /// `list2` must be a valid head, `node` a node of it or `list2` itself,
    /// and every node must stay at its address.
    pub unsafe fn split(
        list1: NonNull<List>,
        list2: NonNull<List>,
        node: NonNull<List>,
    ) {
        // SAFETY: the caller promises the heads and `node` are valid.
        unsafe {
            if (*list2.as_ptr()).is_empty() || list2 == node {
                return;
            }
            let first2 = (*list2.as_ptr()).next;
            if first2 == Some(node) {
                return;
            }
            if let Some(first2) = first2 {
                (*list1.as_ptr()).next = Some(first2);
                (*first2.as_ptr()).prev = Some(list1);
            }
            (*list1.as_ptr()).prev = (*node.as_ptr()).prev;
            if let Some(prev) = (*node.as_ptr()).prev {
                (*prev.as_ptr()).next = Some(list1);
            }
            (*list2.as_ptr()).next = Some(node);
            (*node.as_ptr()).prev = Some(list2);
        }
    }
}

/// The nodes of a list, excluding the head.
pub struct Iter<'a> {
    head: NonNull<List>,
    next: Option<NonNull<List>>,
    _marker: PhantomData<&'a List>,
}

impl Iterator for Iter<'_> {
    type Item = NonNull<List>;

    fn next(&mut self) -> Option<NonNull<List>> {
        let node = self.next?;

        if node == self.head {
            self.next = None;
            return None;
        }

        // SAFETY: every link of a list built through these methods points at
        // a node that stays at its address while linked.
        self.next = unsafe { (*node.as_ptr()).next };
        Some(node)
    }
}

/// The list node `node` belongs to, mapped back to its container of type `T`;
/// `list_entry()`/`list_first_entry()` in C.
///
/// # Safety
///
/// `node` must point at the `field` member of a live `T`, and `field` must be
/// a `List` at that offset.
pub unsafe fn entry<T>(node: NonNull<List>, field: usize) -> NonNull<T> {
    // SAFETY: the caller promises `node` lives inside a `T` at the given
    // offset.
    unsafe {
        NonNull::new_unchecked(
            node.as_ptr().cast::<u8>().sub(field).cast::<T>(),
        )
    }
}
