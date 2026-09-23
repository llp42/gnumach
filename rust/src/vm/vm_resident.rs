// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_resident.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Resident memory management, which `vm/vm_resident.c` used to define.

use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{
    kernel_pmap, pmap_enter, pmap_virtual_space, virtual_space_end,
    virtual_space_start, vm_page_bootalloc, vm_page_grab, vm_page_insert,
    vm_page_queue_lock, vm_page_remove,
};
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_map::{round_page, trunc_page};
use core::ffi::{c_int, c_uint};
use core::ptr::{NonNull, addr_of_mut};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high physical
/// memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `pmap_steal_memory()` in C: reserve `size` of kernel virtual space and map
/// fresh physical pages into it.
///
/// On failure the error is the page-rounded size the C passed to its
/// exhaustion panic, in bytes.
pub(crate) fn pmap_steal_memory(size: VmSize) -> Result<VmOffset, VmSize> {
    let size = round_page(size);

    let mut start = unsafe { virtual_space_start };
    let mut end = unsafe { virtual_space_end };
    if start == end {
        // SAFETY: the C globals are the live boot pair, and the caller runs
        // before anything else has taken a mapping out of them.
        unsafe { pmap_virtual_space(&mut start, &mut end) };
        start = round_page(start);
        end = trunc_page(end);
        // SAFETY: the locals hold the range the pmap just reported.
        unsafe {
            virtual_space_start = start;
            virtual_space_end = end;
        }
    }

    let addr = start;
    let new_start = start.wrapping_add(size);
    if new_start < start {
        return Err(size);
    }
    // SAFETY: `virtual_space_start` is the live C global and the check above
    // kept the new value inside the address space.
    unsafe { virtual_space_start = new_start };

    let limit = addr.wrapping_add(size);
    let mut vaddr = round_page(addr);
    while vaddr < limit {
        // SAFETY: `vm_page_bootalloc()` is the real early allocator, and the
        // caller runs after the physical segments are loaded.
        let paddr = unsafe { vm_page_bootalloc(PAGE_SIZE) };
        // SAFETY: `kernel_pmap` is the boot pmap and `vaddr` is inside the
        // range just reserved; the C maps the page without wiring it.
        unsafe {
            pmap_enter(
                kernel_pmap,
                vaddr,
                paddr,
                VmProt::READ | VmProt::WRITE,
                c_int::from(false),
            )
        };
        vaddr = vaddr.wrapping_add(PAGE_SIZE);
    }

    Ok(addr)
}

/// `vm_page_rename()` in C: move a page to another object and offset.
///
/// # Safety
///
/// `page` must be a live page and `object` a live object; the object must be
/// locked, as the C requires.
pub(crate) unsafe fn rename(
    page: NonNull<VmPage>,
    object: NonNull<VmObject>,
    offset: VmOffset,
) {
    // SAFETY: the page-queue lock is the live C spin lock the pageout daemon
    // also takes, and the caller holds the object's lock.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        vm_page_remove(page.as_ptr());
        vm_page_insert(page.as_ptr(), object.as_ptr(), offset);
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }
}

/// `vm_page_alloc_flags()` in C: grab a free page and table it in `object`.
///
/// # Safety
///
/// `object` must be a live, locked object.
pub(crate) unsafe fn alloc_flags(
    object: NonNull<VmObject>,
    offset: VmOffset,
    flags: c_uint,
) -> Option<NonNull<VmPage>> {
    // SAFETY: `vm_page_grab()` is the real C routine, and `flags` is its
    // documented selector set.
    let page = NonNull::new(unsafe { vm_page_grab(flags) })?;

    // SAFETY: the page-queue lock is the live C spin lock, and the caller
    // holds the object's lock.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        vm_page_insert(page.as_ptr(), object.as_ptr(), offset);
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }

    Some(page)
}

/// `vm_page_alloc()` in C: `vm_page_alloc_flags()` with `VM_PAGE_HIGHMEM`.
///
/// # Safety
///
/// `object` must be a live, locked object.
pub(crate) unsafe fn alloc(
    object: NonNull<VmObject>,
    offset: VmOffset,
) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller promises a live, locked object.
    unsafe { alloc_flags(object, offset, VM_PAGE_HIGHMEM) }
}
