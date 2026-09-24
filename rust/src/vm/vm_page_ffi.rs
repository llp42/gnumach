// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/vm_page.c:
//   Copyright (c) 2010-2014 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the physical-page module, one adapter per symbol
//! `vm/vm_page.c` used to define and `vm/vm_page.h` declares.

use crate::glue::Panic;
use crate::vm::types::VmPage;
use crate::vm::vm_page;
use core::ffi::{c_char, c_int, c_uint, c_ushort};
use core::ptr::NonNull;

/// `vm_page_seg_name()` in C.
///
/// # Safety
///
/// The returned pointer is to a static NUL-terminated string and must not be
/// freed; the call has no other requirement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_seg_name(seg_index: c_uint) -> *const c_char {
    match vm_page::seg_name(seg_index) {
        Some(name) => name.as_ptr(),
        // SAFETY: `Panic` does not return; the file, function and message are
        // this port's, as the C `panic()` had them.
        None => unsafe {
            Panic(
                c"rust/src/vm/vm_page_ffi.rs".as_ptr(),
                // Only `c_int` widths can reach `Panic`'s varargs.
                line!() as c_int,
                c"vm_page_seg_name".as_ptr(),
                c"vm_page: invalid segment index".as_ptr(),
            )
        },
    }
}

/// `vm_page_set_type()` in C.
///
/// # Safety
///
/// `page` must point at the first of `1 << order` live, contiguous page
/// descriptors, and the caller must serialize access to them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_set_type(
    page: *mut VmPage,
    order: c_uint,
    type_: c_ushort,
) {
    // SAFETY: the caller promises the run of descriptors.
    unsafe { vm_page::set_type(NonNull::new_unchecked(page), order, type_) };
}

/// `vm_page_wire()` in C.
///
/// # Safety
///
/// `page` must be a live page, and the caller must hold its object lock and
/// the page-queues lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_wire(page: *mut VmPage) {
    // SAFETY: the caller promises a live page and the two locks.
    unsafe { vm_page::wire(NonNull::new_unchecked(page)) };
}
