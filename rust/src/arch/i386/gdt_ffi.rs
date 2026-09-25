// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/gdt.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the GDT module, one adapter per symbol
//! `i386/i386/gdt.c` used to define and `i386/i386/gdt.h` declares.

use crate::arch::i386::gdt;
use core::ffi::c_int;

/// `gdt_init()` of <i386/gdt.h>.
///
/// # Safety
///
/// Runs once on the boot CPU, before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gdt_init() {
    gdt::gdt_init();
}

/// `ap_gdt_init()` of <i386/gdt.h>.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reported, whose table set `mp_desc_init()`
/// has built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_gdt_init(cpu: c_int) {
    gdt::ap_gdt_init(cpu);
}
