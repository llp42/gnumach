// SPDX-License-Identifier: CMU-Mach
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task record's processor-set entries, which `kern/task.h`
//! declares.
//!
//! Only [`task_assign_default()`] moves here so far.  The rest of
//! `kern/task.c` stays C, and so does `struct task`: it embeds an
//! `ipc_space`, a `vm_map` and the emulation vector, none of which has
//! a Rust mirror, so the task is spelled as an opaque pointer here and
//! only passed along.

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
