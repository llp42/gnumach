// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/mach_clock.h:
//   Copyright (C) 2006, 2007 Free Software Foundation, Inc.
// Derived from kern/mach_clock.c:
//   Copyright (c) 1994-1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/mach_clock.c`, which now call the
//! [`clock`](crate::kern::mach_clock) core.

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::types::VmOffset;
use crate::device::r#return::{DeviceSuccess, IoResultExt};
use crate::glue::time_value::{TimeValue, TimeValue64};
use crate::kern::mach_clock::{self as clock, Timeout};
use core::ffi::{c_int, c_uint, c_void};

/// `clock_interrupt()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn clock_interrupt(
    usec: c_int,
    usermode: c_int,
    basepri: c_int,
    _pc: VmOffset,
) {
    clock::interrupt(usec, usermode != 0, basepri != 0);
}

/// `softclock()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn softclock() {
    clock::softclock();
}

/// `set_timeout()` of kern/mach_clock.c.
///
/// # Safety
///
/// `t` must point at a live [`Timeout`] whose `fcn` and `param` are set, and
/// it must stay at its address until it expires or `reset_timeout()`
/// cancels it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_timeout(t: *mut Timeout, interval: c_uint) {
    // SAFETY: the caller's contract.
    unsafe { clock::set_timeout(t, interval) };
}

/// `reset_timeout()` of kern/mach_clock.c.
///
/// # Safety
///
/// `t` must point at a live [`Timeout`] that stays at its address across the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reset_timeout(t: *mut Timeout) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { clock::reset_timeout(t) })
}

/// `init_timeout()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn init_timeout() {
    clock::init_timeout();
}

/// `record_time_stamp()` of kern/mach_clock.c.
///
/// # Safety
///
/// `stamp` must be valid for a write, and `mapable_time_init()` must have
/// run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn record_time_stamp(stamp: *mut TimeValue64) {
    // SAFETY: the caller's contract.
    unsafe { clock::record_time_stamp(stamp) };
}

/// `read_time_stamp()` of <kern/mach_clock.h>.
///
/// # Safety
///
/// `stamp` must point at a readable [`TimeValue64`] and `result` at writable
/// storage for one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read_time_stamp(
    stamp: *const TimeValue64,
    result: *mut TimeValue64,
) {
    // SAFETY: the caller's contract.
    unsafe { clock::read_time_stamp(stamp, result) };
}

/// `host_get_time()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or a live host, and `current_time` must be valid for
/// a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_get_time(
    host: *mut c_void,
    current_time: *mut TimeValue,
) -> c_int {
    match clock::get_time(host) {
        Ok(value) => {
            // SAFETY: the caller promises the writable out-pointer.
            unsafe { current_time.write(value) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_get_time64()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or a live host, and `current_time` must be valid for
/// a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_get_time64(
    host: *mut c_void,
    current_time: *mut TimeValue64,
) -> c_int {
    match clock::get_time64(host) {
        Ok(value) => {
            // SAFETY: the caller promises the writable out-pointer.
            unsafe { current_time.write(value) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_set_time64()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn host_set_time64(
    host: *mut c_void,
    new_time: TimeValue64,
) -> c_int {
    match clock::set_time64(host, new_time) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `host_get_uptime64()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or a live host, and `uptime` must be valid for a
/// write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_get_uptime64(
    host: *mut c_void,
    uptime: *mut TimeValue64,
) -> c_int {
    match clock::get_uptime64(host) {
        Ok(value) => {
            // SAFETY: the caller promises the writable out-pointer.
            unsafe { uptime.write(value) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `mapable_time_init()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn mapable_time_init() {
    clock::mapable_time_init();
}

/// `timeout()` of kern/mach_clock.c.
///
/// # Safety
///
/// `param` is passed to `fcn` at expiry; the pool element stays at its
/// address until it expires or is cancelled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timeout(
    fcn: Option<unsafe extern "C" fn(*mut c_void)>,
    param: *mut c_void,
    interval: c_int,
) -> *mut Timeout {
    // SAFETY: the caller's contract.
    unsafe { clock::timeout(fcn, param, interval) }
}

/// `host_set_time()` of kern/mach_clock.c, the deprecated 32-bit entry.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`; MIG's
/// `_Xhost_set_time` passes the host private port's host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_set_time(
    host: *mut c_void,
    new_time: TimeValue,
) -> c_int {
    match clock::set_time64(host, TimeValue64::from(new_time)) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `host_adjust_time()` of kern/mach_clock.c, the deprecated 32-bit entry.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and `old_adjustment`
/// must be valid for a write on success; MIG's `_Xhost_adjust_time` passes its
/// reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time(
    host: *mut c_void,
    new_adjustment: TimeValue,
    old_adjustment: *mut TimeValue,
) -> c_int {
    match clock::adjust_time(host, TimeValue64::from(new_adjustment)) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for a
            // write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(TimeValue::from(old)) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_adjust_time64()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and `old_adjustment`
/// must be valid for a write on success; MIG's `_Xhost_adjust_time64` passes
/// its reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time64(
    host: *mut c_void,
    new_adjustment: TimeValue64,
    old_adjustment: *mut TimeValue64,
) -> c_int {
    match clock::adjust_time(host, new_adjustment) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for a
            // write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(old) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `timeopen()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn timeopen(
    _dev: DevT,
    _flag: c_int,
    _ior: *mut IoReq,
) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `timeclose()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn timeclose(_dev: DevT, _flag: c_int) {}
