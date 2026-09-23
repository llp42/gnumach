// SPDX-License-Identifier: CMU-Mach
// Derived from device/ds_routines.c:
//   Copyright (c) 1993,1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1996 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device-open compatibility entry point, which `device/ds_routines.c`
//! used to define and MIG's generated <device/device.server.h> declares.

use crate::glue;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// `ds_device_open_new()` in C.
///
/// # Safety
///
/// [`glue::ds_device_open()`]'s contract: `open_port` and `reply_port` must be
/// valid ports, `name` must be readable through its terminator, and `devp`
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_open_new(
    open_port: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    name: *const c_char,
    devp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the arguments meet the callee's contract;
    // this forward adds no requirement of its own.
    unsafe {
        glue::ds_device_open(
            open_port,
            reply_port,
            reply_port_type,
            mode,
            name,
            devp,
        )
    }
}
