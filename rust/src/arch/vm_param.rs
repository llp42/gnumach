// SPDX-License-Identifier: CMU-Mach
// Derived from i386/include/mach/i386/vm_param.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
// And from include/mach/vm_param.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Derived from i386/i386/vm_param.h and x86_64/x86_64/vm_param.h:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Page geometry, from `i386/include/mach/i386/vm_param.h`, the header both
//! x86 kernels install as `<machine/vm_param.h>`.
#![cfg(any(target_arch = "x86", target_arch = "x86_64"))]

use crate::arch::types::{VmOffset, VmSize};

/// `PAGE_SHIFT` of <machine/vm_param.h>: `I386_PGSHIFT`, the number of bits to
/// shift for pages.
pub const PAGE_SHIFT: u32 = 12;

/// `PAGE_SIZE`: one page, `1 << PAGE_SHIFT` in the C.
pub const PAGE_SIZE: VmSize = 1 << PAGE_SHIFT;

/// `PAGE_MASK`: the in-page offset bits, `PAGE_SIZE - 1` in the C.
pub const PAGE_MASK: VmSize = PAGE_SIZE - 1;

/// `KERNEL_STACK_SIZE` of <machine/vm_param.h>: the size and alignment of one
/// kernel stack, `1*I386_PGBYTES` in the C.
pub const KERNEL_STACK_SIZE: VmSize = PAGE_SIZE;

/// `VM_MAX_USER_ADDRESS` of <machine/vm_param.h>: the top of a user map.
///
/// The `--enable-user32` third value is out of scope for the Rust half.
#[cfg(target_pointer_width = "64")]
pub const VM_MAX_USER_ADDRESS: VmOffset = 0x8000_0000_0000;
#[cfg(target_pointer_width = "32")]
pub const VM_MAX_USER_ADDRESS: VmOffset = 0xc000_0000;
