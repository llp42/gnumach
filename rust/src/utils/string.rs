// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The string and memory routines, which `i386/i386/strings.c` and
//! `kern/strings.c` used to define.

use core::ffi::{c_char, c_int, c_void};
use core::{ptr, slice};

/// Copy `n` bytes from `s2` to `s1` and return `s1`.
///
/// # Safety
///
/// `s2` must be valid for reads of `n` bytes, `s1` valid for writes of `n`
/// bytes, and the two regions must not overlap -- `memmove()` is the one that
/// allows that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(
    s1: *mut c_void,
    s2: *const c_void,
    n: usize,
) -> *mut c_void {
    let s1 = s1.cast::<u8>();
    let s2 = s2.cast::<u8>();

    for i in 0..n {
        // SAFETY: the caller promises both are valid for `n` bytes.
        unsafe { *s1.add(i) = *s2.add(i) };
    }

    s1.cast::<c_void>()
}

/// Move `n` bytes from `s2` to `s1` and return `s1`.
///
/// # Safety
///
/// `s2` must be valid for reads of `n` bytes and `s1` valid for writes of `n`
/// bytes; the two regions may overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(
    s1: *mut c_void,
    s2: *const c_void,
    n: usize,
) -> *mut c_void {
    let s1 = s1.cast::<u8>();
    let s2 = s2.cast::<u8>();

    if s1.addr() <= s2.addr() {
        for i in 0..n {
            // SAFETY: the caller promises both regions are valid for `n`
            // bytes; forward, each read happens before its write.
            unsafe { *s1.add(i) = *s2.add(i) };
        }
    } else {
        for i in (0..n).rev() {
            // SAFETY: the caller promises both regions are valid for `n`
            // bytes; backward, each read happens before its write.
            unsafe { *s1.add(i) = *s2.add(i) };
        }
    }

    s1.cast::<c_void>()
}

/// Compare the first `n` bytes at `s1` and `s2`.
///
/// # Safety
///
/// `s1` and `s2` must both be valid for reads of `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(
    s1: *const c_void,
    s2: *const c_void,
    n: usize,
) -> c_int {
    if n == 0 {
        return 0;
    }

    // SAFETY: the caller promises `s1` is valid for reads of `n` bytes.
    let s1 = unsafe { slice::from_raw_parts(s1.cast::<u8>(), n) };
    // SAFETY: the caller promises `s2` is valid for reads of `n` bytes.
    let s2 = unsafe { slice::from_raw_parts(s2.cast::<u8>(), n) };

    for (&a, &b) in s1.iter().zip(s2) {
        if a != b {
            return c_int::from(a) - c_int::from(b);
        }
    }

    0
}

/// Fill the `n` bytes at `s` with the low byte of `c` and return `s`.
///
/// # Safety
///
/// `s` must be valid for writes of `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(
    s: *mut c_void,
    c: c_int,
    n: usize,
) -> *mut c_void {
    let s = s.cast::<u8>();
    let byte = c as u8;

    for i in 0..n {
        // SAFETY: the caller promises `s` is valid for `n` bytes.
        unsafe { *s.add(i) = byte };
    }

    s.cast::<c_void>()
}

/// Return a pointer to the first occurrence of `c` in the NUL-terminated
/// string at `s`, or null.
///
/// # Safety
///
/// `s` must point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let byte = c as u8;
    let mut i = 0;

    loop {
        // SAFETY: the caller promises `s` is NUL-terminated, so the walk stops
        // inside the string.
        let b = unsafe { *s.add(i) } as u8;
        if b == byte {
            // SAFETY: `i` is inside the string.
            return unsafe { s.add(i).cast_mut() };
        }
        if b == 0 {
            return ptr::null_mut();
        }
        i += 1;
    }
}

/// Copy the NUL-terminated string at `s2`, terminator included, to `s1`, and
/// return `s1`.
///
/// # Safety
///
/// `s2` must point to a NUL-terminated string and `s1` to room for it,
/// terminator included; the two must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(
    s1: *mut c_char,
    s2: *const c_char,
) -> *mut c_char {
    let mut i = 0;

    loop {
        // SAFETY: the caller promises `s2` is NUL-terminated and `s1` has room
        // for it, so the walk stops inside both.
        let byte = unsafe { *s2.add(i) };
        // SAFETY: as above.
        unsafe { *s1.add(i) = byte };
        if byte == 0 {
            break;
        }
        i += 1;
    }

    s1
}

