// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_kern.c and vm/vm_kern.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of Rust kernel memory management, one adapter per
//! symbol `vm/vm_kern.c` used to define and `vm/vm_kern.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::{Panic, kernel_map, kernel_pmap};
use crate::vm::error::{KERN_INVALID_ARGUMENT, KERN_SUCCESS, kern_return};
use crate::vm::vm_kern;
use crate::vm::vm_map::VmMap;
use core::ffi::{c_int, c_uint, c_void};
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
    // SAFETY: `kernel_map` is the boot kernel map, built before any caller of
    // this routine.
    let map = unsafe { NonNull::new_unchecked(kernel_map.cast::<VmMap>()) };
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
        // SAFETY: `Panic` does not return; the file, function and message are
        // this port's, as the C `panic("kmem_free")` had them.
        unsafe {
            Panic(
                c"rust/src/vm/vm_kern_ffi.rs".as_ptr(),
                line!() as c_int,
                c"kmem_free".as_ptr(),
                c"kmem_free".as_ptr(),
            )
        };
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
        Err(_) => {
            // SAFETY: `Panic` does not return; the C panicked on either
            // failure with the same message.
            unsafe {
                Panic(
                    c"rust/src/vm/vm_kern_ffi.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_submap".as_ptr(),
                    c"kmem_submap".as_ptr(),
                )
            };
        }
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
    let (map, pmap) = unsafe {
        (
            NonNull::new_unchecked(kernel_map.cast::<VmMap>()),
            kernel_pmap,
        )
    };
    match vm_kern::kmem_init(map, pmap, start, end) {
        Ok(()) => (),
        Err(error) => {
            // SAFETY: `Panic` does not return; the format has the one
            // argument the C's `panic("vm_map_enter failed (%d)\n", rc)` had.
            unsafe {
                Panic(
                    c"rust/src/vm/vm_kern_ffi.rs".as_ptr(),
                    line!() as c_int,
                    c"kmem_init".as_ptr(),
                    c"vm_map_enter failed (%d)\n".as_ptr(),
                    error.as_kern_return(),
                )
            };
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
