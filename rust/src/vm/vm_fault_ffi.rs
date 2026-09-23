// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_fault.c and vm/vm_fault.h:
//   Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the map-entry page faults, one adapter per symbol
//! `vm/vm_fault.c` used to define and `vm/vm_fault.h` declares.

use crate::vm::vm_fault;
use crate::vm::vm_map::{VmMap, VmMapEntry};
use core::ptr::NonNull;

/// `vm_fault_wire()` in C.
///
/// # Safety
///
/// `map` must point at a live, referenced map and `entry` at a live entry of
/// it, and the caller must hold the map's read lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_fault_wire(
    map: *mut VmMap,
    entry: *mut VmMapEntry,
) {
    // SAFETY: the caller promises a live map and entry.
    unsafe { vm_fault::wire(&*map, NonNull::new_unchecked(entry)) };
}
