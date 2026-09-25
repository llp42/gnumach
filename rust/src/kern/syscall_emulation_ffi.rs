// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_emulation.c and kern/syscall_emulation.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/syscall_emulation.c`, which the file used to
//! define for <kern/syscall_emulation.h> and the MIG `mach` interface.

use crate::arch::types::VmOffset;
use crate::kern::syscall_emulation;
use crate::kern::task::Task;
use core::ffi::{c_int, c_uint};

/// `eml_task_reference()` of kern/syscall_emulation.c.
///
/// # Safety
///
/// `task` must be a live task, and `parent` null or a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eml_task_reference(
    task: *mut Task,
    parent: *mut Task,
) {
    // SAFETY: the caller's contract.
    unsafe { syscall_emulation::task_reference(task, parent) };
}

/// `eml_task_deallocate()` of kern/syscall_emulation.c.
///
/// # Safety
///
/// `task` must be a live task whose emulation vector belongs to the task
/// this call is deallocating.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eml_task_deallocate(task: *mut Task) {
    // SAFETY: the caller's contract.
    unsafe { syscall_emulation::task_deallocate(task) };
}

/// `task_set_emulation_vector()` of kern/syscall_emulation.c, the MIG
/// `mach` server entry.
///
/// # Safety
///
/// `task` must be null or a live task, and `emulation_vector` a live
/// `vm_map_copy_t` of `emulation_vector_count` entries or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_set_emulation_vector(
    task: *mut Task,
    vector_start: c_int,
    emulation_vector: *mut VmOffset,
    emulation_vector_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        syscall_emulation::set_vector(
            task,
            vector_start,
            emulation_vector,
            emulation_vector_count,
        )
    }
}

/// `task_get_emulation_vector()` of kern/syscall_emulation.c, the MIG
/// `mach` server entry.
///
/// # Safety
///
/// `task` must be null or a live task, and the three out-pointers writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_get_emulation_vector(
    task: *mut Task,
    vector_start: *mut c_int,
    emulation_vector: *mut *mut VmOffset,
    emulation_vector_count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { syscall_emulation::get_vector(task) } {
        Ok(vector) => {
            // SAFETY: the caller promises the three writable out-pointers,
            // which the C filled on this path.
            unsafe {
                vector_start.write(vector.start);
                emulation_vector.write(vector.vector);
                emulation_vector_count.write(vector.count);
            }
            0
        }
        Err(error) => error,
    }
}

/// `task_set_emulation()` of kern/syscall_emulation.c, the MIG `mach`
/// server entry.
///
/// # Safety
///
/// `task` must be null or a live task; `routine_entry_pt` is the entry
/// address the task will dispatch to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_set_emulation(
    task: *mut Task,
    routine_entry_pt: VmOffset,
    routine_number: c_int,
) -> c_int {
    let mut routine = routine_entry_pt;
    // SAFETY: the caller's contract; `routine` is the one-entry vector the C
    // built on its stack.
    unsafe {
        syscall_emulation::set_vector_internal(
            task,
            routine_number,
            &raw mut routine,
            1,
        )
    }
}

/// `eml_init()` of kern/syscall_emulation.c.  The C body is empty.
#[unsafe(no_mangle)]
pub extern "C" fn eml_init() {}
