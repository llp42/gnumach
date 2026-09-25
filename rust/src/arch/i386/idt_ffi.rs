// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/idt.c:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the IDT module, one adapter per symbol
//! `i386/i386/idt.c` used to define and `i386/i386at/idt.h` declares.

use crate::arch::i386::idt;
use core::ffi::c_int;

/// `idt_init()` of <i386at/idt.h>.
///
/// # Safety
///
/// Runs once on the boot CPU, before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idt_init() {
    idt::idt_init();
}

/// `ap_idt_init()` of <i386at/idt.h>.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reported, whose table set `mp_desc_init()`
/// has built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_idt_init(cpu: c_int) {
    idt::ap_idt_init(cpu);
}
