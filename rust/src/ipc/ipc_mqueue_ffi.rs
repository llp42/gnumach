// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_mqueue.c and ipc/ipc_mqueue.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the message queues, one adapter per symbol
//! `ipc/ipc_mqueue.c` used to define and `ipc/ipc_mqueue.h` declares.

use crate::ipc::ipc_kmsg::MsgReturn;
use crate::ipc::ipc_mqueue::{self, Received};
use crate::ipc::{IpcMqueue, IpcPort, IpcSpace};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_mqueue_init()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// `mqueue` must point at writable storage for a fresh message queue that no
/// other thread can see.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_init(mqueue: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_mqueue::init(mqueue.cast::<IpcMqueue>()) };
}

/// `ipc_mqueue_move()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// `dest` and `source` must point at live message queues, both locked, and
/// `port` must be a live port those locks keep alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_move(
    dest: *mut c_void,
    source: *mut c_void,
    port: *const c_void,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port.cast_mut()) };

    // SAFETY: the caller's contract.
    unsafe {
        ipc_mqueue::move_messages(
            dest.cast::<IpcMqueue>(),
            source.cast::<IpcMqueue>(),
            port,
        )
    };
}

/// `ipc_mqueue_changed()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// `mqueue` must point at a live locked message queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_changed(mqueue: *mut c_void, mr: c_int) {
    // SAFETY: the caller's contract.
    unsafe {
        ipc_mqueue::changed(
            mqueue.cast::<IpcMqueue>(),
            MsgReturn::from_raw(mr),
        )
    };
}

/// `ipc_mqueue_send()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// `kmsg` must be a live message the caller owns, holding a reference for the
/// destination port; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_send(
    kmsg: *mut c_void,
    option: c_uint,
    time_out: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { ipc_mqueue::send(kmsg, option, time_out) }.raw()
}

/// `ipc_mqueue_copyin()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, nothing may be locked, and `mqueuep`
/// and `objectp` must be writable storage for one queue and one object
/// pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_copyin(
    space: *mut c_void,
    name: c_uint,
    mqueuep: *mut *mut c_void,
    objectp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_mqueue::copyin(space, name) } {
        Ok(found) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                mqueuep.write(found.mqueue.cast());
                objectp.write(found.object);
            }
            0
        }
        Err(code) => code.raw(),
    }
}

/// `ipc_mqueue_receive()` of ipc/ipc_mqueue.c.
///
/// # Safety
///
/// The message queue must be locked unless `resume` is set, in which case the
/// call resumes the receive `continuation` began; the caller must hold a
/// reference for the port or port set the queue belongs to, and `kmsgp` and
/// `seqnop` must be writable storage for one message pointer and one sequence
/// number.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_mqueue_receive(
    mqueue: *mut c_void,
    option: c_uint,
    max_size: c_uint,
    time_out: c_uint,
    resume: c_int,
    continuation: Option<unsafe extern "C" fn()>,
    kmsgp: *mut *mut c_void,
    seqnop: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        ipc_mqueue::receive(
            mqueue.cast::<IpcMqueue>(),
            option,
            max_size,
            time_out,
            resume != 0,
            continuation,
        )
    } {
        Received::Kmsg { kmsg, seqno } => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                kmsgp.write(kmsg.as_ptr());
                seqnop.write(seqno);
            }
            0
        }
        Received::TooLarge { size } => {
            // The C wrote the size into the `kmsgp` slot, which the caller
            // reads back through a `mach_msg_size_t` view.
            // SAFETY: the caller promises `kmsgp` writable for one size.
            unsafe { kmsgp.cast::<c_uint>().write(size) };
            MsgReturn::RCV_TOO_LARGE.raw()
        }
        Received::Failed { code } => code.raw(),
    }
}
