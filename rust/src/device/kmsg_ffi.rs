// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/kmsg.c and device/kmsg.h:
//   Copyright (C) 1998, 1999, 2007 Free Software Foundation, Inc.
//   Written by OKUJI Yoshinori.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries of `device/kmsg.c`, declared in <device/kmsg.h>.

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::device::kmsg;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_int, c_uint};

/// `kmsggetstat()` in C.
///
/// # Safety
///
/// For `DEV_GET_SIZE`, the only flavor the C served, `data` must be writable
/// for `DEV_GET_SIZE_COUNT` integers and `count` must be writable; the C wrote
/// both and read neither.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsggetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    match kmsg::getstat(flavor) {
        Some((reply, n)) => {
            // SAFETY: the caller promises room for `n` integers behind `data`
            // and a writable `count`; `reply` has `n` elements.
            unsafe {
                for (i, value) in reply.iter().enumerate() {
                    *data.add(i) = *value;
                }
                *count = n;
            }
            Ok(DeviceSuccess::Success).as_io_return()
        }
        None => Err(DeviceError::InvalidOperation).as_io_return(),
    }
}

/// `kmsgopen()` in C.
///
/// # Safety
///
/// `dev`, `flag` and `ior` are ignored, as the C ignored them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsgopen(
    _dev: DevT,
    _flag: c_int,
    _ior: *mut IoReq,
) -> c_int {
    match kmsg::open() {
        Ok(success) => Ok(success).as_io_return(),
        Err(error) => Err(error).as_io_return(),
    }
}

/// `kmsgclose()` in C.
///
/// # Safety
///
/// `dev` and `flag` are ignored, as the C ignored them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsgclose(_dev: DevT, _flag: c_int) {
    kmsg::close();
}

/// `kmsgread()` in C.
///
/// # Safety
///
/// `ior` must be a live read request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsgread(_dev: DevT, ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    match unsafe { kmsg::read(ior) } {
        Ok(success) => Ok(success).as_io_return(),
        Err(error) => error.as_io_return(),
    }
}

/// `kmsg_putchar()` in C.
///
/// # Safety
///
/// No precondition: the C looked up its own state, and `c` is a plain value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsg_putchar(c: c_int) {
    kmsg::putchar(c);
}
