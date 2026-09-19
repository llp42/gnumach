// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `memcpy()`, which `i386/i386/strings.c` used to define.

use core::ffi::c_void;

/// Copy `n` bytes from `src` to `dest` and return `dest`.
///
/// # Safety
///
/// `src` must be valid for reads of `n` bytes, `dest` valid for writes of
/// `n` bytes, and the two regions must not overlap -- `memmove()` is the
/// one that allows that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(
    dest: *mut c_void,
    src: *const c_void,
    n: usize,
) -> *mut c_void {
    let dest = dest.cast::<u8>();
    let src = src.cast::<u8>();

    for i in 0..n {
        // SAFETY: the caller promises both are valid for `n` bytes.
        unsafe { *dest.add(i) = *src.add(i) };
    }

    dest.cast::<c_void>()
}
