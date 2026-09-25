// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_mig.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the MIG support and kernel-RPC module, one
//! adapter per symbol `kern/ipc_mig.c` used to define and `kern/ipc_mig.h`
//! declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::debug::kpanic;
use crate::kern::ipc_mig::{self, KERN_SUCCESS};
use crate::kern::thread::Thread;
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

/// `mig_strncpy()` of kern/ipc_mig.c.
///
/// # Safety
///
/// `dest` must be valid for writes of `len` bytes, and `src` must be readable
/// to its first NUL or for `len - 1` bytes, whichever comes first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_strncpy(
    dest: *mut c_char,
    src: *const c_char,
    len: VmSize,
) -> VmSize {
    // SAFETY: the caller's contract.
    unsafe { ipc_mig::mig_strncpy(dest, src, len) }
}

/// `mig_put_reply_port()` of kern/ipc_mig.c.
#[unsafe(no_mangle)]
pub extern "C" fn mig_put_reply_port(_reply_port: VmOffset) {}

/// `mig_dealloc_reply_port()` of kern/ipc_mig.c.
///
/// # Panics
///
/// Always halts through [`kpanic!`].
#[unsafe(no_mangle)]
pub extern "C" fn mig_dealloc_reply_port(_reply_port: VmOffset) {
    kpanic!("mig_dealloc_reply_port", "mig_dealloc_reply_port")
}

/// `mach_msg_rpc_from_kernel()` of kern/ipc_mig.c.
///
/// # Panics
///
/// Always halts through [`kpanic!`]: this kernel has never implemented
/// the call, and the C body was the same one `panic()`.
#[unsafe(no_mangle)]
pub extern "C" fn mach_msg_rpc_from_kernel(
    _msg: *const c_void,
    _send_size: c_uint,
    _reply_size: c_uint,
) -> c_int {
    kpanic!("mach_msg_rpc_from_kernel", "mach_msg_rpc_from_kernel")
}

/// `mach_msg_abort_rpc()` of kern/ipc_mig.c.
///
/// # Safety
///
/// `thread` must point at a live thread, and nothing may be locked, as the C
/// documented.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_abort_rpc(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { ipc_mig::abort_rpc(thread) };
}

/// `mig_get_reply_port()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Only the current thread's own reply-port field is read and written, which
/// nothing else touches until the thread dies.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_get_reply_port() -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_mig::mig_get_reply_port() }
}

/// `mig_deallocate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// A non-null `addr` must be the address of a live map-copy object the caller
/// owns and no longer uses; null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_deallocate(addr: VmOffset, size: VmSize) {
    // SAFETY: the caller's contract.
    unsafe { ipc_mig::mig_deallocate(addr, size) };
}

/// `thread_set_self_state()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap -77: a non-null `new_state` must be readable for
/// `new_state_count` `natural_t` words, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_self_state(
    flavor: c_int,
    new_state: *mut c_uint,
    new_state_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        ipc_mig::set_self_state(flavor, new_state, new_state_count)
    } {
        Ok(()) => KERN_SUCCESS,
        Err(error) => c_int::from(error),
    }
}

/// `mach_msg_send_from_kernel()` of kern/ipc_mig.c.
///
/// # Safety
///
/// `msg` must point at a readable kernel message of `send_size` bytes, and
/// the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_send_from_kernel(
    msg: *mut c_void,
    send_size: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { ipc_mig::mach_msg_send_from_kernel(msg, send_size) }.raw()
}

/// `mach_msg()` of kern/ipc_mig.c: like `mach_msg_trap()`, but the message
/// lives in kernel space.  It handles no options, and `time_out` is unused,
/// as in the C.
///
/// # Safety
///
/// `msg` must point at a readable and writable kernel message of the size the
/// selected option needs, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg(
    msg: *mut c_void,
    option: c_int,
    send_size: c_uint,
    rcv_size: c_uint,
    rcv_name: c_uint,
    time_out: c_uint,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        ipc_mig::mach_msg(
            msg, option, send_size, rcv_size, rcv_name, time_out, notify,
        )
    }
    .raw()
}

