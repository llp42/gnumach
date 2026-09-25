// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/mach_msg.c and ipc/mach_msg.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the message traps, one adapter per symbol
//! `ipc/mach_msg.c` used to define and `ipc/mach_msg.h` declares.
//!
//! `mach_msg_continue()` and `mach_msg_receive_continue()` are not here:
//! their addresses are the thread's saved continuation, which the C
//! compares against the symbols, so each has to be the single function
//! `ipc::mach_msg` defines.

use crate::ipc::mach_msg;
use crate::kern::thread::Thread;
use core::ffi::{c_int, c_uint, c_void};

/// `mach_msg_send()` of ipc/mach_msg.c.
///
/// # Safety
///
/// `msg` must name a readable user message of `send_size` bytes; the caller
/// holds no locks and permits an allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_send(
    msg: *mut c_void,
    option: c_int,
    send_size: c_uint,
    time_out: c_uint,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { mach_msg::send(msg, option, send_size, time_out, notify) }.raw()
}

/// `mach_msg_receive()` of ipc/mach_msg.c.
///
/// # Safety
///
/// `msg` must name a writable user message of `rcv_size` bytes; the caller
/// holds no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_receive(
    msg: *mut c_void,
    option: c_int,
    rcv_size: c_uint,
    rcv_name: c_uint,
    time_out: c_uint,
    notify: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        mach_msg::receive(msg, option, rcv_size, rcv_name, time_out, notify)
    }
    .raw()
}

/// `mach_msg_trap()` of ipc/mach_msg.c, the entry `mach_trap_table` holds.
///
/// # Safety
///
/// `msg` must name a readable and writable user message of the sizes
/// `option` selects; the caller is the trap dispatcher and holds no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_trap(
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
        mach_msg::trap(
            msg, option, send_size, rcv_size, rcv_name, time_out, notify,
        )
    }
    .raw()
}

/// `mach_msg_interrupt()` of ipc/mach_msg.c.
///
/// # Safety
///
/// `thread` must be a live thread that is not runnable, and its receive
/// state must be the one a blocked `mach_msg` left; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_interrupt(thread: *mut Thread) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { mach_msg::interrupt(thread) })
}
