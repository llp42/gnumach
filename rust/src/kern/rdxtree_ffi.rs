// SPDX-License-Identifier: BSD-2-Clause
// Derived from kern/rdxtree.c, kern/rdxtree.h and kern/rdxtree_i.h:
//   Copyright (c) 2011-2015 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the radix tree, one adapter per symbol
//! `kern/rdxtree.c` used to define and <kern/rdxtree.h> and
//! <kern/rdxtree_i.h> declare.

use crate::kern::rdxtree::{
    self, Found, Lookup, Rdxtree, RdxtreeIter, RdxtreeKey,
};
use crate::vm::error::{Error, KERN_SUCCESS};
use core::ffi::{c_int, c_void};
use core::ptr::{self, NonNull};

/// `rdxtree_cache_init()` of <kern/rdxtree.h>.
#[unsafe(no_mangle)]
pub extern "C" fn rdxtree_cache_init() {
    rdxtree::cache_init();
}

/// Store the slot a successful insert produced, when the C caller asked
/// for one.
///
/// # Safety
///
/// `slotp` must be null, or point at writable storage for one pointer.
unsafe fn write_slot(slotp: *mut *mut *mut c_void, slot: *mut *mut c_void) {
    if let Some(slotp) = NonNull::new(slotp) {
        // SAFETY: the caller's contract.
        unsafe { slotp.as_ptr().write(slot) };
    }
}

/// `rdxtree_insert_common()` of <kern/rdxtree_i.h>.
///
/// # Safety
///
/// `tree` must point at a live tree, `ptr` must not be null, and `slotp`,
/// when not null, must point at writable storage for one slot address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_insert_common(
    tree: *mut Rdxtree,
    key: u32,
    ptr: *mut c_void,
    slotp: *mut *mut *mut c_void,
) -> c_int {
    let (Some(tree), Some(ptr)) = (NonNull::new(tree), NonNull::new(ptr))
    else {
        return Error::InvalidArgument.as_kern_return();
    };

    // SAFETY: the caller promises a live tree.
    let result =
        unsafe { (*tree.as_ptr()).insert(RdxtreeKey::from_raw(key), ptr) };

    match result {
        Ok(slot) => {
            // SAFETY: the caller promises `slotp` is null or writable.
            unsafe { write_slot(slotp, slot) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `rdxtree_insert_alloc_common()` of <kern/rdxtree_i.h>.
///
/// # Safety
///
/// `tree` must point at a live tree, `ptr` must not be null, `keyp` must
/// point at writable storage for one key, and `slotp`, when not null, must
/// point at writable storage for one slot address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_insert_alloc_common(
    tree: *mut Rdxtree,
    ptr: *mut c_void,
    keyp: *mut u32,
    slotp: *mut *mut *mut c_void,
) -> c_int {
    let (Some(tree), Some(ptr), Some(keyp)) =
        (NonNull::new(tree), NonNull::new(ptr), NonNull::new(keyp))
    else {
        return Error::InvalidArgument.as_kern_return();
    };

    // SAFETY: the caller promises a live tree.
    let result = unsafe { (*tree.as_ptr()).insert_alloc(ptr) };

    match result {
        Ok((key, slot)) => {
            // SAFETY: the caller promises writable key storage.
            unsafe { keyp.as_ptr().write(key.into_raw()) };
            // SAFETY: the caller promises `slotp` is null or writable.
            unsafe { write_slot(slotp, slot) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `rdxtree_remove()` of <kern/rdxtree.h>.
///
/// # Safety
///
/// `tree` must point at a live tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_remove(
    tree: *mut Rdxtree,
    key: u32,
) -> *mut c_void {
    let Some(tree) = NonNull::new(tree) else {
        return ptr::null_mut();
    };

    // SAFETY: the caller promises a live tree.
    unsafe { (*tree.as_ptr()).remove(RdxtreeKey::from_raw(key)) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `rdxtree_lookup_common()` of <kern/rdxtree_i.h>.
///
/// # Safety
///
/// `tree` must point at a live tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_lookup_common(
    tree: *const Rdxtree,
    key: u32,
    get_slot: c_int,
) -> *mut c_void {
    let Some(tree) = NonNull::new(tree.cast_mut()) else {
        return ptr::null_mut();
    };

    let want = if get_slot != 0 {
        Lookup::Slot
    } else {
        Lookup::Value
    };

    // SAFETY: the caller promises a live tree.
    unsafe { (*tree.as_ptr()).lookup(RdxtreeKey::from_raw(key), want) }
        .map_or(ptr::null_mut(), Found::address)
}

/// `rdxtree_walk()` of <kern/rdxtree_i.h>.
///
/// # Safety
///
/// `tree` must point at a live tree, and `iter` at a live iterator over it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_walk(
    tree: *mut Rdxtree,
    iter: *mut RdxtreeIter,
) -> *mut c_void {
    let (Some(tree), Some(iter)) = (NonNull::new(tree), NonNull::new(iter))
    else {
        return ptr::null_mut();
    };

    // SAFETY: the caller promises a live tree and iterator.
    unsafe { (*tree.as_ptr()).walk(&mut *iter.as_ptr()) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `rdxtree_remove_all()` of <kern/rdxtree.h>.
///
/// # Safety
///
/// `tree` must point at a live tree that no other user is walking.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rdxtree_remove_all(tree: *mut Rdxtree) {
    let Some(tree) = NonNull::new(tree) else {
        return;
    };

    // SAFETY: the caller promises a live tree.
    unsafe { (*tree.as_ptr()).remove_all() };
}

/// `rdxtree_replace_slot()` of <kern/rdxtree.h>.
///
/// # Safety
///
/// `slot` must point at a live, aligned `void *` slot that the caller owns
/// for writing, and no reference to its contents may outlive the call.
#[unsafe(no_mangle)]
#[must_use]
pub unsafe extern "C" fn rdxtree_replace_slot(
    slot: *mut *mut c_void,
    ptr: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller promises `slot` is a live slot it owns for writing.
    rdxtree::replace_slot(unsafe { &mut *slot }, ptr)
}
