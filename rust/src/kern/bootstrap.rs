// SPDX-License-Identifier: CMU-Mach
// Derived from kern/bootstrap.c:
//   Copyright (c) 1992-1989 Carnegie Mellon University.
//   Copyright (c) 1995-1993 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The boot-script allocator callbacks, which `kern/bootstrap.c` used
//! to define for <kern/boot_script.h>.
//!
//! `kern/boot_script.c` is written to be reused outside the kernel, so
//! it asks its host for memory through these two names instead of
//! calling an allocator itself.  In GNU Mach the host is the kernel
//! and the allocator is `kalloc`/`kfree`, which is all these two do.
//! The rest of `kern/bootstrap.c`, the bootstrap task setup, stays C.

use crate::arch::types::VmSize;
use crate::glue;
use core::ffi::{c_uint, c_void};
use core::ptr;

/// Allocate `size` bytes for the boot-script parser, returning null on
/// failure.  `boot_script_malloc()` in C.
///
/// # Safety
///
/// `kalloc_init()` must have run.  The caller owns the returned
/// allocation and releases it with [`boot_script_free()`], passing the
/// same `size`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_malloc(size: c_uint) -> *mut c_void {
    // `c_uint` is 32 bits and `VmSize` is 32 or 64 on the two targets,
    // so the conversion is the widening the C call did implicitly and
    // cannot lose a bit.
    let size = size as VmSize;
    // SAFETY: the caller promises the allocator is up; `kalloc`
    // reports failure as address zero, which becomes a null pointer.
    let address = unsafe { glue::kalloc(size) };
    ptr::with_exposed_provenance_mut(address)
}

/// Release an allocation [`boot_script_malloc()`] returned.
/// `boot_script_free()` in C.
///
/// # Safety
///
/// `ptr` must name a live allocation of exactly `size` bytes made by
/// [`boot_script_malloc()`], and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_free(ptr: *mut c_void, size: c_uint) {
    // The same widening as above.
    let size = size as VmSize;
    // SAFETY: the caller promises a live allocation of `size` bytes
    // based at `ptr`.
    unsafe { glue::kfree(ptr.addr(), size) };
}
