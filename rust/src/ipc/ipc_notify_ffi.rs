// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_notify.c and ipc/ipc_notify.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the notification senders, one adapter per
//! symbol `ipc/ipc_notify.c` used to define and `ipc/ipc_notify.h`
//! declares.

use crate::ipc::ipc_notify;
use core::ffi::{c_uint, c_void};

/// `ipc_notify_init()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// The boot path must call this once, before any notification is sent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_init() {
    ipc_notify::init();
}

/// `ipc_notify_port_deleted()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_port_deleted(
    port: *mut c_void,
    name: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::port_deleted(port, name) };
}

/// `ipc_notify_msg_accepted()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_msg_accepted(
    port: *mut c_void,
    name: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::msg_accepted(port, name) };
}

/// `ipc_notify_port_destroyed()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes, and `right` a receive right the message takes over; nothing may
/// be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_port_destroyed(
    port: *mut c_void,
    right: *mut c_void,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::port_destroyed(port, right) };
}

/// `ipc_notify_no_senders()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_no_senders(
    port: *mut c_void,
    mscount: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::no_senders(port, mscount) };
}

/// `ipc_notify_send_once()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_send_once(port: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::send_once(port) };
}

/// `ipc_notify_dead_name()` of ipc/ipc_notify.c.
///
/// # Safety
///
/// `port` must be a live port holding the send-once right the notification
/// consumes; nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_notify_dead_name(
    port: *mut c_void,
    name: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_notify::dead_name(port, name) };
}
