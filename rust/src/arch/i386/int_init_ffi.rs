// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/int_init.c:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the interrupt gate module, one adapter per symbol
//! `i386/i386at/int_init.c` used to define and `i386/i386at/int_init.h`
//! declares.

use crate::arch::i386::int_init;
use core::ffi::c_int;

/// `int_init()` of <i386at/int_init.h>.
///
/// # Safety
///
/// Runs once on the boot CPU, before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn int_init() {
    int_init::int_init();
}

/// `ap_int_init()` of <i386at/int_init.h>.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reported, whose table set `mp_desc_init()`
/// has built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_int_init(cpu: c_int) {
    int_init::ap_int_init(cpu);
}
