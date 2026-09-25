// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/io_perm.c and i386/i386/io_perm.h:
//   Copyright (C) 2002, 2007 Free Software Foundation, Inc.
//   Copyright (c) 1993,1992,1991,1990 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/io_perm.c`, which the MIG
//! `mach_i386` server stubs and the device emulation call.

use crate::arch::i386::io_perm::{self, Access, IoPerm};
use crate::kern::task::Task;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};

/// The `kern_return_t` a Rust result stands for.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `convert_io_perm_to_port()` of i386/i386/io_perm.h.
///
/// # Safety
///
/// `io_perm` must be null or point at a live [`IoPerm`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_io_perm_to_port(
    io_perm: *mut IoPerm,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { io_perm::convert_io_perm_to_port(io_perm) }
}

/// `convert_port_to_io_perm()` of i386/i386/io_perm.h.
///
/// # Safety
///
/// `port` must be null or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_io_perm(
    port: *mut c_void,
) -> *mut IoPerm {
    // SAFETY: the caller's contract.
    unsafe { io_perm::convert_port_to_io_perm(port) }
}

/// `io_perm_deallocate()` of i386/i386/io_perm.h, the MIG destructor.
///
/// # Safety
///
/// `io_perm` must point at a live [`IoPerm`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn io_perm_deallocate(io_perm: *mut IoPerm) {
    // SAFETY: the caller's contract.
    unsafe { io_perm::deallocate(io_perm) };
}

/// `i386_io_perm_create()` of the MIG `mach_i386` server.
///
/// # Safety
///
/// `master_port` must be null or a live port, and `new` writable storage for
/// one pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_io_perm_create(
    master_port: *mut c_void,
    from: u16,
    to: u16,
    new: *mut *mut IoPerm,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { io_perm::create(master_port, from, to, new) })
}

/// `i386_io_perm_modify()` of the MIG `mach_i386` server.
///
/// # Safety
///
/// `target_task` must be null or a live task, and `io_perm` null or a live
/// [`IoPerm`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i386_io_perm_modify(
    target_task: *mut Task,
    io_perm: *mut IoPerm,
    enable: c_int,
) -> c_int {
    let access = match enable {
        0 => Access::Withdraw,
        _ => Access::Grant,
    };

    // SAFETY: the caller's contract.
    kern_return(unsafe { io_perm::modify(target_task, io_perm, access) })
}
