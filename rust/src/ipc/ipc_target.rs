// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_target.c and ipc/ipc_target.h:
//   Copyright (c) 1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The common part of IPC ports and port sets, which
//! `ipc/ipc_target.h` declares.
//!
//! Only [`ipc_target_terminate()`] moves here so far.
//! `ipc_target_init()` stays C: it writes `ipt_name` and calls the
//! `ipc_mqueue_init()` macro chain over `struct ipc_target`, which
//! has no Rust mirror yet.

use core::ffi::c_void;

/// Tear down the common part of a port or port set.
/// `ipc_target_terminate()` of ipc/ipc_target.c.
///
/// The C body is empty and reads none of the record: the message
/// queue `ipc_target_init()` set up is drained by the caller
/// (`ipc_port_destroy()` and `ipc_pset_destroy()` both call
/// `ipc_mqueue_changed()` first), and `ipt_name` owns nothing.  The
/// symbol stays because both callers name it, and it is where the
/// migrating-RPC teardown would go.
///
/// The argument is a `struct ipc_target *`, spelled opaque here
/// because nothing reads it.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_target_terminate(_ipt: *mut c_void) {}
