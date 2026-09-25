// SPDX-License-Identifier: CMU-Mach
// Derived from device/cons.c and device/cons.h:
//   Copyright (c) 1988-1994, The University of Utah and
//   the Computer Systems Laboratory (CSL).  All rights reserved.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries of `device/cons.c`, declared in <device/cons.h>.

use crate::device::cons;
use core::ffi::{c_char, c_int};

/// `cninit()` in C.
///
/// # Safety
///
/// Called once, during the boot, after the device tables and `constab` exist
/// and before any console user runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cninit() {
    // SAFETY: the caller upholds the boot ordering.
    unsafe { cons::init() }
}

/// `cngetc()` in C: the blocking console read.
///
/// # Safety
///
/// The console is initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cngetc() -> c_int {
    // SAFETY: the caller promises the initialized console.
    unsafe { cons::getc(1) }
}

/// `cnmaygetc()` in C: the polling console read.
///
/// # Safety
///
/// The console is initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnmaygetc() -> c_int {
    // SAFETY: the caller promises the initialized console.
    unsafe { cons::getc(0) }
}

/// `cnputc()` in C.
///
/// # Safety
///
/// The console and ROM tables are the machine's, and the pending buffer is
/// this module's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnputc(c: c_char) {
    // SAFETY: the caller permits the console write.
    unsafe { cons::putc(c) }
}
