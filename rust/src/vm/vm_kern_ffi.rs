// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_kern.c and vm/vm_kern.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of Rust kernel memory management, one adapter per
//! symbol `vm/vm_kern.c` used to define and `vm/vm_kern.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::kernel_pmap;
use crate::kern::debug::kpanic;
use crate::vm::error::{KERN_INVALID_ARGUMENT, KERN_SUCCESS, kern_return};
use crate::vm::types::{VmInherit, VmObject, VmProt};
use crate::vm::vm_kern;
use crate::vm::vm_map::{VmMap, VmMapCopy};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

/// `projected_buffer_collect()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map, and no other thread
/// may modify its entry chain during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn projected_buffer_collect(map: *mut VmMap) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    kern_return(vm_kern::projected_buffer_collect(map))
}

/// `projected_buffer_in_range()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid map, and no other thread may modify
/// its entry chain during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn projected_buffer_in_range(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return c_int::from(false);
    };
    // SAFETY: the caller promises a valid map.
    c_int::from(vm_kern::projected_buffer_in_range(
        unsafe { &*map.as_ptr() },
        start,
        end,
    ))
}

/// `kmem_alloc_wired_flags()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addrp` writable storage for one
/// address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc_wired_flags(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
    flags: c_uint,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    match vm_kern::kmem_alloc_wired_flags(map, size, flags) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_alloc_wired()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addrp` writable storage for one
/// address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc_wired(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    match vm_kern::kmem_alloc_wired(map, size) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_map_aligned_table()` in C.
///
/// # Safety
///
/// `phys_address..phys_address + size` must be a physical range the kernel may
/// map, and the kernel map must already be initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_map_aligned_table(
    phys_address: VmOffset,
    size: VmSize,
    mode: c_int,
) -> *mut c_void {
    // SAFETY: `kernel_map` is the boot kernel map storage, live before any
    // caller of this routine.
    let map = unsafe { NonNull::new_unchecked(vm_kern::kernel_map) };
    vm_kern::kmem_map_aligned_table(map, phys_address, size, mode)
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `kmem_alloc_pageable()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addrp` writable storage for one
/// address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc_pageable(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { &mut *map };
    match vm_kern::kmem_alloc_pageable(map, size) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_free()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and the range must have come from a
/// `kmem_alloc*` call on it, as the C requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_free(
    map: *mut VmMap,
    addr: VmOffset,
    size: VmSize,
) {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { &mut *map };
    if vm_kern::kmem_free(map, addr, size).is_err() {
        kpanic!("kmem_free", "kmem_free");
    }
}

/// `kmem_submap()` in C.
///
/// # Safety
///
/// `map` must point at writable storage for a VM map, `parent` must be a
/// valid, unlocked map, and both out-pointers writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_submap(
    map: *mut VmMap,
    parent: *mut VmMap,
    min: *mut VmOffset,
    max: *mut VmOffset,
    size: VmSize,
) {
    // SAFETY: the caller promises a valid, non-null parent map.
    let parent = unsafe { NonNull::new_unchecked(parent) };
    // SAFETY: the caller promises writable storage for a map.
    let map = unsafe { &mut *map };
    match vm_kern::kmem_submap(map, parent, size) {
        Ok((min_addr, max_addr)) => {
            // SAFETY: the caller promises writable out-pointers.
            unsafe {
                min.write(min_addr);
                max.write(max_addr);
            }
        }
        Err(_) => kpanic!("kmem_submap", "kmem_submap"),
    }
}

/// `kmem_init()` in C.
///
/// # Safety
///
/// Must be called once, by the VM bootstrap, before any other user of the
/// kernel map; `kernel_map` and `kernel_pmap` must be the boot ones.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_init(start: VmOffset, end: VmOffset) {
    // SAFETY: `kernel_map` points at the boot map storage and `kernel_pmap`
    // is the boot pmap; both exist before `vm_mem_bootstrap` runs this.
    let (map, pmap) =
        unsafe { (NonNull::new_unchecked(vm_kern::kernel_map), kernel_pmap) };
    match vm_kern::kmem_init(map, pmap, start, end) {
        Ok(()) => (),
        Err(error) => {
            kpanic!(
                "kmem_init",
                "vm_map_enter failed ({})\n",
                error.as_kern_return()
            );
        }
    }
}

/// `kmem_io_map_deallocate()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addr..addr + size` the
/// page-aligned range `kmem_io_map_copyout()` established in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_io_map_deallocate(
    map: *mut VmMap,
    addr: VmOffset,
    size: VmSize,
) {
    // SAFETY: the caller promises a valid, non-null map.
    vm_kern::kmem_io_map_deallocate(unsafe { &mut *map }, addr, size);
}

