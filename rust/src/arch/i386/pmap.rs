// SPDX-License-Identifier: CMU-Mach
// Derived from i386/intel/pmap.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The pmap pageable advice, which `i386/intel/pmap.c` used to define
//! and `vm/pmap.h` declares.
//!
//! The rest of `pmap.c` stays C: it is the page tables themselves.

use crate::arch::types::VmOffset;
use crate::vm::types::Pmap;
use core::ffi::c_int;

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
