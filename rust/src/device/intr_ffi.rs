// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/intr.c and device/intr.h:
//   Copyright (c) 2010, 2011, 2016, 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries of `device/intr.c`, declared in <device/intr.h>.

use crate::arch::i386::io_req::DevT;
use crate::arch::i386::irq::{IrqDev, UserIntr};
use crate::device::intr;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_int, c_uint, c_ulong, c_void};
use core::ptr::{self, NonNull};

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
    match intr::getstat(flavor) {
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

/// `irq_acknowledge()` in C.
///
/// # Safety
///
/// `receive_port` must be the port named by a live registration, whose
/// reference the caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn irq_acknowledge(receive_port: *mut c_void) -> c_int {
    // SAFETY: the caller promises the registered port.
    match unsafe { intr::irq_acknowledge(receive_port) } {
        Ok(id) => {
            intr::enable_line(id);
            0
        }
        Err(code) => code,
    }
}

/// `deliver_user_intr()` in C.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, `id` inside its `irq` table, and `e` the
/// live registration for that line.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deliver_user_intr(
    dev: *mut IrqDev,
    id: c_int,
    e: *mut UserIntr,
) -> c_int {
    // SAFETY: the caller promises the live table, id and entry.
    c_int::from(unsafe { intr::deliver_user_intr(dev, id, e) })
}

/// `insert_intr_entry()` in C.
///
/// # Safety
///
/// `dev` must be the live `irqtab` with an initialized `intr_queue`, and
/// `receive_port` a port the caller keeps alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn insert_intr_entry(
    dev: *mut IrqDev,
    id: c_int,
    receive_port: *mut c_void,
) -> *mut UserIntr {
    // SAFETY: the caller promises the live table and port.
    unsafe { intr::insert_intr_entry(dev, id, receive_port) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `install_user_intr_handler()` in C.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, `id` inside its `irq` table, and `entry`
/// the live entry [`insert_intr_entry()`] returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn install_user_intr_handler(
    dev: *mut IrqDev,
    id: c_int,
    flags: c_ulong,
    entry: *mut UserIntr,
) -> c_int {
    // SAFETY: the caller promises the live table and entry.
    match unsafe { intr::install_user_intr_handler(dev, id, flags, entry) } {
        Ok(()) => Ok(DeviceSuccess::Success).as_io_return(),
        Err(error) => Err(error).as_io_return(),
    }
}

/// `intr_thread()` in C: the interrupt service thread.
///
/// # Safety
///
/// Started once, as the `intr` kernel thread, after the device layer and the
/// interrupt tables exist.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn intr_thread() {
    // SAFETY: the caller starts it once, as the C startup did.
    unsafe { intr::intr_thread() }
}
