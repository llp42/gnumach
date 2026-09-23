// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_name.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device-name comparison and the empty device-table entries of
//! `device/dev_name.c`, declared in <device/dev_hdr.h> and
//! <device/conf.h>.
//!
//! The eleven `nulldev_*`/`nodev_*`/`nomap` routines fill the empty
//! slots of the device switch tables: each answers a plain constant
//! and reads none of its arguments, so none of them needs an `unsafe`
//! export and none carries a `# Safety` section.  `name_equal()` is
//! the one with a caller contract, and it is this file's only `unsafe`
//! function.
//!
//! The other two functions of the C file, `dev_name_lookup()` and
//! `dev_set_indirection()`, stay C and keep their prototypes.
//!
//! The device return codes come from <device/device_types.h> through
//! their single Rust home in [`return`](crate::device::return).

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::types::VmOffset;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use core::ffi::{c_char, c_int, c_uint, c_ushort, c_void};
use core::slice;

/// The empty reset slot of the device tables.  `nulldev_reset()` in C.
///
/// The C ignores its argument and always succeeds.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_reset(_dev: DevT) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// The default open slot of the device tables.  `nulldev_open()` in C.
///
/// The C ignores its arguments and succeeds; the request is never
/// read.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_open(
    _dev: DevT,
    _flags: c_int,
    _ior: *mut IoReq,
) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// The default close slot of the device tables.  `nulldev_close()` in
/// C.
///
/// The C ignores its arguments and returns nothing.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_close(_dev: DevT, _flags: c_int) {}

/// The default read slot of the device tables.  `nulldev_read()` in C.
///
/// The C ignores its arguments and succeeds; the request is never
/// read.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_read(_dev: DevT, _ior: *mut IoReq) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// The default write slot of the device tables.  `nulldev_write()` in
/// C.
///
/// The C ignores its arguments and succeeds; the request is never
/// read.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_write(_dev: DevT, _ior: *mut IoReq) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// The status query slot of a device with no status.
/// `nulldev_getstat()` in C.
///
/// The C ignores its arguments and reports `D_INVALID_OPERATION`; the
/// data and count are never read.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_getstat(
    _dev: DevT,
    _flavor: c_uint,
    _data: *mut c_int,
    _count: *mut c_uint,
) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// The status setter slot of a device with no status.
/// `nulldev_setstat()` in C.
///
/// The C ignores its arguments and reports `D_INVALID_OPERATION`; the
/// data is never read.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_setstat(
    _dev: DevT,
    _flavor: c_uint,
    _data: *mut c_int,
    _count: c_uint,
) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// The port-death slot of a device with no reply ports.
/// `nulldev_portdeath()` in C.
///
/// The C ignores its arguments and always succeeds.
#[unsafe(no_mangle)]
pub extern "C" fn nulldev_portdeath(_dev: DevT, _port: VmOffset) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// The asynchronous-input slot of a device with no filters.
/// `nodev_async_in()` in C.
///
/// The C ignores its arguments and reports `D_INVALID_OPERATION`; the
/// port and filter are never read.
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

/// The driver-info slot of a device with no driver info.
/// `nodev_info()` in C.
///
/// The C ignores its arguments and reports `D_INVALID_OPERATION`; the
/// output array is never read.
#[unsafe(no_mangle)]
pub extern "C" fn nodev_info(_dev: DevT, _a: c_int, _b: *mut c_int) -> c_int {
    Err(DeviceError::InvalidOperation).as_io_return()
}

/// The mmap slot of a device that cannot be mapped.  `nomap()` in C.
///
/// The C ignores its arguments and reports failure with
/// `(vm_offset_t)-1`, the largest address.
#[unsafe(no_mangle)]
pub extern "C" fn nomap(_dev: DevT, _off: VmOffset, _prot: c_int) -> VmOffset {
    VmOffset::MAX
}

/// Compare the first `len` bytes of `src` with the zero-terminated
/// `target`.  `name_equal()` in C.
///
/// A match means the prefix is equal and the byte after it is
/// `target`'s terminator, so `target` is exactly `len` bytes long.
/// The C's `while (--len >= 0)` never runs for a zero or negative
/// `len` and answers `target[0] == 0`; the adapter clamps the length,
/// which keeps that answer and keeps a negative value out of the loop
/// bound below.
///
/// The comparison stops at the first differing byte, so `target` is
/// only read through that byte, or through `target[len]` when every
/// byte matches: the same footprint the C has.
///
/// # Safety
///
/// `src` must be readable for `len` bytes when `len` is positive
/// (nothing is read when it is not), and `target` must be readable
/// through the first byte that differs from `src`, or through
/// `target[len]` when the first `len` bytes all match.  A
/// NUL-terminated `target` satisfies this.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn name_equal(
    src: *const c_char,
    len: c_int,
    target: *const c_char,
) -> c_int {
    // The C's pre-decrement skips its loop for any len <= 0 and then
    // asks only whether target is empty; the clamp keeps that answer.
    // A nonnegative c_int converts to usize without loss on both
    // targets.
    let len = len.max(0) as usize;
    // SAFETY: the caller promises `len` readable bytes at `src` when
    // positive.  A zero length names nothing, and the empty slice
    // spares the caller from producing a non-null pointer for it.
    let src: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(src.cast::<u8>(), len) }
    };
    let target = target.cast::<u8>();
    for (i, &want) in src.iter().enumerate() {
        // SAFETY: the caller promises `target` readable through the
        // first byte that differs from `src`, and no byte before `i`
        // has differed yet.
        if unsafe { *target.add(i) } != want {
            return 0;
        }
    }
    // SAFETY: every byte of the prefix matched, and the caller
    // promises `target[len]` readable for that case; it is the
    // terminator the C tests.
    c_int::from(unsafe { *target.add(len) } == 0)
}
