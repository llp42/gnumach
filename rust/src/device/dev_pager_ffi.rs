// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_pager.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `device/dev_pager.c`, the MIG device-pager
//! server entries of <device/device_pager.server.h> plus `device_map_page()`
//! and `device_pager_setup()` of the C headers.
//!
//! Every adapter hands its raw arguments to the matching core in
//! [`dev_pager`] without adding an obligation of its own.

use crate::arch::types::{VmOffset, VmSize};
use crate::device::dev_pager;
use crate::glue;
use crate::ipc::IpcPort;
use crate::vm::types::VmProt;
use core::ffi::{CStr, c_int, c_uint, c_void};

/// `KERN_SUCCESS` of <mach/kern_return.h>.
const KERN_SUCCESS: c_int = 0;

/// Halt with the message the C `panic()` of `device/dev_pager.c` printed.
fn unimplemented(line: c_int, fun: &CStr, message: &CStr) -> ! {
    // SAFETY: `Panic` does not return.
    unsafe {
        glue::Panic(
            c"device/dev_pager.c".as_ptr(),
            line,
            fun.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// `device_pager_setup()` of `device/dev_pager.c`.
///
/// # Safety
///
/// `device` must be a live, referenced mach device whose `dev_ops` is live,
/// and `pager` writable storage for the new send right on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_setup(
    device: *mut c_void,
    prot: c_int,
    offset: VmOffset,
    // The C rounded the size into the record and never read it back, so the
    // core does not take it.
    _size: VmSize,
    pager: *mut *mut c_void,
) -> c_int {
    match unsafe { dev_pager::setup(device.cast(), prot, offset) } {
        Ok(port) => {
            // SAFETY: the caller promises the writable slot.
            unsafe { *pager = port.as_ptr() };
            KERN_SUCCESS
        }
        Err(error) => error.code(),
    }
}

/// `device_pager_data_request()` of `device/dev_pager.c`, the MIG
/// `memory_object_data_request` server entry.
///
/// # Safety
///
/// `pager` must be the live port of a set-up pager record and
/// `pager_request` the live control port the kernel bound to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_data_request(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    offset: VmOffset,
    length: VmSize,
    _protection_required: VmProt,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_pager::data_request(pager, pager_request, offset, length) };
    KERN_SUCCESS
}

/// `device_pager_init_pager()` of `device/dev_pager.c`, the MIG
/// `memory_object_init` server entry.
///
/// # Safety
///
/// The MIG server calls this once for the pager `pager` denotes, before any
/// data request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_init_pager(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    pager_name: Option<IpcPort>,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_pager::init_pager(pager, pager_request, pager_name) };
    KERN_SUCCESS
}

/// `device_pager_terminate()` of `device/dev_pager.c`, the MIG
/// `memory_object_terminate` server entry.
///
/// # Safety
///
/// The MIG server calls this once after a completed init, with the ports of
/// that init.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_terminate(
    pager: Option<IpcPort>,
    pager_request: Option<IpcPort>,
    pager_name: Option<IpcPort>,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_pager::terminate(pager, pager_request, pager_name) };
    KERN_SUCCESS
}

/// `device_map_page()` of <device/dev_pager.h>.
///
/// # Safety
///
/// `dsp` must be the live record `device_pager_data_request()` passes to
/// `vm_object_page_map()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_map_page(
    dsp: *mut c_void,
    offset: VmOffset,
) -> VmOffset {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_pager::device_map_page(dsp, offset) }
}

/// `device_pager_init()` of `device/dev_pager.c`.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during
/// boot, before any pager exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_init() {
    // SAFETY: the caller's contract is the core's.
    unsafe { dev_pager::init() };
}

/// `device_pager_copy()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_copy(
    _old_memory_object: Option<IpcPort>,
    _old_memory_control: Option<IpcPort>,
    _offset: VmOffset,
    _length: VmSize,
    _new_memory_object: Option<IpcPort>,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_copy",
        c"(device_pager)copy: called",
    )
}

/// `device_pager_supply_completed()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_supply_completed(
    _device_pager: Option<IpcPort>,
    _memory_control: Option<IpcPort>,
    _offset: VmOffset,
    _length: VmSize,
    _result: c_int,
    _error_offset: VmOffset,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_supply_completed",
        c"(device_pager)supply_completed: called",
    )
}

/// `device_pager_data_return()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_data_return(
    _device_pager: Option<IpcPort>,
    _memory_control: Option<IpcPort>,
    _offset: VmOffset,
    _data: VmOffset,
    _data_cnt: c_uint,
    _dirty: c_int,
    _kernel_copy: c_int,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_data_return",
        c"(device_pager)data_return: called",
    )
}

/// `device_pager_change_completed()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_change_completed(
    _device_pager: Option<IpcPort>,
    _may_cache: c_int,
    _copy_strategy: c_int,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_change_completed",
        c"(device_pager)change_completed: called",
    )
}

/// `device_pager_data_unlock()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_data_unlock(
    _device_pager: Option<IpcPort>,
    _memory_control: Option<IpcPort>,
    _offset: VmOffset,
    _length: VmSize,
    _desired_access: VmProt,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_data_unlock",
        c"(device_pager)data_unlock: called",
    )
}

/// `device_pager_lock_completed()` of `device/dev_pager.c`.
///
/// # Safety
///
/// Never returns; the caller must accept the halt.
///
/// # Panics
///
/// Always, through [`glue::Panic`], as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_pager_lock_completed(
    _device_pager: Option<IpcPort>,
    _memory_control: Option<IpcPort>,
    _offset: VmOffset,
    _length: VmSize,
) -> c_int {
    unimplemented(
        line!() as c_int,
        c"device_pager_lock_completed",
        c"(device_pager)lock_completed: called",
    )
}
