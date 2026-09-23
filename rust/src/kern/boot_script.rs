// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/boot_script.c and kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The boot-script parser's error names, which `kern/boot_script.c`
//! used to describe in `boot_script_error_string()`.
//!
//! The `BOOT_SCRIPT_*` codes of <kern/boot_script.h> are a fixed
//! domain, so [`Error`] is the enum they should always have been and
//! the exhaustive `match` in [`Error::message`] is what keeps a new
//! code from silently returning nothing.  The parser itself stays C.

use core::ffi::{CStr, c_char, c_int};
use core::ptr;

/// The errors the boot-script parser reports, named after the
/// `BOOT_SCRIPT_*` codes of <kern/boot_script.h>.
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
    /// Returns the [`Error`] `code` names, or [`None`] when it names
    /// none of them.
    ///
    /// Zero is success in the C and reaches the arm no code matches,
    /// which is why the C returned a null pointer for it.
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

/// Returns a string describing `err`, or null when no code matches.
/// `boot_script_error_string()` in C.
///
/// The C returns a `char *` into read-only storage and every caller
/// only prints it, so the returned pointer must not be written
/// through.
#[unsafe(no_mangle)]
pub extern "C" fn boot_script_error_string(err: c_int) -> *mut c_char {
    match Error::from_code(err) {
        // The C returned a string literal through a `char *` too; the
        // storage is read-only in both halves.
        Some(error) => error.message().as_ptr().cast_mut(),
        None => ptr::null_mut(),
    }
}
