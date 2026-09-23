// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/machine_task.c and i386/i386/io_perm.h:
//   Copyright (c) 2002, 2007 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The machine task module's startup, which
//! `i386/i386/machine_task.c` used to define and `i386/i386/task.h`
//! declares.
//!
//! The module's one piece of state is the slab cache the I/O
//! permission bitmaps come from.  Building it is a single
//! `kmem_cache_init()` over a cache object that stays in C, so the
//! startup routine moved while the rest of the file waits on a mirror
//! of `task->machine`.

use crate::arch::types::VmSize;
use crate::glue;
use core::ffi::CStr;

/// `IOPB_MAX` of <i386/io_perm.h>: the highest I/O port a task's
/// permission bitmap can name.
const IOPB_MAX: VmSize = 0xffff;

/// `IOPB_BYTES` of <i386/io_perm.h>: one bit per port, rounded up to
/// whole bytes.
const IOPB_BYTES: VmSize = (IOPB_MAX + 1).div_ceil(8);

/// The name `machine_task_iopb_cache` is registered under, as the C
/// spelled it.
const IOPB_CACHE_NAME: &CStr = c"i386_task_iopb";

/// Initialize the machine task module.  `machine_task_module_init()`
/// in i386/i386/machine_task.c.
///
/// The cache takes no constructor and no alignment, exactly as the C
/// passed `NULL` and `0`.
///
/// # Safety
///
/// Called once at startup, from `task_init()` in kern/task.c, before
/// any task exists and so before anything can allocate from
/// [`glue::machine_task_iopb_cache`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_module_init() {
    let cache = &raw mut glue::machine_task_iopb_cache;
    // SAFETY: the caller promises this runs once before any user of
    // the cache, so nothing else can be touching the cache object
    // while the slab layer builds it in place.
    unsafe {
        glue::kmem_cache_init(
            cache,
            IOPB_CACHE_NAME.as_ptr(),
            IOPB_BYTES,
            0,
            None,
            0,
        );
    }
}
