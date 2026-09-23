// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_subr.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The console-print trap of `kern/syscall_subr.c`, declared in
//! <kern/syscall_subr.h>.
//!
//! [`mach_print`] is a debugging tool that bypasses messaging
//! altogether; it is trap -30 in <mach/syscall_sw.h>.
//!
//! The rest of `kern/syscall_subr.c` stays C: every other routine
//! there runs inside the scheduler, reading `struct thread` fields
//! and switching stacks.

use crate::glue;
use core::ffi::c_char;

/// Displays the NUL-terminated `s` on the Mach console.
/// `mach_print()` of kern/syscall_subr.c.
///
/// # Safety
///
/// `s` must point at a NUL-terminated string that stays readable for
/// the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_print(s: *const c_char) {
    // SAFETY: the caller promises a readable NUL-terminated `s`, and
    // the format string is the C call's literal, which matches it.
    unsafe { glue::printf(c"%s".as_ptr(), s) };
}
