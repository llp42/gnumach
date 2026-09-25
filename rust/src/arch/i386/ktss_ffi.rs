// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/ktss.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the kernel TSS module, one adapter per symbol
//! `i386/i386/ktss.c` used to define and `i386/i386/ktss.h` declares.

use crate::arch::i386::ktss;
use core::ffi::c_int;

/// `ktss_init()` of <i386/ktss.h>.
///
/// # Safety
///
/// Runs once on the boot CPU, before any other CPU starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ktss_init() {
    ktss::ktss_init();
}

/// `ap_ktss_init()` of <i386/ktss.h>.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reported, whose table set `mp_desc_init()`
/// has built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_ktss_init(cpu: c_int) {
    ktss::ap_ktss_init(cpu);
}
