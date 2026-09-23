// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/intr.c:
//   Copyright (c) 2010, 2011, 2016, 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt device's status query, which `device/intr.c` used to
//! define and <device/intr.h> declares.
//!
//! `irqgetstat()` serves one flavor, `IRQGETPICMODE`, which reports the
//! machine's interrupt controller through [`glue::pic_mode`]: the APIC
//! builds define that global in `i386/i386at/ioapic.c` and the PIC
//! build in `i386/i386/pic.c`.  The rest of the file -- the user
//! interrupt queues and the delivery thread -- stays C.

use crate::arch::i386::io_req::DevT;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use crate::glue;
use core::ffi::{c_int, c_uint};

/// `IRQGETPICMODE` of <device/irq_status.h>.
const IRQGETPICMODE: c_uint = 0;

/// The status reply for `flavor`, or [`None`] for a flavor the device
/// does not serve.
fn getstat(flavor: c_uint) -> Option<(c_int, u32)> {
    match flavor {
        IRQGETPICMODE => {
            // SAFETY: `pic_mode` is one machine global, initialized
            // before the device layer starts and never written again;
            // the C read it exactly this way.
            let mode = unsafe { glue::pic_mode };
            Some((mode, 1))
        }
        _ => None,
    }
}

/// Device status query.  `irqgetstat()` in C.
///
/// # Safety
///
/// For `IRQGETPICMODE`, the only flavor the C served, `data` must be
/// writable for one integer and `count` must be writable; the C wrote
/// both and read neither.  Every other flavor leaves both untouched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn irqgetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    match getstat(flavor) {
        Some((mode, n)) => {
            // SAFETY: the caller promises one writable integer behind
            // `data` and a writable `count`.
            unsafe {
                *data = mode;
                *count = n;
            }
            Ok(DeviceSuccess::Success).as_io_return()
        }
        None => Err(DeviceError::InvalidOperation).as_io_return(),
    }
}
