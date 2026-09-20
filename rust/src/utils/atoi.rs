// SPDX-License-Identifier: BSD-2-Clause
// Derived from util/atoi.c and util/atoi.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
//   Copyright 1988, 1989 by Intel Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `mach_atoi()`, which `util/atoi.c` used to define; `util/atoi.h`
//! keeps the C declaration and the `MACH_ATOI_DEFAULT` macro.
//!
//! The C interface returns the count of bytes consumed and stores the
//! number -- or `MACH_ATOI_DEFAULT` when there was none -- through an
//! out-parameter.  [`parse()`] is the Rust behind it: bytes in, count
//! and `Option` out.  Only the adapter knows the sentinel.

use core::ffi::c_int;
use core::slice;

/// `MACH_ATOI_DEFAULT` of <util/atoi.h>: the "no number" value the C
/// interface stores.
const MACH_ATOI_DEFAULT: c_int = -1;

/// Parse the leading decimal digits of `bytes`.
///
/// Returns how many bytes the digits occupied and, when there was at
/// least one, the number they spell; `None` means there were no digits.
/// The accumulator wraps, as the C original's did.
#[must_use]
fn parse(bytes: &[u8]) -> (usize, Option<c_int>) {
    let mut number: c_int = 0;
    let mut used = 0;

    while let Some(&byte) = bytes.get(used) {
        if !byte.is_ascii_digit() {
            break;
        }
        // A digit is 0..=9, so widening it to `c_int` loses nothing.
        number = number
            .wrapping_mul(10)
            .wrapping_add(c_int::from(byte - b'0'));
        used += 1;
    }

    let number = (used != 0).then_some(number);
    (used, number)
}

/// `mach_atoi()` of <util/atoi.h>: parse the leading decimal digits at
/// `s`, store the number or `MACH_ATOI_DEFAULT` at `nump`, and return
/// the number of bytes consumed.
///
/// # Safety
///
/// `s` must be readable up to and including the first non-digit byte --
/// a NUL-terminated string satisfies this; `nump` must be valid for a
/// write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_atoi(s: *const u8, nump: *mut c_int) -> c_int {
    let mut len = 0;

    // SAFETY: the caller promises a non-digit byte is reachable, so the
    // walk stops inside the readable region, and every digit read before
    // that is inside it too.
    while unsafe { *s.add(len) }.is_ascii_digit() {
        len += 1;
    }

    // SAFETY: the caller promises the bytes from `s` up to the first
    // non-digit are readable; they were just read one by one.
    let bytes = unsafe { slice::from_raw_parts(s, len) };

    let (used, number) = parse(bytes);

    // SAFETY: the caller promises `nump` is valid for a write.
    unsafe { *nump = number.unwrap_or(MACH_ATOI_DEFAULT) };

    // The C original converted `cp - original` to an `int`; `used` is
    // bounded by the readable byte count, so only a multi-gigabyte digit
    // run could differ.
    used as c_int
}
