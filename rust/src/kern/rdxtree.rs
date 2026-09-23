// SPDX-License-Identifier: BSD-2-Clause
// Derived from kern/rdxtree.c:
//   Copyright (c) 2011-2015 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The radix-tree leaf `kern/rdxtree.c` used to define.

use core::ffi::c_void;

/// Store `ptr` into `*slot`, returning the previous value, as
/// `rdxtree_replace_slot()` of `kern/rdxtree.c` did.
fn replace_slot(slot: &mut *mut c_void, ptr: *mut c_void) -> *mut c_void {
    core::mem::replace(slot, ptr)
}

/// The `rdxtree_replace_slot()` entry of <kern/rdxtree.h>, which
/// `kern/rdxtree.c` used to define.
///
/// # Safety
///
/// `slot` must point at a live, aligned `void *` slot that the caller owns for
/// writing, and no reference to its contents may outlive the call.
#[unsafe(no_mangle)]
#[must_use]
pub unsafe extern "C" fn rdxtree_replace_slot(
    slot: *mut *mut c_void,
    ptr: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller promises `slot` is a live slot it owns for writing.
    replace_slot(unsafe { &mut *slot }, ptr)
}
