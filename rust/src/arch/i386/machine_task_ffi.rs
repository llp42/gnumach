// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/machine_task.c and i386/i386/task.h:
//   Copyright (c) 2002, 2007 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/machine_task.c`, one adapter per
//! symbol <i386/task.h> declares.

use crate::arch::i386::machine_task::{IOPB_BYTES, IOPB_CACHE};
use crate::kern::slab::CacheInitFlags;
use crate::kern::task::Task;
use core::ffi::CStr;

/// The name `machine_task_iopb_cache` is registered under, as the C spelled
/// it.
const IOPB_CACHE_NAME: &CStr = c"i386_task_iopb";

/// `machine_task_module_init()` of <i386/task.h>: build the iopb cache.
///
/// # Safety
///
/// Called once at startup, before any task exists and so before anything can
/// allocate from the cache.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_module_init() {
    let cache = &raw mut IOPB_CACHE;
    // SAFETY: the caller promises this runs once before any user of the
    // cache, so nothing else can be touching the cache object while the slab
    // layer builds it in place.
    unsafe {
        (*cache).init(
            IOPB_CACHE_NAME.to_bytes(),
            IOPB_BYTES,
            0,
            None,
            CacheInitFlags::EMPTY,
        );
    }
}

/// `machine_task_init()` of <i386/task.h>.
///
/// # Safety
///
/// `task` must point at a live task whose machine part nothing else has
/// initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_init(task: *mut Task) {
    // SAFETY: the caller promises the live task.
    unsafe { (*task).machine.init() };
}

/// `machine_task_terminate()` of <i386/task.h>.
///
/// # Safety
///
/// `task` must point at a live task that no other reference reaches.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_terminate(task: *mut Task) {
    // SAFETY: the caller promises the task's last reference.
    unsafe { (*task).machine.terminate() };
}

/// `machine_task_collect()` of <i386/task.h>.
///
/// # Safety
///
/// `task` must point at a live task the caller holds a reference on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_collect(task: *mut Task) {
    // SAFETY: the caller promises the live task.
    unsafe { (*task).machine.collect() };
}
