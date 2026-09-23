// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/kmsg.c:
//   Copyright (C) 1998, 1999, 2007 Free Software Foundation, Inc.
//   Written by OKUJI Yoshinori.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel message device's status query, which `device/kmsg.c` used
//! to define and <device/kmsg.h> declares.
//!
//! `kmsggetstat()` serves one flavor, `DEV_GET_SIZE`, reporting a
//! device size it does not know and a record size of one byte, the
//! stream interface the device provides.  The rest of the file -- the
//! buffer, the lock and the read/write plumbing -- stays C.

use crate::arch::i386::io_req::{
    DEV_GET_SIZE, DEV_GET_SIZE_COUNT, DEV_GET_SIZE_DEVICE_SIZE,
    DEV_GET_SIZE_RECORD_SIZE, DevT,
};
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_int, c_uint};

/// The `DEV_GET_SIZE` reply: the device size is unknown (zero), and the
/// record size is one, which marks the device as sequential.
const GET_SIZE_REPLY: [c_int; DEV_GET_SIZE_COUNT as usize] = {
    let mut reply = [0; DEV_GET_SIZE_COUNT as usize];
    reply[DEV_GET_SIZE_DEVICE_SIZE] = 0;
    reply[DEV_GET_SIZE_RECORD_SIZE] = 1;
    reply
};

/// The status reply for `flavor`, or [`None`] for a flavor the device
/// does not serve.
fn getstat(
    flavor: c_uint,
) -> Option<([c_int; DEV_GET_SIZE_COUNT as usize], u32)> {
    match flavor {
        DEV_GET_SIZE => Some((GET_SIZE_REPLY, DEV_GET_SIZE_COUNT)),
        _ => None,
    }
}

/// Device status query.  `kmsggetstat()` in C.
///
/// # Safety
///
/// For `DEV_GET_SIZE`, the only flavor the C served, `data` must be
/// writable for `DEV_GET_SIZE_COUNT` integers and `count` must be
/// writable; the C wrote both and read neither.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmsggetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    match getstat(flavor) {
        Some((reply, n)) => {
            // SAFETY: the caller promises room for `n` integers behind
            // `data` and a writable `count`; `reply` has `n` elements.
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
