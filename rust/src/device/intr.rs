// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/intr.c:
//   Copyright (c) 2010, 2011, 2016, 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt device's status query, which `device/intr.c` used to define
//! and <device/intr.h> declares.

use crate::arch::i386::io_req::DevT;
use crate::arch::i386::ioapic;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_int, c_uint};

/// `IRQGETPICMODE` of <device/irq_status.h>.
const IRQGETPICMODE: c_uint = 0;

/// The status reply for `flavor`, or [`None`] for a flavor the device does not
/// serve.
fn getstat(flavor: c_uint) -> Option<(c_int, u32)> {
    match flavor {
        IRQGETPICMODE => {
            // SAFETY: `pic_mode` is the machine global the APIC setup
            // initialized before the device layer starts and never wrote
            // again.
            let mode = unsafe { ioapic::pic_mode };
            Some((mode, 1))
        }
        _ => None,
    }
}

/// `irqgetstat()` in C.
///
/// # Safety
///
/// For `IRQGETPICMODE`, the only flavor the C served, `data` must be writable
/// for one integer and `count` must be writable; the C wrote both and read
/// neither.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn irqgetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    match getstat(flavor) {
        Some((mode, n)) => {
            // SAFETY: the caller promises one writable integer behind `data`
            // and a writable `count`.
            unsafe {
                *data = mode;
                *count = n;
            }
            Ok(DeviceSuccess::Success).as_io_return()
        }
        None => Err(DeviceError::InvalidOperation).as_io_return(),
    }
}
