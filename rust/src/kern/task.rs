// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Derived from include/mach/gnumach.defs:
//   Copyright (C) 2012 Free Software Foundation
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task entries of `kern/task.c`, which `kern/task.h` declares.
//!
//! [`task_create()`], [`task_ras_control()`] and
//! [`register_new_task_notification()`] join the
//! `task_assign_default()` that was already here.  The rest of
//! `kern/task.c` stays C, and so does `struct task`: it embeds an
//! `ipc_space`, a `vm_map` and the emulation vector, none of which has
//! a Rust mirror, so the task is spelled as an opaque pointer here and
//! only passed along.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::processor::ProcessorSet;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::ptr;

/// Assign a task to the default processor set.
/// `task_assign_default()` of kern/task.c.
///
/// The C forwards to `task_assign()` with `&default_pset`, and so does
/// this: the assignment itself is the `#if MACH_HOST` half of
/// kern/task.c and stays C.  `assign_threads` is the C `boolean_t`,
/// passed through unread.
///
/// # Safety
///
/// `task` must be null or a live `struct task` the caller holds an
/// extra reference to, and the caller must hold no locks:
/// `task_assign()` may block.  The MIG server calls this with the
/// operation-in-progress reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_assign_default(
    task: *mut c_void,
    assign_threads: c_int,
) -> c_int {
    // SAFETY: the caller's contract is `task_assign()`'s own, and the
    // default set is a live `struct processor_set` for the life of the
    // kernel.
    let assigned = unsafe {
        glue::task_assign(
            task,
            ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>(),
            assign_threads,
        )
    };

    // `task_assign()` answers KERN_SUCCESS, KERN_FAILURE or
    // KERN_INVALID_ARGUMENT, all inside the byte range; a code outside
    // it could not name a defined error, so it becomes the catch-all.
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
/// `parent_task` must designate a live task, and the caller must hold
/// no locks: creation may block on the memory it takes.
unsafe fn create(
    parent_task: *mut c_void,
    inherit_memory: c_int,
) -> Result<*mut c_void, KernError> {
    let mut child = ptr::null_mut();
    // SAFETY: the caller promises a live parent; `task_create_kernel()`
    // writes the child slot on success and leaves it alone on failure.
    let created = unsafe {
        glue::task_create_kernel(parent_task, inherit_memory, &raw mut child)
    };

    // `task_create_kernel()` answers KERN_SUCCESS or
    // KERN_RESOURCE_SHORTAGE, both inside the byte range; a code
    // outside it could not name a defined error, so it becomes the
    // catch-all.
    let result = match u8::try_from(created) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    };

    match result {
        Ok(()) => Ok(child),
        Err(error) => Err(error),
    }
}

/// Create a task under `parent_task`, inheriting its memory when
/// `inherit_memory` is nonzero.  `task_create()` of kern/task.c.
///
/// The C refuses a null parent before it reaches `task_create_kernel()`;
/// the kernel task, created by that routine directly, is the one
/// creation without a parent.
///
/// # Safety
///
/// `parent_task` must be null or a live `struct task`, and
/// `child_task` must be writable for one pointer.  The caller must
/// hold no locks: creation may block on the memory and processor set
/// it takes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_create(
    parent_task: *mut c_void,
    inherit_memory: c_int,
    child_task: *mut *mut c_void,
) -> c_int {
    if parent_task.is_null() {
        return c_int::from(KernError::InvalidTask);
    }

    // SAFETY: the null check above is the C's, and the caller promises
    // the rest of the routine's contract.
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

/// Establish the restart PC of an interrupted atomic sequence.
/// `task_ras_control()` of kern/task.c.
///
/// The C body ignores its arguments and returns `KERN_FAILURE`:
/// restartable atomic sequences are not implemented.  The signature
/// keeps the four arguments the MIG server passes.
///
/// # Safety
///
/// The MIG server calls this with the task it converted from the
/// request port; nothing here reads or writes any argument.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_ras_control(
    _task: *mut c_void,
    _pc: VmOffset,
    _endpc: VmOffset,
    _flavor: c_int,
) -> c_int {
    c_int::from(KernError::Failure)
}

/// Register the port that gets a notification for every new task.
/// `register_new_task_notification()` of kern/task.c.
///
/// The registration is permanent: once a port is set, the C refuses a
/// second one with `KERN_NO_ACCESS`.
///
/// # Safety
///
/// `host` must be null or the live host privilege pointer the MIG stub
/// converted, and `notification` the port the request carried; the
/// caller holds no locks.  Nothing here reads the port, it is only
/// stored for `task_create_kernel()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn register_new_task_notification(
    host: *mut c_void,
    notification: *mut c_void,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidHost);
    }

    // SAFETY: the global is the C `ipc_port_t`, never borrowed as a
    // Rust reference; this read is the C body's own unlocked access.
    if !unsafe { glue::new_task_notification }.is_null() {
        return c_int::from(KernError::NoAccess);
    }

    // SAFETY: as above; the store is the C body's own.
    unsafe { glue::new_task_notification = notification };
    0
}
