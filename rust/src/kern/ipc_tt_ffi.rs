// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_tt.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the task and thread IPC module, one adapter per
//! symbol `kern/ipc_tt.c` used to define and `kern/ipc_tt.h` declares.

use crate::arch::types::VmOffset;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::ipc_tt::{self, TaskSpecialPort, ThreadSpecialPort};
use crate::kern::slab::kfree;
use crate::kern::task::{TASK_PORT_REGISTER_MAX, Task};
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use crate::vm::vm_map::VmMap;
use core::ffi::{c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr::{self, NonNull};
use core::slice;

/// `ipc_task_init()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a fresh task whose IPC fields are unwritten, `parent` must
/// be null or a live initialized task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_task_init(task: *mut Task, parent: *mut Task) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_task_init(task, parent) };
}

/// `ipc_task_enable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a live task whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_task_enable(task: *mut Task) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_task_enable(task) };
}

/// `ipc_task_disable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a live task whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_task_disable(task: *mut Task) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_task_disable(task) };
}

/// `ipc_task_terminate()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a live, suspended task, or the current thread's own task,
/// and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_task_terminate(task: *mut Task) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_task_terminate(task) };
}

/// `ipc_thread_init()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a fresh thread whose IPC fields are unwritten, and the
/// caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_init(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_thread_init(thread) };
}

/// `ipc_thread_enable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a live thread whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_enable(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_thread_enable(thread) };
}

/// `ipc_thread_disable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a live thread whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_disable(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_thread_disable(thread) };
}

/// `ipc_thread_terminate()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a live, suspended thread, or the current thread, and the
/// caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_terminate(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::ipc_thread_terminate(thread) };
}

/// `retrieve_task_self_fast()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn retrieve_task_self_fast(
    task: *mut Task,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::retrieve_task_self_fast(task) }
        .map_or(ptr::null_mut(), IpcPort::as_ptr)
}

/// `retrieve_thread_self_fast()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a live thread, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn retrieve_thread_self_fast(
    thread: *mut Thread,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::retrieve_thread_self_fast(thread) }
        .map_or(ptr::null_mut(), IpcPort::as_ptr)
}

/// `mach_task_self()` of kern/ipc_tt.c, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context, with nothing locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_task_self() -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::mach_task_self() }
}

/// `mach_thread_self()` of kern/ipc_tt.c, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context, with nothing locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_thread_self() -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::mach_thread_self() }
}

/// `mach_reply_port()` of kern/ipc_tt.c, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context, with nothing locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_reply_port() -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::mach_reply_port() }
}

