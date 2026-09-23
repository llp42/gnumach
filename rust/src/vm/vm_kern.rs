// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_kern.c and vm/vm_kern.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Kernel memory management, which `vm/vm_kern.c` used to define and
//! `vm/vm_kern.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{
    kernel_map, kernel_object, kmem_alloc_pages, kmem_valloc, pmap_map_bd,
    pmap_reference, pmap_remove, printf, projected_buffer_deallocate,
    vm_object_reference, vm_submap_object,
};
use crate::vm::error::{Error, error_from_kern_return};
use crate::vm::types::{Pmap, VmInherit, VmProt};
use crate::vm::vm_map::{EnterRequest, VmMap, round_page, trunc_page};
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::{self, NonNull, with_exposed_provenance_mut};
use core::sync::atomic::{AtomicBool, Ordering};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high physical
/// memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `VM_MIN_KERNEL_ADDRESS` of <machine/vm_param.h>: `KERNEL_MAP_BASE` on
/// x86_64 and `0xC0000000` on i686.  A `kernel_object` offset is linear in
/// the kernel virtual address, so a kernel mapping is stored at
/// `addr - VM_MIN_KERNEL_ADDRESS`.
#[cfg(target_arch = "x86_64")]
const VM_MIN_KERNEL_ADDRESS: VmOffset = 0xffff_ffff_8000_0000;
#[cfg(target_arch = "x86")]
const VM_MIN_KERNEL_ADDRESS: VmOffset = 0xC000_0000;