/// `projected_buffer_allocate()` in C.
///
/// # Safety
///
/// `map` must be null or a valid, unlocked map; `kernel_p` and `user_p` must
/// be writable storage for one address each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn projected_buffer_allocate(
    map: *mut VmMap,
    size: VmSize,
    persistence: c_int,
    kernel_p: *mut VmOffset,
    user_p: *mut VmOffset,
    protection: VmProt,
    inheritance: VmInherit,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    match vm_kern::projected_buffer_allocate(
        map,
        size,
        persistence != 0,
        protection,
        inheritance,
    ) {
        Ok((kernel, user)) => {
            // SAFETY: the caller promises both out-pointers.
            unsafe {
                kernel_p.write(kernel);
                user_p.write(user);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `projected_buffer_map()` in C.
///
/// # Safety
///
/// `map` must be null or a valid, unlocked map, the kernel range must be an
/// existing mapping, and `user_p` must be writable storage for one address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn projected_buffer_map(
    map: *mut VmMap,
    kernel_addr: VmOffset,
    size: VmSize,
    user_p: *mut VmOffset,
    protection: VmProt,
    inheritance: VmInherit,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    match vm_kern::projected_buffer_map(
        map,
        kernel_addr,
        size,
        protection,
        inheritance,
    ) {
        Ok(user) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { user_p.write(user) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `projected_buffer_deallocate()` in C.
///
/// # Safety
///
/// `map` must be null or a valid map, and `start..end` a range
/// `projected_buffer_allocate()` or `projected_buffer_map()` established in
/// it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn projected_buffer_deallocate(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    kern_return(vm_kern::projected_buffer_deallocate(map, start, end))
}

/// `kmem_alloc()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addrp` writable storage for one
/// address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    match vm_kern::kmem_alloc(map, size) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_valloc()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `addrp` writable storage for one
/// address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_valloc(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    match vm_kern::kmem_valloc(map, size) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_alloc_aligned()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map, `size` must be a non-zero power of
/// two, and `addrp` writable storage for one address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc_aligned(
    map: *mut VmMap,
    addrp: *mut VmOffset,
    size: VmSize,
) -> c_int {
    if size & size.wrapping_sub(1) != 0 {
        kpanic!("kmem_alloc_aligned", "kmem_alloc_aligned");
    }

    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    match vm_kern::kmem_alloc_aligned(map, size) {
        Ok(addr) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { addrp.write(addr) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `kmem_alloc_pages()` in C.
///
/// # Safety
///
/// `object` must be a live object mapped into the kernel map, and its lock
/// must not be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_alloc_pages(
    object: *mut VmObject,
    offset: VmOffset,
    start: VmOffset,
    end: VmOffset,
    protection: VmProt,
    flags: c_uint,
) {
    // SAFETY: the caller's contract is the allocator's own.
    unsafe {
        vm_kern::alloc_pages(object, offset, start, end, protection, flags)
    };
}

/// `kmem_remap_pages()` in C.
///
/// # Safety
///
/// `object` must be a live object mapped into the kernel map, and its lock
/// must not be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_remap_pages(
    object: *mut VmObject,
    offset: VmOffset,
    start: VmOffset,
    end: VmOffset,
    protection: VmProt,
) {
    // SAFETY: the caller's contract is the remapper's own.
    unsafe { vm_kern::remap_pages(object, offset, start, end, protection) };
}

/// `kmem_io_map_copyout()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map, `copy` a live page-list copy, and the
/// three out-pointers writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_io_map_copyout(
    map: *mut VmMap,
    addr: *mut VmOffset,
    alloc_addr: *mut VmOffset,
    alloc_size: *mut VmSize,
    copy: *mut VmMapCopy,
    min_size: VmSize,
) -> c_int {
    let Some(copy) = NonNull::new(copy) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { &mut *map };
    match vm_kern::kmem_io_map_copyout(map, copy, min_size) {
        Ok(mapped) => {
            // SAFETY: the caller promises all three out-pointers.
            unsafe {
                addr.write(mapped.addr);
                alloc_addr.write(mapped.alloc_addr);
                alloc_size.write(mapped.alloc_size);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `copyinmap()` in C.
///
/// # Safety
///
/// `map` must be a valid map, and `fromaddr`/`toaddr` must be readable and
/// writable for `length` bytes as the C `copyin`/`memcpy` required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn copyinmap(
    map: *mut VmMap,
    fromaddr: *mut c_char,
    toaddr: *mut c_char,
    length: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid map and the byte ranges.
    unsafe { vm_kern::copyinmap(&*map, fromaddr, toaddr, length) }
}

/// `copyoutmap()` in C.
///
/// # Safety
///
/// Same contract as `copyinmap()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn copyoutmap(
    map: *mut VmMap,
    fromaddr: *mut c_char,
    toaddr: *mut c_char,
    length: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid map and the byte ranges.
    unsafe { vm_kern::copyoutmap(&*map, fromaddr, toaddr, length) }
}
