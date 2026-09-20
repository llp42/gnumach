// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Kernel return codes as Rust errors, from `mach/kern_return.h`.
//!
//! The C `kern_return_t` is an `int`; the Rust core works in `Error`
//! and the `extern "C"` adapters convert, so no function behind them
//! returns an error code.

use core::ffi::c_int;

/// `KERN_SUCCESS`.
pub const KERN_SUCCESS: c_int = 0;
/// `KERN_INVALID_ADDRESS`.
pub const KERN_INVALID_ADDRESS: c_int = 1;
/// `KERN_PROTECTION_FAILURE`.
pub const KERN_PROTECTION_FAILURE: c_int = 2;
/// `KERN_NO_SPACE`.
pub const KERN_NO_SPACE: c_int = 3;
/// `KERN_INVALID_ARGUMENT`.
pub const KERN_INVALID_ARGUMENT: c_int = 4;
/// `KERN_FAILURE`.
pub const KERN_FAILURE: c_int = 5;
/// `KERN_RESOURCE_SHORTAGE`.
pub const KERN_RESOURCE_SHORTAGE: c_int = 6;
/// `KERN_NO_ACCESS`.
pub const KERN_NO_ACCESS: c_int = 8;
/// `KERN_MEMORY_ERROR`.
pub const KERN_MEMORY_ERROR: c_int = 10;

/// The errors the VM map can report, named as in
/// `mach/kern_return.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `KERN_SUCCESS`: no error.  `Result<_, Error>` never carries it,
    /// but the C conversion needs the code.
    Success,
    /// `KERN_INVALID_ADDRESS`.
    InvalidAddress,
    /// `KERN_PROTECTION_FAILURE`.
    ProtectionFailure,
    /// `KERN_NO_SPACE`.
    NoSpace,
    /// `KERN_INVALID_ARGUMENT`.
    InvalidArgument,
    /// `KERN_FAILURE`.
    Failure,
    /// `KERN_RESOURCE_SHORTAGE`.
    ResourceShortage,
    /// `KERN_NO_ACCESS`.
    NoAccess,
    /// `KERN_MEMORY_ERROR`.
    MemoryError,
}

impl Error {
    /// The `kern_return_t` a C caller sees.
    pub const fn as_kern_return(self) -> c_int {
        match self {
            Error::Success => KERN_SUCCESS,
            Error::InvalidAddress => KERN_INVALID_ADDRESS,
            Error::ProtectionFailure => KERN_PROTECTION_FAILURE,
            Error::NoSpace => KERN_NO_SPACE,
            Error::InvalidArgument => KERN_INVALID_ARGUMENT,
            Error::Failure => KERN_FAILURE,
            Error::ResourceShortage => KERN_RESOURCE_SHORTAGE,
            Error::NoAccess => KERN_NO_ACCESS,
            Error::MemoryError => KERN_MEMORY_ERROR,
        }
    }
}

/// The `kern_return_t` of a `Result`, with `KERN_SUCCESS` for `Ok`.
pub const fn kern_return(result: Result<(), Error>) -> c_int {
    match result {
        Ok(()) => KERN_SUCCESS,
        Err(error) => error.as_kern_return(),
    }
}

/// The `Error` for a C `kern_return_t`.  A code with no named error
/// becomes `Failure`, which a C caller sees unchanged.
pub const fn error_from_kern_return(code: c_int) -> Result<(), Error> {
    match code {
        KERN_SUCCESS => Ok(()),
        KERN_INVALID_ADDRESS => Err(Error::InvalidAddress),
        KERN_PROTECTION_FAILURE => Err(Error::ProtectionFailure),
        KERN_NO_SPACE => Err(Error::NoSpace),
        KERN_INVALID_ARGUMENT => Err(Error::InvalidArgument),
        KERN_RESOURCE_SHORTAGE => Err(Error::ResourceShortage),
        KERN_NO_ACCESS => Err(Error::NoAccess),
        KERN_MEMORY_ERROR => Err(Error::MemoryError),
        _ => Err(Error::Failure),
    }
}