/// `projected_buffer_collect()` in C: unmap every projected buffer of `map`.
pub(crate) fn projected_buffer_collect(
    map: NonNull<VmMap>,
) -> Result<(), Error> {
    // SAFETY: `kernel_map` is the boot kernel map the C global holds.
    if map.as_ptr().cast::<c_void>() == unsafe { kernel_map } {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a live map, and the sentinel is the
    // header's links as `vm_map_to_entry()` computes it.
    let sentinel = unsafe { (*map.as_ptr()).to_entry() };
    // SAFETY: the header's `next` is the sentinel or a live entry.
    let mut entry =
        unsafe { (*map.as_ptr()).hdr.links.next.unwrap_or(sentinel) };

    while entry != sentinel {
        // SAFETY: `entry` is the sentinel or a live entry of the map; the
        // caller promises the chain stays stable except for the deallocation
        // below.
        let (next, start, end, projected) = unsafe {
            let entry = &*entry.as_ptr();
            (
                entry.links.next.unwrap_or(sentinel),
                entry.links.start,
                entry.links.end,
                !entry.projected_on.is_null(),
            )
        };
        if projected {
            // SAFETY: `map` is valid and unlocked, as
            // `projected_buffer_deallocate` requires; `start` and `end` were
            // read from the live entry before the call, which may delete it.
            unsafe { projected_buffer_deallocate(map.as_ptr(), start, end) };
        }
        entry = next;
    }

    Ok(())
}

/// `projected_buffer_in_range()` in C: whether a projected buffer overlaps
/// `start..end`.
pub(crate) fn projected_buffer_in_range(
    map: &VmMap,
    start: VmOffset,
    end: VmOffset,
) -> bool {
    let sentinel = map.to_entry();
    let (found, entry) = map.lookup_entry(start);
    let mut entry = if found {
        entry
    } else {
        // SAFETY: `lookup_entry` returned the sentinel or a live entry, so
        // its `next` is the sentinel or a live entry.
        unsafe { (*entry.as_ptr()).links.next.unwrap_or(sentinel) }
    };

    while entry != sentinel
        && unsafe { (*entry.as_ptr()).projected_on.is_null() }
        && unsafe { (*entry.as_ptr()).links.start } <= end
    {
        // SAFETY: as above.
        entry = unsafe { (*entry.as_ptr()).links.next.unwrap_or(sentinel) };
    }

    entry != sentinel && unsafe { (*entry.as_ptr()).links.start } <= end
}

/// `kmem_alloc_wired_flags()` in C: reserve kernel virtual space and wire
/// memory for it.
pub(crate) fn kmem_alloc_wired_flags(
    map: NonNull<VmMap>,
    size: VmSize,
    flags: c_uint,
) -> Result<VmOffset, Error> {
    let mut addr: VmOffset = 0;
    // SAFETY: the caller promises a valid, unlocked map; `kmem_valloc` writes
    // the address it reserves.
    let status = unsafe { kmem_valloc(map.as_ptr(), &mut addr, size) };
    error_from_kern_return(status)?;

    let offset = addr.wrapping_sub(VM_MIN_KERNEL_ADDRESS);
    // SAFETY: `kernel_object` is the boot object every kernel mapping maps,
    // and `addr..addr + size` is the region `kmem_valloc` just reserved.
    unsafe {
        kmem_alloc_pages(
            kernel_object,
            offset,
            addr,
            addr.wrapping_add(size),
            VmProt::READ | VmProt::WRITE,
            flags,
        )
    };

    Ok(addr)
}

/// `kmem_alloc_wired()` in C: `kmem_alloc_wired_flags()` with
/// `VM_PAGE_HIGHMEM`.
pub(crate) fn kmem_alloc_wired(
    map: NonNull<VmMap>,
    size: VmSize,
) -> Result<VmOffset, Error> {
    kmem_alloc_wired_flags(map, size, VM_PAGE_HIGHMEM)
}

/// `kmem_map_aligned_table()` in C: map a physical table at a kernel address
/// with the physical address's in-page offset.
pub(crate) fn kmem_map_aligned_table(
    map: NonNull<VmMap>,
    phys_address: VmOffset,
    size: VmSize,
    mode: c_int,
) -> Option<NonNull<c_void>> {
    let into_page = phys_address % PAGE_SIZE;
    let nearest_page = phys_address.wrapping_sub(into_page);
    let size = round_page(size.wrapping_add(into_page));

    let virt_addr = kmem_alloc_wired(map, size).ok()?;

    // SAFETY: `virt_addr` is the wired region just allocated, and
    // `nearest_page..nearest_page + size` is the physical range it stands for.
    unsafe {
        pmap_map_bd(
            virt_addr,
            nearest_page,
            nearest_page.wrapping_add(size),
            VmProt::from_bits(mode),
        )
    };

    // SAFETY: the C casts the integer address back to a pointer, and
    // `virt_addr + into_page` is inside the mapping just made.
    Some(unsafe {
        NonNull::new_unchecked(with_exposed_provenance_mut::<c_void>(
            virt_addr.wrapping_add(into_page),
        ))
    })
}

/// `kmem_alloc_pageable()` in C: reserve pageable space in the kernel map.
pub(crate) fn kmem_alloc_pageable(
    map: &mut VmMap,
    size: VmSize,
) -> Result<VmOffset, Error> {
    let mut addr = map.hdr.links.start;

    let entered = map.enter(EnterRequest {
        address: &mut addr,
        size: round_page(size),
        mask: 0,
        anywhere: true,
        object: ptr::null_mut(),
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    });

    match entered {
        Ok(()) => Ok(addr),
        Err(error) => {
            // The C `printf_once` guards a diagnostic; a relaxed flag is
            // enough because the worst case is printing it twice.
            static PRINTED: AtomicBool = AtomicBool::new(false);
            if !PRINTED.swap(true, Ordering::Relaxed) {
                // SAFETY: the format is a literal with the `%p` and `%s`
                // arguments it reads; `map.name` is the NUL-terminated string
                // the C passed.
                unsafe {
                    printf(
                        c"no more room for kmem_alloc_pageable in %p (%s)\n"
                            .as_ptr(),
                        ptr::from_ref(map).cast_mut().cast::<c_void>(),
                        map.name,
                    )
                };
            }
            Err(error)
        }
    }
}

/// `kmem_free()` in C: release a region a `kmem_alloc*` call made.
pub(crate) fn kmem_free(
    map: &mut VmMap,
    addr: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    map.remove(trunc_page(addr), round_page(addr.wrapping_add(size)))
}

/// `kmem_submap()` in C: build `map` as a submap of `parent`.
pub(crate) fn kmem_submap(
    map: &mut VmMap,
    parent: NonNull<VmMap>,
    size: VmSize,
) -> Result<(VmOffset, VmOffset), Error> {
    let size = round_page(size);

    // SAFETY: `vm_submap_object` is the boot placeholder and is live for the
    // life of the kernel.
    let object = unsafe { vm_submap_object };
    // SAFETY: the parent's new entry holds the reference taken here, as the C
    // does before `vm_map_enter`.
    unsafe { vm_object_reference(object) };

    // SAFETY: the caller promises a valid, unlocked parent map.
    let parent = unsafe { &mut *parent.as_ptr() };
    let mut addr = parent.hdr.links.start;
    parent.enter(EnterRequest {
        address: &mut addr,
        size,
        mask: 0,
        anywhere: true,
        object,
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    })?;

    let pmap = parent.pmap;
    // SAFETY: the parent owns a reference to `pmap`, and the submap must hold
    // its own.
    unsafe { pmap_reference(pmap) };
    VmMap::setup(map, pmap, addr, addr.wrapping_add(size));

    // The caller promises the parent is a live map and `map` the storage just
    // set up, which is what `vm_map_submap()` requires.
    parent.submap(addr, addr.wrapping_add(size), ptr::from_mut(map))?;

    Ok((addr, addr.wrapping_add(size)))
}

/// `kmem_init()` in C: initialize the kernel map's address range.
pub(crate) fn kmem_init(
    map: NonNull<VmMap>,
    pmap: *mut Pmap,
    start: VmOffset,
    end: VmOffset,
) -> Result<(), Error> {
    // SAFETY: the caller passes the kernel's own map storage before anything
    // else uses it, and `pmap` is the boot pmap.
    unsafe {
        VmMap::setup(&mut *map.as_ptr(), pmap, VM_MIN_KERNEL_ADDRESS, end)
    };

    if start == VM_MIN_KERNEL_ADDRESS {
        return Ok(());
    }

    let mut addr = VM_MIN_KERNEL_ADDRESS;
    // SAFETY: the map was just set up and is unlocked.
    unsafe {
        (*map.as_ptr()).enter(EnterRequest {
            address: &mut addr,
            size: start.wrapping_sub(VM_MIN_KERNEL_ADDRESS),
            mask: 0,
            anywhere: true,
            object: ptr::null_mut(),
            offset: 0,
            needs_copy: false,
            cur_protection: VmProt::READ | VmProt::WRITE,
            max_protection: VmProt::ALL,
            inheritance: VmInherit::COPY,
        })
    }
}

/// `kmem_io_map_deallocate()` in C: drop the mapping `kmem_io_map_copyout()`
/// established.
pub(crate) fn kmem_io_map_deallocate(
    map: &mut VmMap,
    addr: VmOffset,
    size: VmSize,
) {
    let end = addr.wrapping_add(size);
    // SAFETY: the caller promises the range belongs to this map, whose pmap
    // is live.
    unsafe { pmap_remove(map.pmap, addr, end) };
    let _ = map.remove(addr, end);
}
