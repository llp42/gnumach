// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_pageout.c and vm/vm_pageout.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the pageout daemon, one adapter per symbol
//! `vm/vm_pageout.c` used to define and `vm/vm_pageout.h` declares.

use crate::arch::types::VmOffset;
use crate::vm::types::{VmObject, VmPage};
use crate::vm::vm_pageout;
use core::ffi::c_int;
use core::ptr::{self, NonNull};

/// `vm_pageout_setup()` in C.
///
/// # Safety
///
/// `m` must be a live, busy page that is on no pageout queue, its object must
/// be unlocked and hold a paging reference, and `new_object` must be an
/// unlocked object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pageout_setup(
    m: *mut VmPage,
    paging_offset: VmOffset,
    new_object: *mut VmObject,
    new_offset: VmOffset,
    flush: c_int,
) -> *mut VmPage {
    let (m, new_object) = unsafe {
        (
            NonNull::new_unchecked(m),
            NonNull::new_unchecked(new_object),
        )
    };
    // SAFETY: the caller's contract is the setup routine's own.
    unsafe {
        vm_pageout::setup(m, paging_offset, new_object, new_offset, flush != 0)
    }
    .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_pageout_page()` in C.
///
/// # Safety
///
/// `m` must be a live, busy page that is on no pageout queue, and its object
/// must be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pageout_page(
    m: *mut VmPage,
    initial: c_int,
    flush: c_int,
) {
    // SAFETY: the caller's contract is the page routine's own.
    unsafe {
        vm_pageout::page(NonNull::new_unchecked(m), initial != 0, flush != 0)
    };
}

/// `vm_pageout()` in C.
///
/// # Safety
///
/// Must be called on the kernel thread that becomes the daemon, as the C
/// `kern/startup.c` did; the call never returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pageout() -> ! {
    // SAFETY: the caller's contract is the daemon's own.
    unsafe { vm_pageout::pageout() }
}

/// `vm_pageout_start()` in C.
///
/// # Safety
///
/// The caller must hold `vm_page_queue_free_lock`, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pageout_start() {
    // SAFETY: the caller holds the free lock, as the C required.
    unsafe { vm_pageout::start() };
}

/// `vm_pageout_resume()` in C.
///
/// # Safety
///
/// The caller must hold `vm_page_queue_free_lock`, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pageout_resume() {
    // SAFETY: the caller holds the free lock, as the C required.
    unsafe { vm_pageout::resume() };
}
