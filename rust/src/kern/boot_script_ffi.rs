// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/boot_script.c and kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the boot-script parser, one adapter per symbol
//! `kern/boot_script.c` used to define and <kern/boot_script.h> declares.

use crate::kern::boot_script;
use core::ffi::{c_char, c_int, c_long, c_void};

/// `boot_script_parse_line()` of kern/boot_script.c.
///
/// # Safety
///
/// `cmdline` must point at a writable NUL-terminated string that stays
/// mapped and unmodified until `boot_script_exec()` returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_parse_line(
    hook: *mut c_void,
    cmdline: *mut c_char,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { boot_script::parse_line(hook, cmdline) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `boot_script_exec()` of kern/boot_script.c.
///
/// # Safety
///
/// Every line passed to `boot_script_parse_line()` since the last call must
/// still be mapped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_exec() -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { boot_script::exec() } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `boot_script_set_variable()` of kern/boot_script.c.
///
/// # Safety
///
/// `name` must name a NUL-terminated string that outlives the symbol table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_set_variable(
    name: *const c_char,
    type_: c_int,
    val: c_long,
) -> c_int {
    // SAFETY: the caller's contract.
    if unsafe { boot_script::set_variable(name, type_, val) } {
        0
    } else {
        1
    }
}

/// `boot_script_define_function()` of kern/boot_script.c.
///
/// # Safety
///
/// `name` must name a NUL-terminated string that outlives the symbol table,
/// and `func` must be a function callable with the command and an `int`
/// out-parameter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_define_function(
    name: *const c_char,
    ret_type: c_int,
    func: boot_script::DefinedFn,
) -> c_int {
    // SAFETY: the caller's contract.
    if unsafe { boot_script::define_function(name, ret_type, func) } {
        0
    } else {
        1
    }
}

/// `boot_script_error_string()` of kern/boot_script.c.
///
/// # Safety
///
/// The returned pointer, when non-null, names a static message the caller
/// must neither write nor free; an unknown code gives back null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_error_string(err: c_int) -> *mut c_char {
    boot_script::error_string(err)
}
