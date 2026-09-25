// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/ldt.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the default LDT module, one adapter per symbol
//! `i386/i386/ldt.c` used to define and `i386/i386/ldt.h` declares.

use crate::arch::i386::ldt;
use core::ffi::c_int;

/// `ldt_init()` of <i386/ldt.h>.
///
/// # Safety
///
/// Runs once on the boot CPU, before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ldt_init() {
    ldt::ldt_init();
}

/// `ap_ldt_init()` of <i386/ldt.h>.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reported, whose table set `mp_desc_init()`
/// has built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_ldt_init(cpu: c_int) {
    ldt::ap_ldt_init(cpu);
}
