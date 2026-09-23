// SPDX-License-Identifier: CMU-Mach
// Derived from i386/intel/pmap.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The pmap leaves, which `i386/intel/pmap.c` used to define and
//! `i386/intel/pmap.h` declares: the advisory pageable call and the boot-time
//! unmapping of page zero.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::vm::types::Pmap;
use core::arch::asm;
use core::ffi::c_int;
use core::ptr::NonNull;

/// `LINEAR_DS` of <i386/gdt.h>: the flat data selector `gdt_fill()` builds
/// with base zero, which makes an offset in it a linear address.
const LINEAR_DS: u16 = 0x38;

/// `KERNEL_DS` of <i386/gdt.h>: the kernel data selector, which the TLB
/// invalidation puts back in `%es`.
const KERNEL_DS: u16 = 0x10;

/// Invalidate the TLB entry for one page, given its linear address.
fn invalidate_linear_page(linear: VmOffset) {
    // SAFETY: The two selectors are the architectural constants of
    // <i386/gdt.h>, and the kernel's GDT describes them from `gdt_init()` on,
    // so neither load can fault.
    unsafe {
        asm!(
            "movw {linear_ds:x}, %es",
            "invlpg %es:({addr})",
            "movw {kernel_ds:x}, %es",
            addr = in(reg) linear,
            linear_ds = in(reg) LINEAR_DS,
            kernel_ds = in(reg) KERNEL_DS,
            options(att_syntax, nostack, preserves_flags),
        );
    }
}

/// `pmap_pageable()` in i386/intel/pmap.c: advisory, and empty in the C
/// too, since `pmap_enter()` already learns whether a page is wired.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_pageable(
    _pmap: *mut Pmap,
    _start: VmOffset,
    _end: VmOffset,
    _pageable: c_int,
) {
}

/// Unmap the page at virtual address zero so that a null reference faults.
fn unmap_page_zero() {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares, and
    // this format has no conversion specifier.
    unsafe {
        glue::printf(
            c"Unmapping the zero page.  Some BIOS functions may not be working any more.\n"
                .as_ptr(),
        )
    };
    // SAFETY: `kernel_pmap` is the kernel's live pmap from `pmap_bootstrap()`
    // on, and `pmap_pte()` returns null rather than something invalid for an
    // address with no page-table entry.
    let pte = unsafe { glue::pmap_pte(glue::kernel_pmap, 0) };
    let Some(pte) = NonNull::new(pte) else {
        return;
    };
    // SAFETY: `pmap_pte()` returned a non-null pointer to a live page table
    // entry for address zero.
    unsafe { pte.as_ptr().cast::<c_int>().write(0) };
    invalidate_linear_page(0);
}

/// Unmap the page at virtual address zero so that a null reference faults.
///
/// # Safety
///
/// The kernel pmap must be initialized, so that `pmap_pte()` can walk it, and
/// nothing may depend on the zero page's mapping afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_unmap_page_zero() {
    unmap_page_zero();
}
