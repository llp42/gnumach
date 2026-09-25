// SPDX-License-Identifier: CMU-Mach
// Derived from include/mach/kern_return.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Kernel return codes as Rust errors, from `mach/kern_return.h`.

use core::ffi::c_int;

pub const KERN_SUCCESS: c_int = 0;
pub const KERN_INVALID_ADDRESS: c_int = 1;
pub const KERN_PROTECTION_FAILURE: c_int = 2;
pub const KERN_NO_SPACE: c_int = 3;
pub const KERN_INVALID_ARGUMENT: c_int = 4;
pub const KERN_FAILURE: c_int = 5;
pub const KERN_RESOURCE_SHORTAGE: c_int = 6;
pub const KERN_NO_ACCESS: c_int = 8;
pub const KERN_MEMORY_ERROR: c_int = 10;
pub const KERN_INVALID_NAME: c_int = 15;
pub const KERN_INVALID_TASK: c_int = 16;
pub const KERN_INVALID_HOST: c_int = 22;
pub const KERN_MEMORY_PRESENT: c_int = 23;
pub const KERN_WRITE_PROTECTION_FAILURE: c_int = 24;
/// `MACH_SEND_INTERRUPTED` of <mach/message.h>: the pager wait an object copy
/// performs was interrupted.
pub const MACH_SEND_INTERRUPTED: c_int = 0x10000007;

/// The errors the VM map can report, named as in `mach/kern_return.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `KERN_SUCCESS`: no error.
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
    /// `KERN_INVALID_NAME`: a port the call needs is not valid.
    InvalidName,
    /// `KERN_INVALID_TASK`: the proxy call's IPC space is `IS_NULL`.
    InvalidTask,
    /// `KERN_INVALID_HOST`: the host argument is `HOST_NULL`.
    InvalidHost,
    /// `KERN_MEMORY_PRESENT`: the data supply found the page already present.
    MemoryPresent,
    /// `KERN_WRITE_PROTECTION_FAILURE`: the entry asks for `VM_PROT_NOTIFY`
    /// and the fault is a write.
    WriteProtectionFailure,
    /// `MACH_SEND_INTERRUPTED`: an object copy waiting for a pager was
    /// interrupted.
    SendInterrupted,
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
            Error::InvalidName => KERN_INVALID_NAME,
            Error::InvalidTask => KERN_INVALID_TASK,
            Error::InvalidHost => KERN_INVALID_HOST,
            Error::MemoryPresent => KERN_MEMORY_PRESENT,
            Error::WriteProtectionFailure => KERN_WRITE_PROTECTION_FAILURE,
            Error::SendInterrupted => MACH_SEND_INTERRUPTED,
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

/// The `Error` for a C `kern_return_t`.
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
        KERN_INVALID_NAME => Err(Error::InvalidName),
        KERN_INVALID_TASK => Err(Error::InvalidTask),
        KERN_INVALID_HOST => Err(Error::InvalidHost),
        KERN_MEMORY_PRESENT => Err(Error::MemoryPresent),
        KERN_WRITE_PROTECTION_FAILURE => Err(Error::WriteProtectionFailure),
        MACH_SEND_INTERRUPTED => Err(Error::SendInterrupted),
        _ => Err(Error::Failure),
    }
}
