// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Derived from include/mach/gnumach.defs:
//   Copyright (C) 2012 Free Software Foundation
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task entries of `kern/task.c`, which `kern/task.h` declares.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::processor::ProcessorSet;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::ptr;

/// `task_assign_default()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or a live `struct task` the caller holds an extra
/// reference to, and the caller must hold no locks: `task_assign()` may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_assign_default(
    task: *mut c_void,
    assign_threads: c_int,
) -> c_int {
    // SAFETY: the caller's contract is `task_assign()`'s own, and the default
    // set is a live `struct processor_set` for the life of the kernel.
    let assigned = unsafe {
        glue::task_assign(
            task,
            ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>(),
            assign_threads,
        )
    };

    let result = match u8::try_from(assigned) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    };

    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Create a task under `parent_task`, giving it its own map.
///
/// # Safety
///
/// `parent_task` must designate a live task, and the caller must hold no
/// locks: creation may block on the memory it takes.
unsafe fn create(
    parent_task: *mut c_void,
    inherit_memory: c_int,
) -> Result<*mut c_void, KernError> {
    let mut child = ptr::null_mut();
    // SAFETY: the caller promises a live parent; `task_create_kernel()` writes
    // the child slot on success and leaves it alone on failure.
    let created = unsafe {
        glue::task_create_kernel(parent_task, inherit_memory, &raw mut child)
    };

    let result = match u8::try_from(created) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    };

    match result {
        Ok(()) => Ok(child),
        Err(error) => Err(error),
    }
}

/// `task_create()` of kern/task.c.
///
/// # Safety
///
/// `parent_task` must be null or a live `struct task`, and `child_task` must
/// be writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_create(
    parent_task: *mut c_void,
    inherit_memory: c_int,
    child_task: *mut *mut c_void,
) -> c_int {
    if parent_task.is_null() {
        return c_int::from(KernError::InvalidTask);
    }

    // SAFETY: the null check above is the C's, and the caller promises the
    // rest of the routine's contract.
    match unsafe { create(parent_task, inherit_memory) } {
        Ok(child) => {
            // SAFETY: the caller promises `child_task` is writable;
            // `task_create_kernel()` wrote its value on success only.
            unsafe { *child_task = child };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_ras_control()` of kern/task.c.
///
/// # Safety
///
/// The MIG server calls this with the task it converted from the request port;
/// nothing here reads or writes any argument.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_ras_control(
    _task: *mut c_void,
    _pc: VmOffset,
    _endpc: VmOffset,
    _flavor: c_int,
) -> c_int {
    c_int::from(KernError::Failure)
}

/// `register_new_task_notification()` of kern/task.c.
///
/// # Safety
///
/// `host` must be null or the live host privilege pointer the MIG stub
/// converted, and `notification` the port the request carried; the caller
/// holds no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn register_new_task_notification(
    host: *mut c_void,
    notification: *mut c_void,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidHost);
    }

    // SAFETY: the global is the C `ipc_port_t`, never borrowed as a Rust
    // reference; this read is the C body's own unlocked access.
    if !unsafe { glue::new_task_notification }.is_null() {
        return c_int::from(KernError::NoAccess);
    }

    // SAFETY: as above; the store is the C body's own.
    unsafe { glue::new_task_notification = notification };
    0
}
