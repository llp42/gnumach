// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_resident.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of resident memory management, one adapter per symbol
//! `vm/vm_resident.c` used to define and `vm_page.h`/`pmap.h` declare.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::Panic;
use crate::vm::types::{VmObject, VmPage};
use crate::vm::vm_resident;
use core::ffi::{c_int, c_uint};
use core::ptr::{self, NonNull};

/// `pmap_steal_memory()` in C.
///
/// # Safety
///
/// Must be called during bootstrap, before the page module hands out normal
/// pages, and `kernel_pmap` must already be the boot pmap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_steal_memory(size: VmSize) -> VmOffset {
    match vm_resident::pmap_steal_memory(size) {
        Ok(addr) => addr,
        Err(size) => {
            // SAFETY: `Panic` does not return; the message and its `%d`
            // argument are the C `panic()`'s.  The C passed `vm_size_t` to a
            // `%d`, which reads the low 32 bits; the cast spells that.
            unsafe {
                Panic(
                    c"rust/src/vm/vm_resident_ffi.rs".as_ptr(),
                    // Only `c_int` widths can reach `Panic`'s varargs.
                    line!() as c_int,
                    c"pmap_steal_memory".as_ptr(),
                    c"not enough kernel virtual space for %dMB virtual allocation!\n"
                        .as_ptr(),
                    (size >> 20) as c_int,
                )
            }
        }
    }
}

/// `vm_page_rename()` in C.
///
/// # Safety
///
/// `page` must be a live page, `new_object` a live object, and the object's
/// lock must be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_rename(
    page: *mut VmPage,
    new_object: *mut VmObject,
    new_offset: VmOffset,
) {
    // SAFETY: the caller promises a live page and object.
    unsafe {
        vm_resident::rename(
            NonNull::new_unchecked(page),
            NonNull::new_unchecked(new_object),
            new_offset,
        )
    };
}

/// `vm_page_alloc_flags()` in C.
///
/// # Safety
///
/// `object` must be a live, locked object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_alloc_flags(
    object: *mut VmObject,
    offset: VmOffset,
    flags: c_uint,
) -> *mut VmPage {
    // SAFETY: the caller promises a live, locked object.
    let page = unsafe {
        vm_resident::alloc_flags(NonNull::new_unchecked(object), offset, flags)
    };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_alloc()` in C.
///
/// # Safety
///
/// `object` must be a live, locked object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_alloc(
    object: *mut VmObject,
    offset: VmOffset,
) -> *mut VmPage {
    // SAFETY: the caller promises a live, locked object.
    let page =
        unsafe { vm_resident::alloc(NonNull::new_unchecked(object), offset) };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_init()` in C.
///
/// # Safety
///
/// `page` must point at writable storage for a live page that no other
/// thread can see yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_init(page: *mut VmPage) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe { vm_resident::init(&mut *page) };
}

/// `vm_page_module_init()` in C.
///
/// # Safety
///
/// Must be called once during the VM bootstrap, after the slab package is
/// initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_module_init() {
    // SAFETY: the caller promises the bootstrap ordering.
    unsafe { vm_resident::module_init() };
}

/// `vm_page_grab()` in C.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock`, must be in a context
/// where the allocator may spin, and must be the page queues' only user of
/// the returned page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_grab(flags: c_uint) -> *mut VmPage {
    // SAFETY: the caller promises `grab()`'s contract.
    let page = unsafe { vm_resident::grab(flags) };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_grab_phys_addr()` in C.
///
/// # Safety
///
/// Same contract as [`vm_page_grab()`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_grab_phys_addr() -> VmOffset {
    // SAFETY: the caller promises `grab()`'s contract.
    unsafe { vm_resident::grab_phys_addr() }
}

/// `vm_page_release()` in C.
///
/// # Safety
///
/// `page` must be a live page that no one else holds, and the caller must
/// not hold `vm_page_queue_free_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_release(
    page: *mut VmPage,
    laundry: c_int,
    external_laundry: c_int,
) {
    // SAFETY: the caller promises a live page with no other holder.
    unsafe {
        vm_resident::release(
            NonNull::new_unchecked(page),
            laundry != 0,
            external_laundry != 0,
        )
    };
}

/// `vm_page_zero_fill()` in C.
///
/// # Safety
///
/// `page` must be a live page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_zero_fill(page: *mut VmPage) {
    // SAFETY: the caller promises a live page.
    unsafe { vm_resident::zero_fill(NonNull::new_unchecked(page)) };
}

/// `vm_page_copy()` in C.
///
/// # Safety
///
/// `src` and `dest` must be live pages.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_copy(src: *mut VmPage, dest: *mut VmPage) {
    // SAFETY: the caller promises two live pages.
    unsafe {
        vm_resident::copy(
            NonNull::new_unchecked(src),
            NonNull::new_unchecked(dest),
        )
    };
}
