// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls.

use core::ffi::{c_char, c_int};

// `panic()` in <kern/debug.h> is a macro over `Panic()`.
unsafe extern "C" {
    pub fn Panic(
        file: *const c_char,
        line: c_int,
        fun: *const c_char,
        s: *const c_char,
        ...
    ) -> !;
}
