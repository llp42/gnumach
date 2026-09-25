// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/bootstrap.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from kern/bootstrap.c:
//   Copyright (c) 1992-1989 Carnegie Mellon University.
//   Copyright (c) 1995-1993 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the multiboot bootstrap, one adapter per symbol
//! `kern/bootstrap.c` used to define.

use crate::arch::types::VmSize;
use crate::kern::boot_script::Cmd;
use crate::kern::bootstrap;
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::Task;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

/// `boot_script_malloc()` of kern/bootstrap.c, which the header asks the
/// user to define.
///
/// # Safety
///
/// `kalloc_init()` must have run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_malloc(size: c_uint) -> *mut c_void {
    // `c_uint` is 32 bits and `VmSize` is 32 or 64 on the two targets, so the
    // conversion is the widening the C call did implicitly and cannot lose a
    // bit.
    let size = size as VmSize;
    // `kalloc` reports failure as address zero, which becomes a null
    // pointer; the caller promises the allocator is up.
    kalloc(size).map_or(ptr::null_mut(), |buf| buf.as_ptr().cast::<c_void>())
}

/// `boot_script_free()` of kern/bootstrap.c, which the header asks the user
/// to define.
///
/// # Safety
///
/// `ptr` must name a live allocation of exactly `size` bytes made by
/// [`boot_script_malloc()`], and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_free(ptr: *mut c_void, size: c_uint) {
    // The same widening as above.
    let size = size as VmSize;
    if let Some(ptr) = NonNull::new(ptr.cast::<u8>()) {
        // SAFETY: the caller promises a live allocation of `size` bytes based
        // at `ptr`.
        unsafe { kfree(ptr, size) };
    }
}

/// `boot_script_free_task()` of kern/bootstrap.c.
///
/// # Safety
///
/// `task` must be null or the live task the corresponding
/// `boot_script_task_create()` returned; the caller must hold no locks,
/// because termination may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_free_task(
    task: *mut Task,
    aborting: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { bootstrap::free_task(task, aborting != 0) };
}

/// `bootstrap_create()` of kern/bootstrap.c.
///
/// # Safety
///
/// Runs once from the boot sequence, after `boot_info` and the IPC layers
/// are live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bootstrap_create() {
    // SAFETY: the caller's contract.
    unsafe { bootstrap::create() };
}

/// `boot_script_exec_cmd()` of kern/boot_script.c.
///
/// # Safety
///
/// `hook` must be the module `boot_script_parse_line()` recorded, `task` a
/// live task or null, and `argv` its null-terminated vector.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_exec_cmd(
    hook: *mut c_void,
    task: *mut Task,
    _path: *mut c_char,
    _argc: c_int,
    argv: *mut *mut c_char,
    _strings: *mut c_char,
    _stringlen: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { bootstrap::exec_cmd(hook, task, argv) };
    0
}

/// `boot_script_task_create()` of kern/bootstrap.c.
///
/// # Safety
///
/// `cmd` must be a live command with a NUL-terminated path, and the caller
/// must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_task_create(cmd: *mut Cmd) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { bootstrap::task_create(cmd) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `boot_script_task_resume()` of kern/bootstrap.c.
///
/// # Safety
///
/// `cmd` must be a live command whose task is live, and the caller must hold
/// no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_task_resume(cmd: *mut Cmd) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { bootstrap::task_resume(cmd) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `boot_script_prompt_task_resume()` of kern/bootstrap.c.
///
/// # Safety
///
/// `cmd` must be a live command whose task is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_prompt_task_resume(
    cmd: *mut Cmd,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { bootstrap::prompt_task_resume(cmd) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `boot_script_insert_right()` of kern/bootstrap.c.
///
/// # Safety
///
/// `cmd` must be a live command with a live task, `port` a live port, and
/// `namep` writable for one name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_insert_right(
    cmd: *mut Cmd,
    port: *mut c_void,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let name = unsafe { bootstrap::insert_right(cmd, port) };
    // SAFETY: the caller promises the writable slot.
    unsafe { namep.write(name) };
    0
}

/// `boot_script_insert_task_port()` of kern/bootstrap.c.
///
/// # Safety
///
/// `cmd` must be a live command with a live task, `task` a live task, and
/// `namep` writable for one name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boot_script_insert_task_port(
    cmd: *mut Cmd,
    task: *mut Task,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let name = unsafe { bootstrap::insert_task_port(cmd, task) };
    // SAFETY: the caller promises the writable slot.
    unsafe { namep.write(name) };
    0
}
