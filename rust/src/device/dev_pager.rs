// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_pager.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The memory-object entry points the device pager does not implement, which
//! `device/dev_pager.c` used to define.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue;
use crate::ipc::IpcPort;
use crate::vm::types::VmProt;
use core::ffi::{CStr, c_int, c_uint};

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

/// `device_pager_copy()` in C.
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

/// `device_pager_supply_completed()` in C.
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

/// `device_pager_data_return()` in C.
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

/// `device_pager_change_completed()` in C.
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

/// `device_pager_data_unlock()` in C.
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

/// `device_pager_lock_completed()` in C.
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
