// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/phys.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel virtual-to-physical lookup, which `i386/i386/phys.c`
//! used to define and `i386/intel/pmap.h` declares.
//!
//! [`kvtophys()`] walks the kernel pmap's page tables and rebuilds
//! the address from the page frame number and the in-page offset.
//! The rest of `phys.c` stays C: the temporary map windows and the
//! physical copies need the pmap internals.

use crate::arch::types::VmOffset;
use crate::glue;
use core::ptr::NonNull;

/// `INTEL_OFFMASK` of `i386/intel/pmap.h`: the offset within a page.
const OFFMASK: VmOffset = 0xfff;

/// `INTEL_PTE_PFN`: the physical-page-number field of a page table
/// entry.
///
/// PAE is on in the x86_64 build, where the field is the upper 52
/// bits of the 64-bit entry, and off in the configured i386 build,
/// where it is the upper 20 bits of a 32-bit entry.  An i386
/// `--enable-pae` kernel takes a third value and is a known gap.
#[cfg(target_pointer_width = "64")]
const PFN_MASK: VmOffset = 0xffff_ffff_ffff_f000;
#[cfg(target_pointer_width = "32")]
const PFN_MASK: VmOffset = 0xffff_f000;

/// The physical address `addr` maps to, or zero when it maps to
/// nothing.  The body of `kvtophys()` in `i386/i386/phys.c`.
fn phys_of(addr: VmOffset) -> VmOffset {
    // SAFETY: `kernel_pmap` is the kernel's live pmap after
    // `pmap_bootstrap()`, and `pmap_pte()` returns null rather than
    // something invalid for an address without a mapping, as the C
    // check relies on.
    let pte = unsafe { glue::pmap_pte(glue::kernel_pmap, addr) };
    let Some(pte) = NonNull::new(pte) else {
        return 0;
    };
    // SAFETY: `pmap_pte()` returned a non-null pointer to a live page
    // table entry, which the C dereferenced the same way.
    let entry = unsafe { *pte.as_ptr() };
    (entry & PFN_MASK) | (addr & OFFMASK)
}

/// Convert a kernel virtual address to a physical address.
/// `kvtophys()` of `i386/intel/pmap.h`, which `i386/i386/phys.c`
/// defined.
///
/// Returns zero when `addr` has no mapping.
#[unsafe(no_mangle)]
pub extern "C" fn kvtophys(addr: VmOffset) -> VmOffset {
    phys_of(addr)
}
