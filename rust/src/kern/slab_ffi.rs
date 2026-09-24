// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/slab.c and kern/slab.h:
//   Copyright (c) 2011 Free Software Foundation.
//   Copyright (c) 2010, 2011 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the slab allocator, one adapter per symbol
//! `kern/slab.c` used to define and `kern/slab.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue;
use crate::kern::slab::{
    self, CacheInfo, CacheInitFlags, KmemCache, KmemCacheCtor,
};
use crate::kern::types::KernError;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::vm_kern;
use crate::vm::vm_map::VmMap;
use crate::vm::vm_map::round_page;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr::{self, NonNull, with_exposed_provenance_mut};
use core::slice;

/// `kmem_cache_init()` in C.
///
/// # Safety
///
/// `cache` must point at writable storage for a [`KmemCache`] that no other
/// thread can see yet, and `name` at a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_cache_init(
    cache: *mut KmemCache,
    name: *const c_char,
    obj_size: usize,
    align: usize,
    ctor: KmemCacheCtor,
    flags: c_int,
) {
    let Some(cache) = NonNull::new(cache) else {
        return;
    };
    let Some(name) = NonNull::new(name.cast_mut()) else {
        return;
    };

    // SAFETY: the caller promises the NUL-terminated string.
    let name = unsafe { CStr::from_ptr(name.as_ptr()) };

    // SAFETY: the caller promises writable, unshared cache storage.
    unsafe {
        (*cache.as_ptr()).init(
            name.to_bytes(),
            obj_size,
            align,
            ctor,
            CacheInitFlags::from_bits(flags),
        )
    };
}

/// `kmem_cache_alloc()` in C.
///
/// # Safety
///
/// `cache` must point at a live cache that [`kmem_cache_init`] built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_cache_alloc(cache: *mut KmemCache) -> VmOffset {
    let Some(cache) = NonNull::new(cache) else {
        return 0;
    };

    // SAFETY: the caller promises a live, initialized cache.
    let buf = unsafe { (*cache.as_ptr()).alloc() };

    buf.map_or(0, |buf| buf.as_ptr().addr())
}

/// `kmem_cache_free()` in C.
///
/// # Safety
///
/// `cache` must point at a live cache, and `obj` be a live allocation from
/// it that nothing uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmem_cache_free(
    cache: *mut KmemCache,
    obj: VmOffset,
) {
    let Some(cache) = NonNull::new(cache) else {
        return;
    };
    let Some(obj) = NonNull::new(with_exposed_provenance_mut::<u8>(obj))
    else {
        return;
    };

    // SAFETY: the caller's contract.
    unsafe { (*cache.as_ptr()).free(obj) };
}

/// `slab_bootstrap()` in C.
///
/// # Safety
///
/// Must be called once, before any cache is initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn slab_bootstrap() {
    slab::slab_bootstrap();
}

/// `slab_init()` in C.
///
/// # Safety
///
/// Must be called once, after [`slab_bootstrap`] and before
/// [`kalloc_init`], as the VM bootstrap does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn slab_init() {
    slab::slab_init();
}

/// `kalloc_init()` in C.
///
/// # Safety
///
/// Must be called once, after [`slab_init`] and before any allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kalloc_init() {
    slab::kalloc_init();
}

/// `slab_collect()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn slab_collect() {
    slab::slab_collect();
}

/// `slab_info()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn slab_info() {
    slab::slab_info();
}

/// `kalloc()` in C.
///
/// # Safety
///
/// The allocator must be initialized, and the returned allocation released
/// with [`kfree`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kalloc(size: VmSize) -> VmOffset {
    slab::kalloc(size).map_or(0, |buf| buf.as_ptr().addr())
}

/// `kfree()` in C.
///
/// # Safety
///
/// `data` must be null or a live allocation of `size` bytes from [`kalloc`]
/// that nothing uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfree(data: VmOffset, size: VmSize) {
    let Some(data) = NonNull::new(with_exposed_provenance_mut::<u8>(data))
    else {
        return;
    };

    // SAFETY: the caller's contract.
    unsafe { slab::kfree(data, size) };
}

/// `host_slab_info()` in <mach_debug/mach_debug.defs>.
///
/// # Safety
///
/// `info` must point at writable storage for one pointer, `info_cnt` at
/// writable storage for one count, and when `*info` is not used it must be
/// the caller's to overwrite.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_slab_info(
    host: *mut c_void,
    info: *mut *mut CacheInfo,
    info_cnt: *mut c_uint,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidHost);
    }

    let (Some(info), Some(info_cnt)) =
        (NonNull::new(info), NonNull::new(info_cnt))
    else {
        return c_int::from(KernError::InvalidArgument);
    };

    loop {
        let nr_caches = slab::nr_caches();
        let info_size = nr_caches as usize * size_of::<CacheInfo>();

        // `kalloc` reports a zero-size request as failure, as the C's
        // `info == NULL` check does.
        let Some(base) = slab::kalloc(info_size) else {
            return c_int::from(KernError::ResourceShortage);
        };

        // SAFETY: the allocation holds `nr_caches` records.
        let out = unsafe {
            slice::from_raw_parts_mut(
                base.as_ptr().cast::<CacheInfo>(),
                nr_caches as usize,
            )
        };

        let Some(count) = slab::collect(nr_caches, out) else {
            // SAFETY: `base` is the live allocation from above.
            unsafe { slab::kfree(base, info_size) };
            continue;
        };

        // SAFETY: the caller promises the count is readable.
        let room = unsafe { info_cnt.as_ptr().read() };

        if count <= room {
            // SAFETY: the caller promises `*info` writable for `room`
            // records, which covers `info_size` bytes, and the allocation is
            // readable for as many.
            unsafe {
                let dst = info.as_ptr().read();
                ptr::copy_nonoverlapping(
                    base.as_ptr(),
                    dst.cast::<u8>(),
                    info_size,
                );
            }
        } else {
            // SAFETY: `ipc_kernel_map` is the live kernel IPC map.
            let map = unsafe { &mut *glue::ipc_kernel_map.cast::<VmMap>() };

            let info_addr = match vm_kern::kmem_alloc_pageable(map, info_size)
            {
                Ok(addr) => addr,
                Err(error) => {
                    // SAFETY: `base` is the live allocation from above.
                    unsafe { slab::kfree(base, info_size) };
                    return error.as_kern_return();
                }
            };

            // SAFETY: the pageable region is `info_size` bytes long, as is
            // the allocation.
            unsafe {
                ptr::copy_nonoverlapping(
                    base.as_ptr(),
                    with_exposed_provenance_mut::<u8>(info_addr),
                    info_size,
                );
            }

            let total_size = round_page(info_size);

            if info_size < total_size {
                // SAFETY: the region is `total_size` bytes, and the range
                // starts at `info_size`.
                unsafe {
                    ptr::write_bytes(
                        with_exposed_provenance_mut::<u8>(
                            info_addr + info_size,
                        ),
                        0,
                        total_size - info_size,
                    );
                }
            }

            let copy = map.copyin(info_addr, info_size, true);

            // SAFETY: the caller promises `*info` writable for one pointer;
            // the C stored the copy there on both results.
            unsafe {
                info.as_ptr().write(match copy {
                    Ok(copy) => copy.cast::<CacheInfo>().as_ptr(),
                    Err(_) => ptr::null_mut(),
                });
            }
        }

        // SAFETY: the caller promises the count writable.
        unsafe { info_cnt.as_ptr().write(count) };
        // SAFETY: `base` is the live allocation from above.
        unsafe { slab::kfree(base, info_size) };

        return KERN_SUCCESS;
    }
}
