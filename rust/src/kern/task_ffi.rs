// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Derived from include/mach/gnumach.defs:
//   Copyright (C) 2012 Free Software Foundation
// Derived from include/mach/mach.defs:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the task module, one adapter per symbol
//! `kern/task.c` used to define and `kern/task.h` declares.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::processor::ProcessorSet;
use crate::kern::task::{self, MapSource};
use crate::kern::types::KernError;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

/// `task_init()` of kern/task.c.
///
/// # Safety
///
/// Runs once, from the boot sequence, after the slab and IPC packages are
/// initialized and before any other task exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_init() {
    // SAFETY: the caller's contract.
    unsafe { task::init() };
}

/// `task_create_kernel()` of kern/task.c.
///
/// # Safety
///
/// `parent_task` must be null or a live task, and `child_task` must be
/// writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_create_kernel(
    parent_task: *mut c_void,
    inherit_memory: c_int,
    child_task: *mut *mut c_void,
) -> c_int {
    let source =
        if child_task.addr() == ptr::addr_of_mut!(task::kernel_task).addr() {
            MapSource::Kernel
        } else if inherit_memory != 0 {
            MapSource::Inherit
        } else {
            MapSource::Fresh
        };

    // SAFETY: the caller promises the parent and the writable slot; `create`
    // writes the slot's value on success only.
    match unsafe { task::create_kernel_task(parent_task.cast(), source) } {
        Ok(child) => {
            // SAFETY: the caller promises `child_task` is writable.
            unsafe { child_task.write(child.cast()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_deallocate()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or a live task the caller holds a reference to, and
/// the caller must hold no locks: the cleanup may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_deallocate(task: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { task::deallocate(task.cast()) };
}

/// `task_reference()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_reference(task: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { task::reference(task.cast()) };
}

/// `task_terminate()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or a live task, and the caller must hold no locks:
/// the routine blocks and deallocates.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_terminate(task: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::terminate(task.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_hold_locked()` of kern/task.c.
///
/// # Safety
///
/// The caller must hold `task`'s lock, and `task` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_hold_locked(task: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { task::hold_locked(task.cast()) };
}

/// `task_hold()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_hold(task: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::hold(task.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_dowait()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks: the
/// routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_dowait(
    task: *mut c_void,
    must_wait: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `must_wait` is the C boolean.
    match unsafe { task::dowait(task.cast(), must_wait != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_release()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_release(task: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::release(task.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_threads()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or a live task, and both output pointers must be
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_threads(
    task: *mut c_void,
    thread_list: *mut *mut VmOffset,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::threads(task.cast()) } {
        Ok((list, actual)) => {
            // SAFETY: the caller promises both slots writable; the C wrote
            // them only on success.
            unsafe {
                thread_list
                    .write(list.map_or(ptr::null_mut(), NonNull::as_ptr));
                count.write(actual);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_suspend()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_suspend(task: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::suspend(task.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_resume()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_resume(task: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::resume(task.cast()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_info()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, `task_info_out` must be writable for
/// `*task_info_count` `integer_t`s, and `task_info_count` must be readable
/// and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_info(
    task: *mut c_void,
    flavor: c_int,
    task_info_out: *mut c_int,
    task_info_count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller promises the count pointer is readable.
    let count = unsafe { *task_info_count };

    // SAFETY: the caller's contract.
    match unsafe { task::info(task.cast(), flavor, task_info_out, count) } {
        Ok(count) => {
            // SAFETY: the caller promises the count pointer is writable.
            unsafe { *task_info_count = count };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_assign()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, `new_pset` must be a live processor set, and
/// the caller must hold no locks: the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_assign(
    task: *mut c_void,
    new_pset: *mut ProcessorSet,
    assign_threads: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `assign_threads` is the C boolean.
    match unsafe { task::assign(task.cast(), new_pset, assign_threads != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_get_assignment()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and `pset` must be writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_get_assignment(
    task: *mut c_void,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { task::get_assignment(task.cast()) } {
        Ok(assigned) => {
            // SAFETY: the caller promises `pset` is writable.
            unsafe { pset.write(assigned) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_priority()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_priority(
    task: *mut c_void,
    priority: c_int,
    change_threads: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `change_threads` is the C boolean.
    match unsafe { task::priority(task.cast(), priority, change_threads != 0) }
    {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_set_name()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and `name` must be a NUL-terminated string,
/// as the C dereferenced it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_set_name(
    task: *mut c_void,
    name: *const c_char,
) -> c_int {
    // SAFETY: the caller promises the NUL-terminated string.
    let name = unsafe { CStr::from_ptr(name) };

    // SAFETY: the caller's contract.
    match unsafe { task::set_name(task.cast(), name.to_bytes()) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_set_essential()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_set_essential(
    task: *mut c_void,
    essential: c_int,
) -> c_int {
    // SAFETY: the caller's contract; `essential` is the C boolean.
    match unsafe { task::set_essential(task.cast(), essential != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `consider_task_collect()` of kern/task.c.
///
/// # Safety
///
/// The pageout daemon calls this with nothing locked, as the C did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn consider_task_collect() {
    // SAFETY: the caller's contract.
    unsafe { task::consider_collect() };
}

/// `task_max_priority()` of kern/task.c.
///
/// # Safety
///
/// `host` must be the port MIG converted from the request, `task` must be a
/// live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_max_priority(
    host: *mut c_void,
    task: *mut c_void,
    max_priority: c_int,
    set_priority: c_int,
    change_threads: c_int,
) -> c_int {
    // SAFETY: the caller's contract; the last two are the C booleans.
    match unsafe {
        task::max_priority(
            host,
            task.cast(),
            max_priority,
            set_priority != 0,
            change_threads != 0,
        )
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_assign_default()` of kern/task.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks: `assign()`
/// waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_assign_default(
    task: *mut c_void,
    assign_threads: c_int,
) -> c_int {
    // SAFETY: `default_pset` is the C global live for the life of the
    // kernel; the rest is the caller's contract.
    let default_pset =
        ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
    match unsafe {
        task::assign(task.cast(), default_pset, assign_threads != 0)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `task_create()` of kern/task.c.
///
/// # Safety
///
/// `parent_task` must be a live task, and `child_task` must be writable for
/// one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_create(
    parent_task: *mut c_void,
    inherit_memory: c_int,
    child_task: *mut *mut c_void,
) -> c_int {
    if parent_task.is_null() {
        return c_int::from(KernError::InvalidTask);
    }

    let source = if inherit_memory != 0 {
        MapSource::Inherit
    } else {
        MapSource::Fresh
    };

    // SAFETY: the null check above is the C's, and the caller promises the
    // rest of the routine's contract.
    match unsafe { task::create_kernel_task(parent_task.cast(), source) } {
        Ok(child) => {
            // SAFETY: the caller promises `child_task` is writable;
            // `create_kernel_task()` wrote its value on success only.
            unsafe { *child_task = child.cast() };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_ras_control()` of kern/task.c.
///
/// # Safety
///
/// The MIG server calls this with the task it converted from the request
/// port; nothing here reads or writes any argument.
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
    if !unsafe { task::new_task_notification }.is_null() {
        return c_int::from(KernError::NoAccess);
    }

    // SAFETY: as above; the store is the C body's own.
    unsafe { task::new_task_notification = notification };
    0
}
