// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The port timestamp, which `ipc/ipc_port.c` used to define and
//! `ipc/ipc_port.h` declares.
//!
//! Only [`ipc_port_timestamp()`] and the two globals it owns have
//! moved; the rest of `ipc/ipc_port.c` stays C until its space and
//! object locks have Rust homes.

use crate::kern::lock::SimpleLock;
use core::ffi::c_uint;
use core::sync::atomic::{AtomicU32, Ordering};

/// `ipc_port_timestamp_lock_data` of ipc/ipc_port.c: serializes the
/// counter's read and post-increment.
///
/// C still names this exact symbol from `ipc/ipc_init.c`, so the name
/// and the `struct slock` layout are both kept.
#[unsafe(export_name = "ipc_port_timestamp_lock_data")]
static TIMESTAMP_LOCK: SimpleLock = SimpleLock::new();

/// `ipc_port_timestamp_data` of ipc/ipc_port.c: the next timestamp to
/// hand out, wrapping at 2^32 as its `unsigned int` does.
///
/// `ipc_bootstrap()` writes the initial zero before any other thread
/// can reach it; after that every access is a `Relaxed` read or write
/// under [`TIMESTAMP_LOCK`], whose acquire and release order the
/// counter against other callers.
#[unsafe(export_name = "ipc_port_timestamp_data")]
static TIMESTAMP_DATA: AtomicU32 = AtomicU32::new(0);

/// Returns a timestamp value.  `ipc_port_timestamp()` in C.
///
/// The counter increments modulo 2^32, matching the `unsigned int`
/// arithmetic the C used.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_port_timestamp() -> c_uint {
    TIMESTAMP_LOCK.lock();

    let timestamp = TIMESTAMP_DATA.load(Ordering::Relaxed);
    TIMESTAMP_DATA.store(timestamp.wrapping_add(1), Ordering::Relaxed);

    TIMESTAMP_LOCK.unlock();

    timestamp
}
