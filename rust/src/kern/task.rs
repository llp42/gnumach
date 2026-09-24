// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Derived from include/mach/gnumach.defs:
//   Copyright (C) 2012 Free Software Foundation
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task entries of `kern/task.c`, which `kern/task.h` declares, and the
//! `struct task` mirror of `kern/task.h`.

use crate::arch::i386::machine_task::MachineTask;
use crate::arch::types::VmOffset;
use crate::glue;
use crate::glue::time_value::TimeValue64;
use crate::kern::lock::SimpleLock;
use crate::kern::processor::ProcessorSet;
use crate::kern::queue::QueueEntry;
use crate::kern::types::KernError;
use core::ffi::{c_char, c_int, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;

/// `TASK_PORT_REGISTER_MAX` of <mach/mach_param.h>: the registered send
/// rights a task holds.
const TASK_PORT_REGISTER_MAX: usize = 4;

/// `TASK_NAME_SIZE` of <kern/task.h>.
const TASK_NAME_SIZE: usize = 32;

/// `struct task` of <kern/task.h>: the task record itself.  The C packs the
/// three boolean flags into one `unsigned char`: `active` in bit 0,
/// `may_assign` in bit 1 and `essential` in bit 2.
#[repr(C)]
pub struct Task {
    /// `lock`: the task lock.
    pub lock: SimpleLock,
    pub ref_count: c_int,
    /// `assign_active`: waiting for `may_assign`.
    pub assign_active: u8,
    /// `active`, `may_assign` and `essential`, in that bit order.
    pub flags: u8,
    /// `map`: the address space.  `vm_map_t`, opaque here.
    pub map: *mut c_void,
    /// `pset_tasks`: link in the assigned processor set's task queue.
    pub pset_tasks: QueueEntry,
    pub suspend_count: c_int,
    /// `thread_list`: the task's thread queue head.
    pub thread_list: QueueEntry,
    pub thread_count: c_int,
    pub processor_set: *mut ProcessorSet,
    pub user_stop_count: c_int,
    pub priority: c_int,
    pub max_priority: c_int,
    pub total_user_time: TimeValue64,
    pub total_system_time: TimeValue64,
    pub creation_time: TimeValue64,
    /// `itk_lock_data`: protects the registered-port fields below.
    pub itk_lock_data: SimpleLock,
    pub itk_self: *mut c_void,
    pub itk_sself: *mut c_void,
    pub itk_exception: *mut c_void,
    pub itk_bootstrap: *mut c_void,
    pub itk_registered: [*mut c_void; TASK_PORT_REGISTER_MAX],
    pub itk_space: *mut c_void,
    pub eml_dispatch: *mut c_void,
    pub machine: MachineTask,
    pub faults: c_ulong,
    pub zero_fills: c_ulong,
    pub reactivations: c_ulong,
    pub pageins: c_ulong,
    pub cow_faults: c_ulong,
    pub messages_sent: c_ulong,
    pub messages_received: c_ulong,
    pub name: [c_char; TASK_NAME_SIZE],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Task>() == 336);
    assert!(align_of::<Task>() == 8);
    assert!(offset_of!(Task, lock) == 0);
    assert!(offset_of!(Task, ref_count) == 4);
    assert!(offset_of!(Task, assign_active) == 8);
    assert!(offset_of!(Task, flags) == 9);
    assert!(offset_of!(Task, map) == 16);
    assert!(offset_of!(Task, pset_tasks) == 24);
    assert!(offset_of!(Task, suspend_count) == 40);
    assert!(offset_of!(Task, thread_list) == 48);
    assert!(offset_of!(Task, thread_count) == 64);
    assert!(offset_of!(Task, processor_set) == 72);
    assert!(offset_of!(Task, user_stop_count) == 80);
    assert!(offset_of!(Task, priority) == 84);
    assert!(offset_of!(Task, max_priority) == 88);
    assert!(offset_of!(Task, total_user_time) == 96);
    assert!(offset_of!(Task, total_system_time) == 112);
    assert!(offset_of!(Task, creation_time) == 128);
    assert!(offset_of!(Task, itk_lock_data) == 144);
    assert!(offset_of!(Task, itk_self) == 152);
    assert!(offset_of!(Task, itk_sself) == 160);
    assert!(offset_of!(Task, itk_exception) == 168);
    assert!(offset_of!(Task, itk_bootstrap) == 176);
    assert!(offset_of!(Task, itk_registered) == 184);
    assert!(offset_of!(Task, itk_space) == 216);
    assert!(offset_of!(Task, eml_dispatch) == 224);
    assert!(offset_of!(Task, machine) == 232);
    assert!(offset_of!(Task, faults) == 248);
    assert!(offset_of!(Task, zero_fills) == 256);
    assert!(offset_of!(Task, reactivations) == 264);
    assert!(offset_of!(Task, pageins) == 272);
    assert!(offset_of!(Task, cow_faults) == 280);
    assert!(offset_of!(Task, messages_sent) == 288);
    assert!(offset_of!(Task, messages_received) == 296);
    assert!(offset_of!(Task, name) == 304);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Task>() == 220);
    assert!(align_of::<Task>() == 4);
    assert!(offset_of!(Task, lock) == 0);
    assert!(offset_of!(Task, ref_count) == 4);
    assert!(offset_of!(Task, assign_active) == 8);
    assert!(offset_of!(Task, flags) == 9);
    assert!(offset_of!(Task, map) == 12);
    assert!(offset_of!(Task, pset_tasks) == 16);
    assert!(offset_of!(Task, suspend_count) == 24);
    assert!(offset_of!(Task, thread_list) == 28);
    assert!(offset_of!(Task, thread_count) == 36);
    assert!(offset_of!(Task, processor_set) == 40);
    assert!(offset_of!(Task, user_stop_count) == 44);
    assert!(offset_of!(Task, priority) == 48);
    assert!(offset_of!(Task, max_priority) == 52);
    assert!(offset_of!(Task, total_user_time) == 56);
    assert!(offset_of!(Task, total_system_time) == 72);
    assert!(offset_of!(Task, creation_time) == 88);
    assert!(offset_of!(Task, itk_lock_data) == 104);
    assert!(offset_of!(Task, itk_self) == 108);
    assert!(offset_of!(Task, itk_sself) == 112);
    assert!(offset_of!(Task, itk_exception) == 116);
    assert!(offset_of!(Task, itk_bootstrap) == 120);
    assert!(offset_of!(Task, itk_registered) == 124);
    assert!(offset_of!(Task, itk_space) == 140);
    assert!(offset_of!(Task, eml_dispatch) == 144);
    assert!(offset_of!(Task, machine) == 148);
    assert!(offset_of!(Task, faults) == 160);
    assert!(offset_of!(Task, zero_fills) == 164);
    assert!(offset_of!(Task, reactivations) == 168);
    assert!(offset_of!(Task, pageins) == 172);
    assert!(offset_of!(Task, cow_faults) == 176);
    assert!(offset_of!(Task, messages_sent) == 180);
    assert!(offset_of!(Task, messages_received) == 184);
    assert!(offset_of!(Task, name) == 188);
};

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
