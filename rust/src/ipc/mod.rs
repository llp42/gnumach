// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! IPC facilities; mirrors `ipc/`.
//!
//! `IpcPort` and `IpcSpace` are the two IPC handles the VM map port
//! needs so far: `struct ipc_port *` and `struct ipc_space *`, opaque
//! to Rust until `ipc/ipc_port.c` and `ipc/ipc_space.c` move.

use core::ffi::c_void;
use core::ptr::NonNull;

pub mod ipc_object;
pub mod ipc_port;
pub mod ipc_table;
pub mod ipc_thread;
pub mod mach_port;

/// `ipc_port_t`: a send right to a kernel port, opaque to Rust so far.
/// `None` is the C `IP_NULL`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcPort(NonNull<c_void>);

impl IpcPort {
    /// A port from the C side, or `None` for `IP_NULL`.
    pub(crate) fn new(port: *mut c_void) -> Option<Self> {
        NonNull::new(port).map(Self)
    }

    /// The raw pointer the C side passes.
    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }
}

/// `ipc_space_t`: a port namespace, opaque to Rust so far.  `None` is
/// the C `IS_NULL`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcSpace(NonNull<c_void>);

impl IpcSpace {
    /// A space from the C side, or `None` for `IS_NULL`.
    pub(crate) fn new(space: *mut c_void) -> Option<Self> {
        NonNull::new(space).map(Self)
    }

    /// The raw pointer the C side passes.
    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }
}
