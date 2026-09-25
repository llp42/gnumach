// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_lookup.c and device/dev_hdr.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `device/dev_lookup.c`, which
//! <device/dev_hdr.h>, the MIG `device_types.defs` translations and
//! <device/ds_routines.h> call.
//!
//! Every adapter hands its raw arguments to the matching core in
//! [`dev_lookup`] without adding an obligation of its own.

use crate::device::dev_lookup;
use crate::device::ds_routines::Device;
use core::ffi::{c_char, c_int, c_void};

/// `dev_lookup_init()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during
/// boot, before any device exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_lookup_init() {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::init() };
}

/// `device_lookup()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `name` must be a NUL-terminated device name, and the device package must
/// be initialized. The returned device, when non-null, carries one
/// reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_lookup(name: *const c_char) -> *mut c_void {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::lookup(name) }
        .map_or(core::ptr::null_mut(), |device| device.as_ptr().cast())
}

/// `mach_device_reference()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live mach device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_device_reference(device: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::reference(device.cast()) };
}

/// `mach_device_deallocate()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live mach device the caller holds a reference on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_device_deallocate(device: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::deallocate(device.cast()) };
}

/// `dev_port_enter()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live device whose port is live, and the caller must
/// own a device reference for the mapping to take.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_port_enter(device: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::port_enter(device.cast()) };
}

/// `dev_port_remove()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live device whose port carries the mapping, and the
/// caller's reference moves into the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_port_remove(device: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::port_remove(device.cast()) };
}

/// `dev_port_lookup()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `port` must be null, dead, or a live port. The returned `struct device`,
/// when non-null, carries the reference the emulation made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_port_lookup(port: *mut c_void) -> *mut c_void {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::port_lookup(port) }.cast::<c_void>()
}

/// `convert_device_to_port()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be null or a live `struct device`, and its emulation must
/// be one whose `dev_to_port` takes the reference the caller consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_device_to_port(
    device: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::convert_to_port(device.cast::<Device>()) }
}

/// `dev_map()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `routine` must be a C callback that takes the `mach_device_t` and
/// `mach_port_t` it is passed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dev_map(
    routine: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    port: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_lookup::map(routine, port) }
}
