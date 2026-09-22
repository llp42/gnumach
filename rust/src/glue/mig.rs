// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! MIG conversions between the Rust error codes and the C result
//! codes.
//!
//! [`KernError`] is one byte; the C edge speaks `c_int`.  Only the
//! widening lives here: a C code can name a value outside the byte
//! range, so narrowing belongs with the error policy that handles it.

use crate::kern::types::KernError;
use core::ffi::c_int;

impl From<KernError> for c_int {
    /// The `c_int` code of `error`.
    fn from(error: KernError) -> Self {
        c_int::from(error.as_u8())
    }
}
