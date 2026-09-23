// SPDX-License-Identifier: CMU-Mach
// Derived from include/device/device_types.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `io_return_t` codes of <device/device_types.h>.
//!
//! C spelled every device result as an `int`: the success code, the
//! "request queued" code, or one of ten errors, in the same domain as
//! `kern_return_t`.  [`IoResult`] is that three-way result, and
//! [`IoResultExt`] is the only place that turns it back into the `int`
//! the `extern "C"` device entries return.

use core::ffi::c_int;

/// A failed device operation, as the `D_*` codes of
/// <device/device_types.h>.
///
/// The C called all of these `io_return_t`, in the same domain as
/// `kern_return_t`; only the codes below are device errors.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// `D_IO_ERROR`: hardware IO error.
    IoError = 2500,
    /// `D_WOULD_BLOCK`: would block, but `D_NOWAIT` set.
    WouldBlock = 2501,
    /// `D_NO_SUCH_DEVICE`: no such device.
    NoSuchDevice = 2502,
    /// `D_ALREADY_OPEN`: exclusive-use device already open.
    AlreadyOpen = 2503,
    /// `D_DEVICE_DOWN`: device has been shut down.
    DeviceDown = 2504,
    /// `D_INVALID_OPERATION`: bad operation for device.
    InvalidOperation = 2505,
    /// `D_INVALID_RECNUM`: invalid record (block) number.
    InvalidRecnum = 2506,
    /// `D_INVALID_SIZE`: invalid IO size.
    InvalidSize = 2507,
    /// `D_NO_MEMORY`: memory allocation failure.
    NoMemory = 2508,
    /// `D_READ_ONLY`: device cannot be written to.
    ReadOnly = 2509,
}

/// The result of a device operation, the Rust form of `io_return_t`.
///
/// `Ok(false)` is `D_SUCCESS`.  `Ok(true)` is `D_IO_QUEUED`: the
/// request is queued and the driver completes it, so the caller must
/// not.  `Err` carries the [`DeviceError`] the C returned as a code.
pub type IoResult = Result<bool, DeviceError>;

/// The C-shaped edge of [`IoResult`]: the two conversions between the
/// `io_return_t` an `extern "C"` device entry returns and the result
/// the core logic works in.
pub trait IoResultExt {
    /// Hand the [`IoResult`] back to C as an `io_return_t`.
    fn as_io_return(&self) -> c_int;

    /// The [`IoResult`] a C `io_return_t` denotes.
    ///
    /// [`None`] means `code` is outside the device codes: it is a
    /// `kern_return_t` the C passed through, and the caller keeps it
    /// raw rather than inventing a device error for it.
    fn from_io_return(code: c_int) -> Option<Self>
    where
        Self: Sized;
}

impl IoResultExt for IoResult {
    fn as_io_return(&self) -> c_int {
        match self {
            Ok(false) => 0,
            Ok(true) => -1,
            Err(e) => *e as i32,
        }
    }

    fn from_io_return(code: c_int) -> Option<Self> {
        match code {
            0 => Some(Ok(false)),
            -1 => Some(Ok(true)),
            2500 => Some(Err(DeviceError::IoError)),
            2501 => Some(Err(DeviceError::WouldBlock)),
            2502 => Some(Err(DeviceError::NoSuchDevice)),
            2503 => Some(Err(DeviceError::AlreadyOpen)),
            2504 => Some(Err(DeviceError::DeviceDown)),
            2505 => Some(Err(DeviceError::InvalidOperation)),
            2506 => Some(Err(DeviceError::InvalidRecnum)),
            2507 => Some(Err(DeviceError::InvalidSize)),
            2508 => Some(Err(DeviceError::NoMemory)),
            2509 => Some(Err(DeviceError::ReadOnly)),
            _ => None,
        }
    }
}
