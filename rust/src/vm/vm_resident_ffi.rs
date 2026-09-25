// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_resident.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of resident memory management, one adapter per symbol
//! `vm/vm_resident.c` used to define and `vm_page.h`/`pmap.h` declare.

use crate::arch::types::{VmOffset, VmSize};
use crate::ipc::HashInfoBucket;
use crate::kern::debug::kpanic;
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
            kpanic!(
                "pmap_steal_memory",
                "not enough kernel virtual space for {}MB virtual allocation!\n",
                size >> 20
            )
        }
    }
}

/// `vm_page_bootstrap()` in C.
///
/// # Safety
///
/// Must be called once during the VM bootstrap, after the physical segments
/// are loaded, and both out-pointers must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_bootstrap(
    startp: *mut VmOffset,
    endp: *mut VmOffset,
) {
    let (start, end) = vm_resident::bootstrap();
    // SAFETY: the caller promises the two writable out-pointers.
    unsafe {
        startp.write(start);
        endp.write(end);
    }
}

/// `vm_page_insert()` in C.
///
/// # Safety
///
/// `mem` must be a live page and `object` a live, locked object; the caller
/// must hold `vm_page_queue_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_insert(
    mem: *mut VmPage,
    object: *mut VmObject,
    offset: VmOffset,
) {
    // SAFETY: the caller promises the live page and object and the lock.
    unsafe {
        vm_resident::insert(
            NonNull::new_unchecked(mem),
            NonNull::new_unchecked(object),
            offset,
        )
    };
}

/// `vm_page_replace()` in C.
///
/// # Safety
///
/// `mem` must be a live page and `object` a live, locked object; the caller
/// must hold `vm_page_queue_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_replace(
    mem: *mut VmPage,
    object: *mut VmObject,
    offset: VmOffset,
) {
    // SAFETY: the caller promises the live page and object and the lock.
    unsafe {
        vm_resident::replace(
            NonNull::new_unchecked(mem),
            NonNull::new_unchecked(object),
            offset,
        )
    };
}

/// `vm_page_remove()` in C.
///
/// # Safety
///
/// `mem` must be a live, tabled page whose object lock and page-queues lock
/// the caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_remove(mem: *mut VmPage) {
    // SAFETY: the caller promises the live page and the two locks.
    unsafe { vm_resident::remove(NonNull::new_unchecked(mem)) };
}

/// `vm_page_lookup()` in C.
///
/// # Safety
///
/// `object` must be a live, locked object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_lookup(
    object: *mut VmObject,
    offset: VmOffset,
) -> *mut VmPage {
    // SAFETY: the caller promises the live, locked object.
    let page =
        unsafe { vm_resident::lookup(NonNull::new_unchecked(object), offset) };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_grab_fictitious()` in C.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_grab_fictitious() -> *mut VmPage {
    // SAFETY: the caller promises the free lock is not held.
    let page = unsafe { vm_resident::grab_fictitious() };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_more_fictitious()` in C.
///
/// # Safety
///
/// The slab package must be up and the caller must be allowed to block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_more_fictitious() {
    // SAFETY: the caller promises the cache is up.
    unsafe { vm_resident::more_fictitious() };
}

/// `vm_page_convert()` in C.
///
/// # Safety
///
/// `mp` must point at a live fictitious page whose object lock the caller
/// holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_convert(mp: *mut *mut VmPage) -> c_int {
    // SAFETY: the caller promises the writable slot and the live page.
    let fictitious = unsafe { *mp };
    // SAFETY: the caller promises the live fictitious page and its lock.
    match unsafe { vm_resident::convert(NonNull::new_unchecked(fictitious)) } {
        Some(real) => {
            // SAFETY: the caller promises the writable slot.
            unsafe { mp.write(real.as_ptr()) };
            c_int::from(true)
        }
        None => c_int::from(false),
    }
}

/// `vm_page_grab_contig()` in C.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be in a
/// context where the allocator may spin.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_grab_contig(
    size: VmSize,
    selector: c_uint,
) -> *mut VmPage {
    // SAFETY: the caller promises the free lock is not held.
    let page = unsafe { vm_resident::grab_contig(size, selector) };
    page.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_free_contig()` in C.
///
/// # Safety
///
/// `mem` must be the first of the live descriptors `vm_page_grab_contig()`
/// returned for `size`, and the caller must not hold
/// `vm_page_queue_free_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_free_contig(mem: *mut VmPage, size: VmSize) {
    // SAFETY: the caller promises the live block and the free lock.
    unsafe { vm_resident::free_contig(NonNull::new_unchecked(mem), size) };
}

/// `vm_page_free()` in C.
///
/// # Safety
///
/// `mem` must be a live page the caller owns, with its object lock and
/// `vm_page_queue_lock` held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_free(mem: *mut VmPage) {
    // SAFETY: the caller promises the live page and the locks.
    unsafe { vm_resident::free(NonNull::new_unchecked(mem)) };
}

/// `vm_page_info()` in C.
///
/// # Safety
///
/// `info` must be writable for `count` records, and the caller must hold no
/// lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_info(
    info: *mut HashInfoBucket,
    count: c_uint,
) -> c_uint {
    // SAFETY: the caller promises the writable buffer and no held lock.
    unsafe { vm_resident::info(info, count) }
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
