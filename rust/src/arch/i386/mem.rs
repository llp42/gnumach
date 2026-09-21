// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/mem.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `/dev/mem`: `memmmap()` of `i386/i386at/mem.c`.
//!
//! The only operation is to hand out physical pages for memory that is
//! not main RAM, so a user can look at the BIOS areas, the VGA window
//! and the like without going through the VM system.  `off` is a
//! byte address, and `-1` refuses it.

use crate::arch::i386::io_req::DevT;
use crate::arch::types::VmOffset;
use crate::arch::vm_param::PAGE_SHIFT;
use crate::glue;
use core::ffi::c_int;

/// `memmmap()` in C.
///
/// # Safety
///
/// Called from the `/dev/mem` device switch in `conf.c`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmmap(
    _dev: DevT,
    off: VmOffset,
    _prot: c_int,
) -> VmOffset {
    // SAFETY: a plain predicate over the boot memory map.
    if unsafe { glue::biosmem_addr_available(off) } != 0 {
        return VmOffset::MAX;
    }
    // i386_btop(): shift by I386_PGSHIFT.
    off >> PAGE_SHIFT
}
