// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/phys.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/phys.c`, one adapter per symbol
//! <i386/intel/pmap.h> declares.

use crate::arch::i386::phys;
use crate::arch::types::VmOffset;
use core::ffi::c_int;

/// `kvtophys()` of <i386/pmap.h>: the physical address a kernel virtual
/// address maps to, or zero when it maps to nothing.
#[unsafe(no_mangle)]
pub extern "C" fn kvtophys(addr: VmOffset) -> VmOffset {
    phys::kvtophys(addr)
}

/// `pmap_zero_page()` of <i386/pmap.h>.
///
/// # Safety
///
/// `pa` must be the page-aligned address of a real physical frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_zero_page(pa: VmOffset) {
    // SAFETY: the caller promises the live frame.
    unsafe { phys::zero_page(pa) };
}

/// `pmap_copy_page()` of <i386/pmap.h>.
///
/// # Safety
///
/// `src` and `dst` must be the page-aligned addresses of real, disjoint
/// physical frames.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_copy_page(src: VmOffset, dst: VmOffset) {
    // SAFETY: the caller promises both live frames.
    unsafe { phys::copy_page(src, dst) };
}

/// `copy_to_phys()` of <i386/intel/pmap.h>.
///
/// # Safety
///
/// `src_addr_v` must be readable for `count` bytes, and `dst_addr_p` must
/// name `count` bytes of real physical memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn copy_to_phys(
    src_addr_v: VmOffset,
    dst_addr_p: VmOffset,
    count: c_int,
) {
    // SAFETY: the caller promises both live ranges.
    unsafe { phys::copy_to_phys(src_addr_v, dst_addr_p, count) };
}

/// `copy_from_phys()` of <i386/intel/pmap.h>.
///
/// # Safety
///
/// `src_addr_p` must name `count` bytes of real physical memory, and
/// `dst_addr_v` must be writable for `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn copy_from_phys(
    src_addr_p: VmOffset,
    dst_addr_v: VmOffset,
    count: c_int,
) {
    // SAFETY: the caller promises both live ranges.
    unsafe { phys::copy_from_phys(src_addr_p, dst_addr_v, count) };
}
