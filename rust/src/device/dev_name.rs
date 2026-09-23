// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_name.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device-name comparison and the empty device-table entries of
//! `device/dev_name.c`, declared in <device/dev_hdr.h> and <device/conf.h>.

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::types::VmOffset;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_char, c_int, c_uint, c_ushort, c_void};
use core::slice;

/// `nulldev_reset()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_reset(_dev: DevT) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `nulldev_open()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_open(
    _dev: DevT,
    _flags: c_int,
    _ior: *mut IoReq,
) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `nulldev_close()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_close(_dev: DevT, _flags: c_int) {}

/// `nulldev_read()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_read(_dev: DevT, _ior: *mut IoReq) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `nulldev_write()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_write(_dev: DevT, _ior: *mut IoReq) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `nulldev_getstat()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_getstat(
    _dev: DevT,
    _flavor: c_uint,
    _data: *mut c_int,
    _count: *mut c_uint,
) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// `nulldev_setstat()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_setstat(
    _dev: DevT,
    _flavor: c_uint,
    _data: *mut c_int,
    _count: c_uint,
) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// `nulldev_portdeath()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_portdeath(_dev: DevT, _port: VmOffset) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `nodev_async_in()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nodev_async_in(
    _dev: DevT,
    _port: *mut c_void,
    _x: c_int,
    _filter: *mut c_ushort,
    _j: c_uint,
) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// `nodev_info()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nodev_info(_dev: DevT, _a: c_int, _b: *mut c_int) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// `nomap()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn nomap(_dev: DevT, _off: VmOffset, _prot: c_int) -> VmOffset {
    VmOffset::MAX
}

/// `name_equal()` in C.
///
/// # Safety
///
/// `src` must be readable for `len` bytes when `len` is positive (nothing is
/// read when it is not), and `target` must be readable through the first byte
/// that differs from `src`, or through `target[len]` when the first `len`
/// bytes all match.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn name_equal(
    src: *const c_char,
    len: c_int,
    target: *const c_char,
) -> c_int {
    // The C's pre-decrement skips its loop for any len <= 0 and then asks only
    // whether target is empty; the clamp keeps that answer.
    let len = len.max(0) as usize;
    // SAFETY: the caller promises `len` readable bytes at `src` when positive.
    let src: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(src.cast::<u8>(), len) }
    };
    let target = target.cast::<u8>();
    for (i, &want) in src.iter().enumerate() {
        // SAFETY: the caller promises `target` readable through the first byte
        // that differs from `src`, and no byte before `i` has differed yet.
        if unsafe { *target.add(i) } != want {
            return 0;
        }
    }
    // SAFETY: every byte of the prefix matched, and the caller promises
    // `target[len]` readable for that case; it is the terminator the C tests.
    c_int::from(unsafe { *target.add(len) } == 0)
}
