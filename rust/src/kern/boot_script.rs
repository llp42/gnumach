// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/boot_script.c and kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The boot-script parser's error names, which `kern/boot_script.c` used to
//! describe in `boot_script_error_string()`.

use core::ffi::{CStr, c_char, c_int};
use core::ptr;

/// The errors the boot-script parser reports, named after the `BOOT_SCRIPT_*`
/// codes of <kern/boot_script.h>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `BOOT_SCRIPT_NOMEM`.
    NoMem,
    /// `BOOT_SCRIPT_SYNTAX_ERROR`.
    SyntaxError,
    /// `BOOT_SCRIPT_INVALID_ASG`.
    InvalidAsg,
    /// `BOOT_SCRIPT_MACH_ERROR`.
    MachError,
    /// `BOOT_SCRIPT_UNDEF_SYM`.
    UndefSym,
    /// `BOOT_SCRIPT_EXEC_ERROR`.
    ExecError,
    /// `BOOT_SCRIPT_INVALID_SYM`.
    InvalidSym,
    /// `BOOT_SCRIPT_BAD_TYPE`.
    BadType,
}

impl Error {
    /// Returns the [`Error`] `code` names, or [`None`] when it names none of
    /// them.
    #[must_use]
    pub const fn from_code(code: c_int) -> Option<Self> {
        match code {
            1 => Some(Self::NoMem),
            2 => Some(Self::SyntaxError),
            3 => Some(Self::InvalidAsg),
            4 => Some(Self::MachError),
            5 => Some(Self::UndefSym),
            6 => Some(Self::ExecError),
            7 => Some(Self::InvalidSym),
            8 => Some(Self::BadType),
            _ => None,
        }
    }

    /// Returns the description the C printed for this error.
    #[must_use]
    pub const fn message(self) -> &'static CStr {
        match self {
            Self::NoMem => c"no memory",
            Self::SyntaxError => c"syntax error",
            Self::InvalidAsg => c"invalid variable in assignment",
            Self::MachError => c"mach error",
            Self::UndefSym => c"undefined symbol",
            Self::ExecError => c"exec error",
            Self::InvalidSym => c"invalid variable in expression",
            Self::BadType => c"invalid value type",
        }
    }
}

/// `boot_script_error_string()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn boot_script_error_string(err: c_int) -> *mut c_char {
    match Error::from_code(err) {
        Some(error) => error.message().as_ptr().cast_mut(),
        None => ptr::null_mut(),
    }
}
