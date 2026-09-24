// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/mem.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `/dev/mem`: `memmmap()` of `i386/i386at/mem.c`.

use crate::arch::i386::biosmem;
use crate::arch::i386::io_req::DevT;
use crate::arch::types::VmOffset;
use crate::arch::vm_param::PAGE_SHIFT;
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
    if !biosmem::addr_available(off) {
        return VmOffset::MAX;
    }
    off >> PAGE_SHIFT
}
