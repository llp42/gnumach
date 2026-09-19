// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! A Rust panic, reported through the kernel's `Panic()`.

use core::ffi::{c_char, c_int};
use core::panic::PanicInfo;

use crate::glue::Panic;

/// A NUL-terminated `&str` in a stack buffer, truncated if it does not fit.
struct CStr<const N: usize> {
    buf: [u8; N],
}

impl<const N: usize> CStr<N> {
    fn new(s: &str) -> Self {
        let mut buf = [0; N];
        // One byte is held back for the terminator.
        let n = if s.len() < N { s.len() } else { N - 1 };
        buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        Self { buf }
    }

    fn as_ptr(&self) -> *const c_char {
        self.buf.as_ptr() as *const c_char
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let (file, line) = match info.location() {
        Some(loc) => (CStr::<128>::new(loc.file()), loc.line() as c_int),
        None => (CStr::<128>::new("<unknown>"), 0),
    };

    // SAFETY: both strings are NUL-terminated, and outlive a call that does
    // not return.
    unsafe {
        Panic(
            file.as_ptr(),
            line,
            c"mach_rs".as_ptr(),
            c"panic in the Rust half of the kernel".as_ptr(),
        )
    }
}
