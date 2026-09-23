// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_target.c and ipc/ipc_target.h:
//   Copyright (c) 1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The common part of IPC ports and port sets, which `ipc/ipc_target.h`
//! declares.

use core::ffi::c_void;

/// `ipc_target_terminate()` of ipc/ipc_target.c.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_target_terminate(_ipt: *mut c_void) {}
