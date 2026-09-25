// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/gsync.c and kern/gsync.h:
//   Copyright (C) 2016 Free Software Foundation, Inc.
//   Contributed by Agustina Arzille <avarzille@riseup.net>, 2016.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/gsync.c`, which now call the
//! [`gsync`](crate::kern::gsync) core.

use crate::arch::types::VmOffset;
use crate::kern::gsync::{self, Flags};
use crate::kern::task::Task;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint};
use core::ptr::NonNull;

/// `gsync_setup()` of kern/gsync.c.
#[unsafe(no_mangle)]
pub extern "C" fn gsync_setup() {
    gsync::setup();
}

/// `gsync_wait()` of kern/gsync.c.
///
/// # Safety
///
/// `task` must be null or a live task, as MIG's server entry passes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gsync_wait(
    task: *mut Task,
    addr: VmOffset,
    lo: c_uint,
    hi: c_uint,
    msec: c_uint,
    flags: c_int,
) -> c_int {
    let Some(task) = NonNull::new(task) else {
        return c_int::from(KernError::InvalidTask);
    };
    match gsync::wait(task, addr, lo, hi, msec, Flags::from_bits(flags)) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `gsync_wake()` of kern/gsync.c.
///
/// # Safety
///
/// `task` must be null or a live task, as MIG's server entry passes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gsync_wake(
    task: *mut Task,
    addr: VmOffset,
    val: c_uint,
    flags: c_int,
) -> c_int {
    let Some(task) = NonNull::new(task) else {
        return c_int::from(KernError::InvalidTask);
    };
    match gsync::wake(task, addr, val, Flags::from_bits(flags)) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `gsync_requeue()` of kern/gsync.c.
///
/// # Safety
///
/// `task` must be null or a live task, as MIG's server entry passes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gsync_requeue(
    task: *mut Task,
    src_addr: VmOffset,
    dst_addr: VmOffset,
    wake_one: c_int,
    flags: c_int,
) -> c_int {
    let Some(task) = NonNull::new(task) else {
        return c_int::from(KernError::InvalidTask);
    };
    match gsync::requeue(
        task,
        src_addr,
        dst_addr,
        wake_one != 0,
        Flags::from_bits(flags),
    ) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}
