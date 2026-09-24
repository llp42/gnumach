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
    Panic, kernel_pmap, kmem_cache_init, pmap_copy_page, pmap_enter,
    pmap_virtual_space, pmap_zero_page, virtual_space_end,
    virtual_space_start, vm_page_alloc_pa, vm_page_bootalloc, vm_page_cache,
    vm_page_check, vm_page_external_laundry_count, vm_page_free_pa,
    vm_page_insert, vm_page_laundry_count, vm_page_queue_free_lock,
    vm_page_queue_lock, vm_page_remove, vm_pageout_resume,
};
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_map::{round_page, trunc_page};
use core::ffi::{c_int, c_uint, c_ushort};
use core::mem::size_of;
use core::ptr::{NonNull, addr_of_mut, null_mut};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high physical
/// memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `VM_PAGE_DMA32` and `VM_PAGE_DIRECTMAP` of <vm/vm_page.h>: the flags that
/// ask for those segments.  The values follow the limit ordering, which the
/// 64-bit and the non-PAE 32-bit builds fix differently; the latter has no
/// DMA32 segment for `vm_page_grab()` to select.
#[cfg(target_arch = "x86_64")]
const VM_PAGE_DMA32: c_uint = 0x04;
#[cfg(target_arch = "x86_64")]
const VM_PAGE_DIRECTMAP: c_uint = 0x02;
#[cfg(target_arch = "x86")]
const VM_PAGE_DIRECTMAP: c_uint = 0x04;

/// `VM_PAGE_SEL_*` of <vm/vm_page.h>: the segment selectors
/// `vm_page_alloc_pa()` takes.  The DMA32 and DIRECTMAP indices swap with
/// the segment ordering.
#[cfg(target_arch = "x86_64")]
const VM_PAGE_SEL_DIRECTMAP: c_uint = 1;
#[cfg(target_arch = "x86_64")]
const VM_PAGE_SEL_DMA32: c_uint = 2;
#[cfg(target_arch = "x86")]
const VM_PAGE_SEL_DIRECTMAP: c_uint = 2;
const VM_PAGE_SEL_DMA: c_uint = 0;
const VM_PAGE_SEL_HIGHMEM: c_uint = 3;

/// `VM_PT_KERNEL` of <vm/vm_page.h>: the type for generic kernel
/// allocations.
const VM_PT_KERNEL: c_ushort = 3;

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
    // SAFETY: `grab()` is the real allocator, and `flags` is its documented
    // selector set.
    let page = unsafe { grab(flags) }?;

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

/// `vm_page_init()` in C: initialize the fields of a page whose storage holds
/// random values.  The C kept the body in a `vm_page_init_template()` static
/// with this one caller.
pub(crate) fn init(page: &mut VmPage) {
    page.object = null_mut();
    page.offset = 0;
    page.set_wire_count(0);
    page.set_inactive(false);
    page.set_active(false);
    page.set_laundry(false);
    page.set_external_laundry(false);
    page.set_free(false);
    page.set_external(false);
    page.set_busy(true);
    page.set_wanted(false);
    page.set_tabled(false);
    page.set_fictitious(false);
    page.set_private(false);
    page.set_absent(false);
    page.set_error(false);
    page.set_dirty(false);
    page.set_precious(false);
    page.set_reference(false);
    page.set_page_lock(VmProt::NONE);
    page.set_unlock_request(VmProt::NONE);
}

/// `vm_page_module_init()` in C: create the `vm_page` slab cache.
///
/// # Safety
///
/// Must run once, in the bootstrap sequence, after the slab package is up.
pub(crate) unsafe fn module_init() {
    // SAFETY: the caller runs the sequence once; `vm_page_cache` is the live
    // cache the C keeps, and `kmem_cache_init()` is its initializer.
    // `size_of::<VmPage>()` is the C `sizeof(struct vm_page)`.
    unsafe {
        kmem_cache_init(
            addr_of_mut!(vm_page_cache),
            c"vm_page".as_ptr(),
            size_of::<VmPage>(),
            0,
            None,
            0,
        )
    };
}

/// The selector `vm_page_grab()` computes from its flags.  The limit macros
/// order the segments differently on the two builds, so x86_64 tests the
/// DMA32 flag before the DIRECTMAP one and the non-PAE i686 build has no
/// DMA32 test at all.
#[cfg(target_arch = "x86_64")]
fn alloc_selector(flags: c_uint) -> c_uint {
    if flags & VM_PAGE_HIGHMEM != 0 {
        VM_PAGE_SEL_HIGHMEM
    } else if flags & VM_PAGE_DMA32 != 0 {
        VM_PAGE_SEL_DMA32
    } else if flags & VM_PAGE_DIRECTMAP != 0 {
        VM_PAGE_SEL_DIRECTMAP
    } else {
        VM_PAGE_SEL_DMA
    }
}

/// The selector `vm_page_grab()` computes from its flags; the non-PAE i686
/// build has no DMA32 segment for the flag to select.
#[cfg(target_arch = "x86")]
fn alloc_selector(flags: c_uint) -> c_uint {
    if flags & VM_PAGE_HIGHMEM != 0 {
        VM_PAGE_SEL_HIGHMEM
    } else if flags & VM_PAGE_DIRECTMAP != 0 {
        VM_PAGE_SEL_DIRECTMAP
    } else {
        VM_PAGE_SEL_DMA
    }
}

