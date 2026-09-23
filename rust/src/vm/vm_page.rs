// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/vm_page.c:
//   Copyright (c) 2010-2014 Richard Braun.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The physical-page module, which `vm/vm_page.c` defines.

use core::ffi::{CStr, c_uint};

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_DMA32: c_uint = 2;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86_64")]
const SEG_HIGHMEM: c_uint = 3;

/// `VM_PAGE_SEG_DMA` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_DMA: c_uint = 0;
/// `VM_PAGE_SEG_DIRECTMAP` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_DIRECTMAP: c_uint = 1;
/// `VM_PAGE_SEG_DMA32`: the direct-map index, as the non-PAE i686 build
/// aliases it.
#[cfg(target_arch = "x86")]
const SEG_DMA32: c_uint = SEG_DIRECTMAP;
/// `VM_PAGE_SEG_HIGHMEM` of <machine/vm_param.h>.
#[cfg(target_arch = "x86")]
const SEG_HIGHMEM: c_uint = 2;

/// `vm_page_seg_name()` in C: the name of a physical segment index.
pub(crate) fn seg_name(seg_index: c_uint) -> Option<&'static CStr> {
    // The C's if-chain, not a match: DMA32 is the DIRECTMAP index on i686,
    // and a duplicate match arm would not compile.
    if seg_index == SEG_HIGHMEM {
        Some(c"HIGHMEM")
    } else if seg_index == SEG_DIRECTMAP {
        Some(c"DIRECTMAP")
    } else if seg_index == SEG_DMA32 {
        Some(c"DMA32")
    } else if seg_index == SEG_DMA {
        Some(c"DMA")
    } else {
        None
    }
}
