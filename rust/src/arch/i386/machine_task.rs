// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/machine_task.c, i386/i386/task.h and
// i386/i386/io_perm.h:
//   Copyright (c) 2002, 2007 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The machine task module's startup, which `i386/i386/machine_task.c` used to
//! define, and the `struct machine_task` mirror of `i386/i386/task.h`.

use crate::arch::types::VmSize;
use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::slab::CacheInitFlags;
use core::ffi::{CStr, c_int};
use core::mem::{align_of, offset_of, size_of};

/// `struct machine_task` of <i386/task.h>: the machine-specific part of a
/// task, the lock and range of its I/O-permission bitmap.
#[repr(C)]
pub struct MachineTask {
    /// `iopb_lock`: protects `iopb_size` and `iopb`.
    pub iopb_lock: SimpleLock,
    /// `iopb_size`: the highest I/O port number enabled.
    pub iopb_size: c_int,
    /// `iopb`: the permission bitmap, or null.
    pub iopb: *mut u8,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<MachineTask>() == 16);
    assert!(align_of::<MachineTask>() == 8);
    assert!(offset_of!(MachineTask, iopb_lock) == 0);
    assert!(offset_of!(MachineTask, iopb_size) == 4);
    assert!(offset_of!(MachineTask, iopb) == 8);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<MachineTask>() == 12);
    assert!(align_of::<MachineTask>() == 4);
    assert!(offset_of!(MachineTask, iopb_lock) == 0);
    assert!(offset_of!(MachineTask, iopb_size) == 4);
    assert!(offset_of!(MachineTask, iopb) == 8);
};

/// `IOPB_MAX` of <i386/io_perm.h>: the highest I/O port a task's permission
/// bitmap can name.
const IOPB_MAX: VmSize = 0xffff;

/// `IOPB_BYTES` of <i386/io_perm.h>: one bit per port, rounded up to whole
/// bytes.
const IOPB_BYTES: VmSize = (IOPB_MAX + 1).div_ceil(8);

/// The name `machine_task_iopb_cache` is registered under, as the C spelled
/// it.
const IOPB_CACHE_NAME: &CStr = c"i386_task_iopb";

/// `machine_task_module_init()` in i386/i386/machine_task.c.
///
/// # Safety
///
/// Called once at startup, from `task_init()` of `kern/task.h`, before any
/// task exists and so before anything can allocate from
/// [`glue::machine_task_iopb_cache`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn machine_task_module_init() {
    let cache = &raw mut glue::machine_task_iopb_cache;
    // SAFETY: the caller promises this runs once before any user of the cache,
    // so nothing else can be touching the cache object while the slab layer
    // builds it in place.
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
