// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/vm_page.c:
//   Copyright (c) 2010-2014 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the physical-page module, one adapter per symbol
//! `vm/vm_page.c` used to define and `vm/vm_page.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::Panic;
use crate::vm::types::VmPage;
use crate::vm::vm_page;
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_ushort};
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

/// `vm_page_load()` in C.
///
/// # Safety
///
/// Must run during bootstrap, before `vm_page_setup()`, with `seg_index`
/// below `VM_PAGE_MAX_SEGS` and both addresses page-aligned, as the C
/// required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_load(
    seg_index: c_uint,
    start: VmOffset,
    end: VmOffset,
) {
    vm_page::load(seg_index, start, end);
}

/// `vm_page_load_heap()` in C.
///
/// # Safety
///
/// Must run after `vm_page_load()` for the same segment, before
/// `vm_page_setup()`, with both addresses page-aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_load_heap(
    seg_index: c_uint,
    start: VmOffset,
    end: VmOffset,
) {
    vm_page::load_heap(seg_index, start, end);
}

/// `vm_page_ready()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_ready() -> c_int {
    c_int::from(vm_page::is_ready())
}

/// `vm_page_bootalloc()` in C.
///
/// # Safety
///
/// Must run after the segments are loaded and before the page module hands
/// out normal pages, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_bootalloc(size: VmSize) -> VmOffset {
    vm_page::bootalloc(size)
}

/// `vm_page_setup()` in C.
///
/// # Safety
///
/// Must run once, after the architecture code loaded every segment, as the C
/// required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_setup() {
    vm_page::setup();
}

/// `vm_page_manage()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor the kernel is handing to the page
/// module, not yet on any list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_manage(page: *mut VmPage) {
    // SAFETY: the caller promises the live descriptor.
    unsafe { vm_page::manage(page) };
}

/// `vm_page_lookup_pa()` in C.
///
/// # Safety
///
/// The returned pointer is to a live descriptor or null; the call has no
/// other requirement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_lookup_pa(pa: VmOffset) -> *mut VmPage {
    vm_page::lookup_pa(pa).map_or(core::ptr::null_mut(), NonNull::as_ptr)
}

/// `vm_page_check()` in C.
///
/// # Safety
///
/// `page` must be a live descriptor, and the caller must hold whatever lock
/// the C's `VM_PAGE_CHECK` call sites held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_check(page: *const VmPage) {
    // SAFETY: the caller promises the live descriptor.
    unsafe { vm_page::check(page) };
}

/// `vm_page_alloc_pa()` in C.  Returns with `vm_page_queue_free_lock` held.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be ready to
/// release it, as the C's callers were.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_alloc_pa(
    order: c_uint,
    selector: c_uint,
    type_: c_ushort,
) -> *mut VmPage {
    // SAFETY: the caller's contract is the allocator's own.
    unsafe { vm_page::alloc_pa(order, selector, type_) }
}

/// `vm_page_free_pa()` in C.
///
/// # Safety
///
/// `page` must be the first of `1 << order` descriptors the module handed
/// out, and the caller must hold `vm_page_queue_free_lock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_free_pa(page: *mut VmPage, order: c_uint) {
    // SAFETY: the caller promises the live descriptor and the free lock.
    unsafe { vm_page::free_pa(page, order) };
}

/// `vm_page_info_all()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_info_all() {
    vm_page::info_all();
}

/// `vm_page_seg_end()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_seg_end(selector: c_uint) -> VmOffset {
    vm_page::seg_end(selector)
}

/// `vm_page_table_size()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_table_size() -> c_ulong {
    vm_page::table_size() as c_ulong
}

/// `vm_page_table_index()` in C.
///
/// # Safety
///
/// `pa` must be inside a loaded segment, as the C's callers guaranteed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_table_index(pa: VmOffset) -> c_ulong {
    vm_page::table_index(pa) as c_ulong
}

/// `vm_page_mem_size()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_mem_size() -> VmOffset {
    vm_page::mem_size()
}

/// `vm_page_mem_free()` in C.
///
/// # Safety
///
/// The call has no other requirement than the C's; the C itself reads the
/// counters without a lock, relying on the kernel being non-preemptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_mem_free() -> c_ulong {
    vm_page::mem_free() as c_ulong
}

/// `vm_page_unwire()` in C.
///
/// # Safety
///
/// `page` must be a live page whose object lock and page-queues lock the
/// caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_unwire(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and the two locks.
    unsafe { vm_page::unwire(page) };
}

/// `vm_page_deactivate()` in C.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_deactivate(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and the lock.
    unsafe { vm_page::deactivate(page) };
}

/// `vm_page_activate()` in C.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_activate(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and the lock.
    unsafe { vm_page::activate(page) };
}

/// `vm_page_queues_remove()` in C.
///
/// # Safety
///
/// `page` must be a live page and the caller must hold the page-queues lock
/// and, when the page is queued, its object lock, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_queues_remove(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and the locks.
    unsafe { vm_page::queues_remove(page) };
}

/// `vm_page_balance()` in C.  Returns with `vm_page_queue_free_lock` held.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be ready to
/// release it, as the C's callers were.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_balance() -> c_int {
    // SAFETY: the caller's contract is the balancer's own.
    c_int::from(unsafe { vm_page::balance() })
}

/// `vm_page_evict()` in C.  Returns with `vm_page_queue_free_lock` held.
///
/// # Safety
///
/// `should_wait` must be writable, and the caller must not hold
/// `vm_page_queue_free_lock` but must be ready to release it, as the C's
/// callers were.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_evict(should_wait: *mut c_int) -> c_int {
    // SAFETY: the caller's contract is the evictor's own.
    c_int::from(unsafe { vm_page::evict(should_wait) })
}

/// `vm_page_refill_inactive()` in C.
///
/// # Safety
///
/// The caller must not hold the page-queues lock, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_refill_inactive() {
    vm_page::refill_inactive();
}

/// `vm_page_wait()` in C.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock` and must be ready to
/// block, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_page_wait(
    continuation: Option<unsafe extern "C" fn()>,
) {
    // SAFETY: the caller promises the free lock is not held.
    unsafe { vm_page::wait(continuation) };
}