/// `vm_page_grab()` in C: take a page out of the free list.
///
/// # Safety
///
/// The caller must not hold `vm_page_queue_free_lock`, and must be in a
/// context where the allocator's spinning is allowed.
pub(crate) unsafe fn grab(flags: c_uint) -> Option<NonNull<VmPage>> {
    // SAFETY: the caller promised the free lock is not held; the allocator
    // takes it and leaves it held for the release below.
    let page =
        unsafe { vm_page_alloc_pa(0, alloc_selector(flags), VM_PT_KERNEL) };
    let page = NonNull::new(page);

    if let Some(page) = page {
        // SAFETY: `page` is the live page just allocated, and the free lock,
        // still held, guards its free flag.
        unsafe { (*page.as_ptr()).set_free(false) };
    }

    // SAFETY: `vm_page_alloc_pa()` returns with the free lock held, on both
    // the found and the exhausted path.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };

    page
}

/// `vm_page_grab_phys_addr()` in C: the physical address of a direct-mapped
/// page, or `-1` when the free list is exhausted.
///
/// # Safety
///
/// Same contract as [`grab()`].
pub(crate) unsafe fn grab_phys_addr() -> VmOffset {
    // SAFETY: the caller promises `grab()`'s contract.
    let Some(page) = (unsafe { grab(VM_PAGE_DIRECTMAP) }) else {
        return VmOffset::MAX;
    };

    // SAFETY: `page` is the live page just allocated.
    unsafe { (*page.as_ptr()).phys_addr }
}

/// `vm_page_release()` in C: return a page to the free list, resuming the
/// pageout daemon when the last laundered page is gone.
///
/// # Safety
///
/// `page` must be a live page that no one else holds, and the caller must
/// not hold `vm_page_queue_free_lock`.
pub(crate) unsafe fn release(
    page: NonNull<VmPage>,
    laundry: bool,
    external_laundry: bool,
) {
    let ptr = page.as_ptr();

    // SAFETY: the caller promised a live page with no other holder, and does
    // not hold the free lock.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).lock() };

    // SAFETY: the free lock serializes the flag, and the page is live.
    if unsafe { (*ptr).is_free() } {
        // SAFETY: `Panic` does not return; the function and message are the
        // C `panic()`'s.
        unsafe {
            Panic(
                c"rust/src/vm/vm_resident.rs".as_ptr(),
                line!() as c_int,
                c"vm_page_release".as_ptr(),
                c"vm_page_release".as_ptr(),
            )
        }
    }

    // SAFETY: the free lock is held and guards the flag.
    unsafe { (*ptr).set_free(true) };
    // SAFETY: the free lock is held; `vm_page_free_pa()` is the real backend
    // and order zero is the C's.
    unsafe { vm_page_free_pa(ptr, 0) };

    if laundry {
        // SAFETY: the free lock guards the counter.
        let count = unsafe { vm_page_laundry_count }.wrapping_sub(1);
        // SAFETY: as above.
        unsafe { vm_page_laundry_count = count };
        if count == 0 {
            // SAFETY: the pageout daemon's resume path is the real symbol,
            // and the C calls it under the free lock.
            unsafe { vm_pageout_resume() };
        }
    }

    if external_laundry {
        // SAFETY: the free lock guards the counter.
        let count = unsafe { vm_page_external_laundry_count };
        if 0 < count {
            let count = count.wrapping_sub(1);
            // SAFETY: as above.
            unsafe { vm_page_external_laundry_count = count };
            if count == 0 {
                // SAFETY: as `vm_pageout_resume()` above.
                unsafe { vm_pageout_resume() };
            }
        }
    }

    // SAFETY: the lock was taken above and nothing released it since.
    unsafe { (*addr_of_mut!(vm_page_queue_free_lock)).unlock() };
}

/// `vm_page_zero_fill()` in C: zero the page's physical memory.
///
/// # Safety
///
/// `page` must be a live page.
pub(crate) unsafe fn zero_fill(page: NonNull<VmPage>) {
    // SAFETY: the caller promises a live page; `vm_page_check()` is the C
    // validator.
    unsafe { vm_page_check(page.as_ptr()) };
    // SAFETY: the page is live, so its physical address names real memory.
    unsafe { pmap_zero_page((*page.as_ptr()).phys_addr) };
}

/// `vm_page_copy()` in C: copy one page's physical memory to another.
///
/// # Safety
///
/// `src` and `dest` must be live pages.
pub(crate) unsafe fn copy(src: NonNull<VmPage>, dest: NonNull<VmPage>) {
    // SAFETY: the caller promises both pages are live.
    unsafe {
        vm_page_check(src.as_ptr());
        vm_page_check(dest.as_ptr());
    }
    // SAFETY: both pages are live, so their physical addresses name real
    // memory.
    unsafe {
        pmap_copy_page((*src.as_ptr()).phys_addr, (*dest.as_ptr()).phys_addr)
    };
}