/// `task_get_special_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be null or a live task, `portp` must be writable, and the
/// caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_get_special_port(
    task: *mut Task,
    which: c_int,
    portp: *mut *mut c_void,
) -> c_int {
    let Some(which) = TaskSpecialPort::from_int(which) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::task_get_special_port(task, which) } {
        Ok(port) => {
            // SAFETY: the caller promises `portp` is writable; the C wrote
            // it only on success.
            unsafe {
                portp.write(port.map_or(ptr::null_mut(), IpcPort::as_ptr))
            };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `task_set_special_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be null or a live task, `port` must be a naked send right or
/// `IP_NULL`, and the caller must hold no locks; on success the right is
/// consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn task_set_special_port(
    task: *mut Task,
    which: c_int,
    port: *mut c_void,
) -> c_int {
    let Some(which) = TaskSpecialPort::from_int(which) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::task_set_special_port(task, which, port) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `thread_get_special_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be null or a live thread, `portp` must be writable, and the
/// caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_get_special_port(
    thread: *mut Thread,
    which: c_int,
    portp: *mut *mut c_void,
) -> c_int {
    let Some(which) = ThreadSpecialPort::from_int(which) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::thread_get_special_port(thread, which) } {
        Ok(port) => {
            // SAFETY: the caller promises `portp` is writable; the C wrote
            // it only on success.
            unsafe {
                portp.write(port.map_or(ptr::null_mut(), IpcPort::as_ptr))
            };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `thread_set_special_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be null or a live thread, `port` must be a naked send right
/// or `IP_NULL`, and the caller must hold no locks; on success the right is
/// consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_special_port(
    thread: *mut Thread,
    which: c_int,
    port: *mut c_void,
) -> c_int {
    let Some(which) = ThreadSpecialPort::from_int(which) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::thread_set_special_port(thread, which, port) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `mach_ports_register()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be null or a live task, `memory` must be a `kalloc`'d array of
/// `ports_cnt` `mach_port_t`s the caller gives up on success, and the caller
/// must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_ports_register(
    task: *mut Task,
    memory: *mut VmOffset,
    ports_cnt: c_uint,
) -> c_int {
    // `mach_msg_type_number_t` is a `u32`, and `usize` holds it on both
    // targets.
    if ports_cnt as usize > TASK_PORT_REGISTER_MAX {
        return c_int::from(KernError::InvalidArgument);
    }
    let count = ports_cnt as usize;

    let ports: &[VmOffset] = if count == 0 {
        &[]
    } else {
        // SAFETY: the caller promises `memory` is readable for `ports_cnt`
        // `mach_port_t`s, and the check above bounded the count.
        unsafe { slice::from_raw_parts(memory, count) }
    };

    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::ports_register(task, ports) } {
        Ok(()) => {
            if ports_cnt != 0 {
                // SAFETY: the caller promises the array came from `kalloc`
                // with exactly this byte size, and the register call returns
                // success before the free.
                unsafe {
                    kfree(
                        NonNull::new_unchecked(memory.cast::<u8>()),
                        count * size_of::<VmOffset>(),
                    );
                }
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `mach_ports_lookup()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be null or a live task, both out-pointers must be writable,
/// and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_ports_lookup(
    task: *mut Task,
    portsp: *mut *mut VmOffset,
    ports_cnt: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { ipc_tt::ports_lookup(task) } {
        Ok((ports, count)) => {
            // SAFETY: the caller promises both out-pointers are writable; the
            // C wrote them only on success.
            unsafe {
                portsp.write(ports.as_ptr());
                ports_cnt.write(count);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `convert_port_to_task()` of kern/ipc_tt.c.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port, and the caller must
/// hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_task(port: *mut c_void) -> *mut Task {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_port_to_task(port) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `convert_port_to_space()` of kern/ipc_tt.c.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port, and the caller must
/// hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_space(
    port: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_port_to_space(port) }
        .map_or(ptr::null_mut(), IpcSpace::as_ptr)
}

/// `convert_port_to_map()` of kern/ipc_tt.c.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port, and the caller must
/// hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_map(port: *mut c_void) -> *mut VmMap {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_port_to_map(port) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `convert_port_to_thread()` of kern/ipc_tt.c.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port, and the caller must
/// hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_thread(
    port: *mut c_void,
) -> *mut Thread {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_port_to_thread(port) }
        .map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// `convert_task_to_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `task` must be a live task the caller holds a reference to; the routine
/// consumes the reference and may deallocate the task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_task_to_port(task: *mut Task) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_task_to_port(task) }
        .map_or(ptr::null_mut(), IpcPort::as_ptr)
}

/// `convert_thread_to_port()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must be a live thread the caller holds a reference to; the
/// routine consumes the reference and may deallocate the thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_thread_to_port(
    thread: *mut Thread,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::convert_thread_to_port(thread) }
        .map_or(ptr::null_mut(), IpcPort::as_ptr)
}

/// `space_deallocate()` of kern/ipc_tt.c: the `is_release()` of a space ref
/// `convert_port_to_space()` produced.
///
/// # Safety
///
/// A non-null `space` must be a live space the caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn space_deallocate(space: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_tt::space_deallocate(space) };
}
