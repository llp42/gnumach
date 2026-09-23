// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/ioapic.c:
//   Copyright (C) 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The default interrupt handler, which `i386/i386at/ioapic.c` used to
//! define and `i386/i386/apic.h` declares.
//!
//! The rest of `ioapic.c` stays C: it programs the IOAPIC registers,
//! whose access window and route entries have no Rust mirror.

use crate::glue;
use core::ffi::c_int;

/// Report the interrupt on a pin that has no handler.  The body of
/// `intnull()` in `i386/i386at/ioapic.c`.
fn null(unit: c_int) {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares;
    // the one `%d` conversion takes the matching `c_int` vararg.
    unsafe { glue::printf(c"intnull(%d)\n".as_ptr(), unit) };
}

/// Report the interrupt on a pin that has no handler.  The `intnull()`
/// entry of <i386/i386/apic.h>, which `i386/i386at/ioapic.c` used to
/// define.
///
/// The APIC configuration is the only one the build accepts today;
/// the dormant non-APIC one has its own print-once `intnull()` in
/// `i386/i386/pic.c`.
///
/// # Safety
///
/// No precondition: the function only formats its argument, and the
/// C prototype's contract is likewise empty.  The marker records the
/// unchecked C entry point that `ivect[]` and the two C comparers
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn intnull(unit_dev: c_int) {
    null(unit_dev);
}
