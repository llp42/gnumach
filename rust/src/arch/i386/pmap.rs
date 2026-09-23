// SPDX-License-Identifier: CMU-Mach
// Derived from i386/intel/pmap.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The pmap leaves, which `i386/intel/pmap.c` used to define and
//! `i386/intel/pmap.h` declares: the advisory pageable call and the
//! boot-time unmapping of page zero.
//!
//! The rest of `pmap.c` stays C: it is the page tables themselves.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::vm::types::Pmap;
use core::arch::asm;
use core::ffi::c_int;
use core::ptr::NonNull;

/// `LINEAR_DS` of <i386/gdt.h>: the flat data selector `gdt_fill()`
/// builds with base zero, which makes an offset in it a linear
/// address.
const LINEAR_DS: u16 = 0x38;

/// `KERNEL_DS` of <i386/gdt.h>: the kernel data selector, which the
/// TLB invalidation puts back in `%es`.
const KERNEL_DS: u16 = 0x10;

/// Invalidate the TLB entry for one page, given its linear address.
/// The constant single-page arm of the `INVALIDATE_TLB` macro in
/// `i386/intel/pmap.c`, which is `invlpg_linear()` of
/// <i386/proc_reg.h>: load `%es` with `LINEAR_DS`, execute `invlpg`,
/// and restore `%es` to `KERNEL_DS`.
fn invalidate_linear_page(linear: VmOffset) {
    // SAFETY: The two selectors are the architectural constants of
    // <i386/gdt.h>, and the kernel's GDT describes them from
    // `gdt_init()` on, so neither load can fault.  `invlpg` only
    // discards a TLB entry, and a linear address with no mapping is
    // not a fault.  The loads and the instruction touch neither the
    // stack nor the flags, and `%es` is restored before returning.
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

/// Make a range of a pmap pageable, or not, as asked.
/// `pmap_pageable()` in i386/intel/pmap.c.
///
/// The routine is advisory: `pmap_enter()` is told whether each page
/// is to be wired down, so the i386 pmap has nothing to record here
/// and the C body was empty too.  Every argument is therefore unused.
///
/// A page that is not pageable may not take a fault, so its page
/// table entry has to stay valid for the duration; that is the
/// caller's affair and this routine does not police it.
#[unsafe(no_mangle)]
pub extern "C" fn pmap_pageable(
    _pmap: *mut Pmap,
    _start: VmOffset,
    _end: VmOffset,
    _pageable: c_int,
) {
}

/// Unmap the page at virtual address zero so that a null reference
/// faults.  The body of `pmap_unmap_page_zero()` in
/// `i386/intel/pmap.c`.
fn unmap_page_zero() {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares,
    // and this format has no conversion specifier.
    unsafe {
        glue::printf(
            c"Unmapping the zero page.  Some BIOS functions may not be working any more.\n"
                .as_ptr(),
        )
    };
    // SAFETY: `kernel_pmap` is the kernel's live pmap from
    // `pmap_bootstrap()` on, and `pmap_pte()` returns null rather than
    // something invalid for an address with no page-table entry.
    let pte = unsafe { glue::pmap_pte(glue::kernel_pmap, 0) };
    let Some(pte) = NonNull::new(pte) else {
        return;
    };
    // SAFETY: `pmap_pte()` returned a non-null pointer to a live page
    // table entry for address zero.  The C wrote through an `int *`,
    // so this zeroes the entry's low 32 bits exactly as the C did.
    unsafe { pte.as_ptr().cast::<c_int>().write(0) };
    // The macro's range is the constant `PAGE_SIZE`, so its `invlpg`
    // arm runs and `flush_tlb()` is not reached.  Its argument is
    // `kvtolin(0)`, and `LINEAR_MIN_KERNEL_ADDRESS` equals
    // `VM_MIN_KERNEL_ADDRESS` on both targets, so `kvtolin` is the
    // identity and the linear address is zero.
    invalidate_linear_page(0);
}

/// Unmap the page at virtual address zero so that a null reference
/// faults.  The `pmap_unmap_page_zero()` entry of <i386/intel/pmap.h>,
/// which `i386/intel/pmap.c` used to define.
///
/// # Safety
///
/// The kernel pmap must be initialized, so that `pmap_pte()` can walk
/// it, and nothing may depend on the zero page's mapping afterwards.
/// The call is also an unsynchronised single-page update: the boot
/// path of `i386/i386at/model_dep.c` is its only caller and runs it
/// once, before another CPU uses the pmap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pmap_unmap_page_zero() {
    unmap_page_zero();
}
