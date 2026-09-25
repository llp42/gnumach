// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/mach_factor.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University
// Derived from kern/mach_factor.h:
//   Copyright (C) 2006, 2007 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entry `kern/mach_factor.c` used to define.

use crate::kern::mach_factor;

/// `compute_mach_factor()` of kern/mach_factor.c.
///
/// # Safety
///
/// The caller must run it where the C scheduler did, with no lock the
/// processor-set locks would nest under.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compute_mach_factor() {
    mach_factor::compute();
}
