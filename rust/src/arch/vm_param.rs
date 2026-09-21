// SPDX-License-Identifier: CMU-Mach
// Derived from i386/include/mach/i386/vm_param.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
// And from include/mach/vm_param.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Page geometry, from `i386/include/mach/i386/vm_param.h`, the header
//! both x86 kernels install as `<machine/vm_param.h>`.
//!
//! `PAGE_SHIFT` is the machine's own value, `I386_PGSHIFT` in the C;
//! the machine-independent `include/mach/vm_param.h` derives the other
//! two from it.
#![cfg(any(target_arch = "x86", target_arch = "x86_64"))]

use crate::arch::types::VmSize;

/// `PAGE_SHIFT` of <machine/vm_param.h>: `I386_PGSHIFT`, the number of
/// bits to shift for pages.
pub const PAGE_SHIFT: u32 = 12;

/// `PAGE_SIZE`: one page, `1 << PAGE_SHIFT` in the C.
pub const PAGE_SIZE: VmSize = 1 << PAGE_SHIFT;

/// `PAGE_MASK`: the in-page offset bits, `PAGE_SIZE - 1` in the C.
pub const PAGE_MASK: VmSize = PAGE_SIZE - 1;