/// `syscall_vm_map()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 64 with user arguments: `address` must name user storage
/// of one address in the current map, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_vm_map(
    target_map: c_uint,
    address: *mut VmOffset,
    size: VmSize,
    mask: VmOffset,
    anywhere: c_int,
    memory_object: c_uint,
    offset: VmOffset,
    copy: c_int,
    cur_protection: c_int,
    max_protection: c_int,
    inheritance: c_int,
) -> c_int {
    let request = ipc_mig::VmMapRequest {
        target_map,
        address,
        size,
        mask,
        anywhere,
        memory_object,
        offset,
        copy,
        cur_protection,
        max_protection,
        inheritance,
    };

    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe { ipc_mig::syscall_vm_map(&request) })
}

/// `syscall_vm_allocate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 65 with user arguments: `address` must name user storage
/// of one address in the current map, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_vm_allocate(
    target_map: c_uint,
    address: *mut VmOffset,
    size: VmSize,
    anywhere: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_vm_allocate(target_map, address, size, anywhere)
    })
}

/// `syscall_vm_deallocate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 66 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_vm_deallocate(
    target_map: c_uint,
    start: VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_vm_deallocate(target_map, start, size)
    })
}

/// `syscall_task_create()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 68 with user arguments: `child_task` must name user
/// storage of one name in the current map, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_task_create(
    parent_task: c_uint,
    inherit_memory: c_int,
    child_task: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_task_create(parent_task, inherit_memory, child_task)
    })
}

/// `syscall_task_terminate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 69 with a user argument, and the caller must hold no
/// locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_task_terminate(task: c_uint) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe { ipc_mig::syscall_task_terminate(task) })
}

/// `syscall_task_suspend()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 70 with a user argument, and the caller must hold no
/// locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_task_suspend(task: c_uint) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe { ipc_mig::syscall_task_suspend(task) })
}

/// `syscall_task_set_special_port()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 71 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_task_set_special_port(
    task: c_uint,
    which_port: c_int,
    port_name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_task_set_special_port(task, which_port, port_name)
    })
}

/// `syscall_mach_port_allocate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 72 with user arguments: `namep` must name user storage of
/// one name in the current map, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_mach_port_allocate(
    task: c_uint,
    right: c_uint,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_mach_port_allocate(task, right, namep)
    })
}

/// `syscall_mach_port_allocate_name()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 75 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_mach_port_allocate_name(
    task: c_uint,
    right: c_uint,
    name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_mach_port_allocate_name(task, right, name)
    })
}

/// `syscall_mach_port_deallocate()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 73 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_mach_port_deallocate(
    task: c_uint,
    name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_mach_port_deallocate(task, name)
    })
}

/// `syscall_mach_port_insert_right()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 74 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_mach_port_insert_right(
    task: c_uint,
    name: c_uint,
    right: c_uint,
    right_type: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_mach_port_insert_right(task, name, right, right_type)
    })
}

/// `syscall_thread_depress_abort()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 76 with a user argument, and the caller must hold no
/// locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_thread_depress_abort(
    thread: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    ipc_mig::kern_return(unsafe {
        ipc_mig::syscall_thread_depress_abort(thread)
    })
}

/// `syscall_device_write_request()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 40 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_device_write_request(
    device_name: c_uint,
    reply_name: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: VmOffset,
    data_count: VmSize,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        ipc_mig::syscall_device_write_request(
            device_name,
            reply_name,
            mode,
            recnum,
            data,
            data_count,
        )
    }
}

/// `syscall_device_writev_request()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap 39 with user arguments, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_device_writev_request(
    device_name: c_uint,
    reply_name: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    iovec: *mut c_void,
    iocount: VmSize,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        ipc_mig::syscall_device_writev_request(
            device_name,
            reply_name,
            mode,
            recnum,
            iovec,
            iocount,
        )
    }
}
