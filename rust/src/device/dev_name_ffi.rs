// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_name.c, device/dev_hdr.h and device/conf.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries of `device/dev_name.c`, declared in
//! <device/dev_hdr.h> and <device/conf.h>.

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::types::VmOffset;
use crate::device::dev_name;
use crate::device::ds_routines::DevOps;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_char, c_int, c_uint, c_ushort, c_void};

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
    // SAFETY: the caller promises the readable name.
    c_int::from(unsafe { dev_name::name_equal(src, len, target) })
}

/// `dev_name_lookup()` in C.
///
/// # Safety
///
/// `name` must be a NUL-terminated string readable by the caller, and `ops`
/// and `unit` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_name_lookup(
    name: *const c_char,
    ops: *mut *mut DevOps,
    unit: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises the NUL-terminated name.
    match unsafe { dev_name::lookup(name) } {
        Some((found_ops, found_unit)) => {
            // SAFETY: the caller promises both out-parameters writable.
            unsafe {
                *ops = found_ops.as_ptr();
                *unit = found_unit;
            }
            1
        }
        None => 0,
    }
}

/// `dev_set_indirection()` in C.
///
/// # Safety
///
/// `name` must be a NUL-terminated string, and `ops` must be a live entry
/// point table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_set_indirection(
    name: *const c_char,
    ops: *mut DevOps,
    unit: c_int,
) {
    // SAFETY: the caller promises both.
    unsafe { dev_name::set_indirection(name, ops, unit) }
}
