// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_kmsg.c and ipc/ipc_kmsg.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the kernel-message module, one adapter per
//! symbol `ipc/ipc_kmsg.c` used to define and `ipc/ipc_kmsg.h` declares.

use crate::ipc::IpcSpace;
use crate::ipc::ipc_kmsg::{self, Kmsg, MsgReturn};
use crate::kern::thread::IpcKmsgQueue;
use crate::vm::vm_map::VmMap;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr;

/// `ipc_kmsg_enqueue()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `queue` must point at a live `struct ipc_kmsg_queue` this caller owns,
/// and `kmsg` at a live message not already queued.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_enqueue(
    queue: *mut c_void,
    kmsg: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::enqueue(queue.cast::<IpcKmsgQueue>(), kmsg) };
}

/// `ipc_kmsg_dequeue()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `queue` must point at a live `struct ipc_kmsg_queue`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_dequeue(queue: *mut c_void) -> *mut c_void {
    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::dequeue(queue.cast::<IpcKmsgQueue>()) } {
        Some(kmsg) => kmsg.as_ptr(),
        None => ptr::null_mut(),
    }
}

/// `ipc_kmsg_rmqueue()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `queue` must point at a live queue and `kmsg` at a live message in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_rmqueue(
    queue: *mut c_void,
    kmsg: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::rmqueue(queue.cast::<IpcKmsgQueue>(), kmsg) };
}

/// `ipc_kmsg_queue_next()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `queue` must point at a live queue and `kmsg` at a live message in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_queue_next(
    queue: *mut c_void,
    kmsg: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::queue_next(queue.cast::<IpcKmsgQueue>(), kmsg) } {
        Some(next) => next.as_ptr(),
        None => ptr::null_mut(),
    }
}

/// `ipc_kmsg_destroy()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_destroy(kmsg: *mut c_void) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::destroy(kmsg) };
}

/// `ipc_kmsg_clean()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_clean(kmsg: *mut c_void) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::clean(kmsg) };
}

/// `ipc_kmsg_free()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose storage this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_free(kmsg: *mut c_void) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::free(kmsg) };
}

/// `ipc_kmsg_get()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must name a readable user message of `size` bytes, `kmsgp` writable
/// storage for one message pointer written only on success, and the caller
/// permits an allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_get(
    msg: *mut c_void,
    size: c_uint,
    kmsgp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::get(msg, size) } {
        Ok(kmsg) => {
            // SAFETY: the caller promises the writable out-pointer; the C
            // writes it only on success.
            unsafe { kmsgp.write(kmsg.as_ptr()) };
            MsgReturn::SUCCESS.raw()
        }
        Err(error) => error.raw(),
    }
}

/// `ipc_kmsg_get_from_kernel()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must name a readable kernel message of `size` bytes, `kmsgp`
/// writable storage for one message pointer written only on success, and the
/// caller permits an allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_get_from_kernel(
    msg: *mut c_void,
    size: c_uint,
    kmsgp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::get_from_kernel(msg, size) } {
        Ok(kmsg) => {
            // SAFETY: the caller promises the writable out-pointer; the C
            // writes it only on success.
            unsafe { kmsgp.write(kmsg.as_ptr()) };
            MsgReturn::SUCCESS.raw()
        }
        Err(error) => error.raw(),
    }
}

/// `ipc_kmsg_put()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must name a writable user message of `size` bytes, and `kmsg` a
/// live message with clean header fields whose storage this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_put(
    msg: *mut c_void,
    kmsg: *mut c_void,
    size: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::put(msg, kmsg, size) }.raw()
}

/// `ipc_kmsg_put_to_kernel()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must name a writable kernel message of `size` bytes, and `kmsg` a
/// live message with clean header fields whose storage this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_put_to_kernel(
    msg: *mut c_void,
    kmsg: *mut c_void,
    size: c_uint,
) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::put_to_kernel(msg, kmsg, size) };
}

/// `ipc_kmsg_copyin_header()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must point at a live message header, `space` must be a live space,
/// and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyin_header(
    msg: *mut c_void,
    space: *mut c_void,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::copyin_header(msg.cast(), space, notify) } {
        Ok(()) => MsgReturn::SUCCESS.raw(),
        Err(error) => error.raw(),
    }
}

/// `ipc_kmsg_copyin()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose right the caller owns, `space` must
/// be live and unlocked, `map` must be the live map the message came from,
/// and the caller permits an allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyin(
    kmsg: *mut c_void,
    space: *mut c_void,
    map: *mut VmMap,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let map = unsafe { &mut *map };

    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::copyin(kmsg, space, map, notify) } {
        Ok(()) => MsgReturn::SUCCESS.raw(),
        Err(error) => error.raw(),
    }
}

/// `ipc_kmsg_copyin_from_kernel()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyin_from_kernel(kmsg: *mut c_void) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::copyin_from_kernel(kmsg) };
}

/// `ipc_kmsg_copyout_header()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `msg` must point at a live message header, `space` must be a live space,
/// and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout_header(
    msg: *mut c_void,
    space: *mut c_void,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_kmsg::copyout_header(msg.cast(), space, notify) } {
        Ok(()) => MsgReturn::SUCCESS.raw(),
        Err(error) => error.raw(),
    }
}

/// `ipc_kmsg_copyout_object()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `space` must be a live space and nothing may be locked; `namep` must be
/// writable storage for one name, written even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout_object(
    space: *mut c_void,
    object: *mut c_void,
    msgt_name: c_uint,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    let (mr, name) =
        unsafe { ipc_kmsg::copyout_object(space, object, msgt_name) };

    // SAFETY: the caller promises the writable out-pointer; the C writes it
    // on every path.
    unsafe { namep.write(name) };

    mr.raw()
}

/// `ipc_kmsg_copyout_body()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live complex message whose rights this call owns,
/// `space` must be live and unlocked, and `map` the live map the message is
/// being received into.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout_body(
    kmsg: *mut c_void,
    space: *mut c_void,
    map: *mut VmMap,
) -> c_int {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let map = unsafe { &mut *map };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::copyout_body(kmsg, space, map) }.raw()
}

/// `ipc_kmsg_copyout()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns, `space` must
/// be live and unlocked, `map` the live map the message is being received
/// into, and `notify` a name in `space` or `MACH_PORT_NULL`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout(
    kmsg: *mut c_void,
    space: *mut c_void,
    map: *mut VmMap,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let map = unsafe { &mut *map };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::copyout(kmsg, space, map, notify) }.raw()
}

/// `ipc_kmsg_copyout_pseudo()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns, `space` must
/// be live and unlocked, and `map` the live map the message is being
/// received into.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout_pseudo(
    kmsg: *mut c_void,
    space: *mut c_void,
    map: *mut VmMap,
) -> c_int {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };
    // SAFETY: the caller's contract.
    let map = unsafe { &mut *map };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::copyout_pseudo(kmsg, space, map) }.raw()
}

/// `ipc_kmsg_copyout_dest()` of ipc/ipc_kmsg.c.
///
/// # Safety
///
/// `kmsg` must be a live message whose rights this call owns, and `space`
/// must be live and unlocked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_kmsg_copyout_dest(
    kmsg: *mut c_void,
    space: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let kmsg = unsafe { Kmsg::from_raw(kmsg) };
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_kmsg::copyout_dest(kmsg, space) };
}