/// Copy up to `n` bytes of the NUL-terminated string at `s2` to `s1`, padding
/// with NULs when `s2` is shorter, and return `s1`.
///
/// # Safety
///
/// `s2` must point to a NUL-terminated string and `s1` to room for `n` bytes;
/// the two must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(
    s1: *mut c_char,
    s2: *const c_char,
    n: usize,
) -> *mut c_char {
    let mut i = 0;

    while i < n {
        // SAFETY: the caller promises `s2` is NUL-terminated and `s1` has room
        // for `n` bytes.
        let byte = unsafe { *s2.add(i) };
        // SAFETY: as above.
        unsafe { *s1.add(i) = byte };
        i += 1;
        if byte == 0 {
            break;
        }
    }

    while i < n {
        // SAFETY: the caller promises `s1` has room for `n` bytes.
        unsafe { *s1.add(i) = 0 };
        i += 1;
    }

    s1
}

/// Split the string at `*strp` on the first byte found in `delim`.
///
/// # Safety
///
/// `strp` must point to a valid pointer to a writable NUL-terminated string,
/// or to null; `delim` must point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strsep(
    strp: *mut *mut c_char,
    delim: *const c_char,
) -> *mut c_char {
    // SAFETY: the caller promises `strp` points to a valid pointer.
    let s = unsafe { *strp };
    if s.is_null() {
        return ptr::null_mut();
    }

    let mut i = 0;
    loop {
        // SAFETY: the caller promises `s` is NUL-terminated, so the walk stops
        // inside the string.
        let byte = unsafe { *s.add(i) };
        if byte == 0 {
            // SAFETY: the caller promises `strp` points to a valid pointer.
            unsafe { *strp = ptr::null_mut() };
            return s;
        }

        let mut j = 0;
        loop {
            // SAFETY: the caller promises `delim` is NUL-terminated.
            let d = unsafe { *delim.add(j) };
            if d == 0 {
                break;
            }
            if d == byte {
                // SAFETY: `i` is inside the writable string.
                unsafe { *s.add(i) = 0 };
                // SAFETY: the caller promises `strp` points to a valid
                // pointer, and `i + 1` is inside the string.
                unsafe { *strp = s.add(i + 1) };
                return s;
            }
            j += 1;
        }

        i += 1;
    }
}

/// Compare the NUL-terminated strings at `s1` and `s2`.
///
/// # Safety
///
/// `s1` and `s2` must point to NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(
    s1: *const c_char,
    s2: *const c_char,
) -> c_int {
    let mut i = 0;

    loop {
        // SAFETY: the caller promises both strings are NUL-terminated, so the
        // walk stops inside both.
        let (a, b) = unsafe { (*s1.add(i) as u8, *s2.add(i) as u8) };
        if a != b {
            return c_int::from(a) - c_int::from(b);
        }
        if a == 0 {
            return 0;
        }
        i += 1;
    }
}

/// Compare at most `n` bytes of the NUL-terminated strings at `s1` and `s2`,
/// as `strcmp()` does.
///
/// # Safety
///
/// `s1` and `s2` must point to NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(
    s1: *const c_char,
    s2: *const c_char,
    n: usize,
) -> c_int {
    for i in 0..n {
        // SAFETY: the caller promises both strings are NUL-terminated, so the
        // walk stops inside both.
        let (a, b) = unsafe { (*s1.add(i) as u8, *s2.add(i) as u8) };
        if a != b {
            return c_int::from(a) - c_int::from(b);
        }
        if a == 0 {
            return 0;
        }
    }

    0
}

/// Return the number of bytes before the NUL terminator at `s`.
///
/// # Safety
///
/// `s` must point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0;

    // SAFETY: the caller promises `s` is NUL-terminated, so this walk stops
    // inside the string.
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }

    len
}

/// Return a pointer to the first occurrence of the string `s2` in the
/// NUL-terminated string at `s1`, or null.
///
/// # Safety
///
/// `s1` and `s2` must point to NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(
    s1: *const c_char,
    s2: *const c_char,
) -> *mut c_char {
    // SAFETY: the caller promises `s2` is NUL-terminated.
    let len = unsafe { strlen(s2) };
    if len == 0 {
        return s1.cast_mut();
    }

    let mut i = 0;
    // SAFETY: the caller promises `s1` is NUL-terminated, so the walk stops
    // inside the string.
    while unsafe { *s1.add(i) } != 0 {
        // SAFETY: `i` is inside the string and `s2` is NUL-terminated.
        if unsafe { strncmp(s1.add(i), s2, len) } == 0 {
            // SAFETY: `i` is inside the string.
            return unsafe { s1.add(i).cast_mut() };
        }
        i += 1;
    }

    ptr::null_mut()
}
