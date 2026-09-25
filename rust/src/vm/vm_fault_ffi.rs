// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_fault.c and vm/vm_fault.h:
//   Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the page faults, one adapter per symbol
//! `vm/vm_fault.c` used to define and `vm/vm_fault.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::vm::types::{VmObject, VmPage};
use crate::vm::vm_fault;
use crate::vm::vm_map::{VmMap, VmMapEntry, VmMapVersion};
use core::ffi::c_int;
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

/// `vm_fault_cleanup()` in C.
///
/// # Safety
///
/// `object` must be a live object whose lock and paging reference the caller
/// holds; `top_page` must be null or the busy top page the fault left.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_fault_cleanup(
    object: *mut VmObject,
    top_page: *mut VmPage,
) {
    // SAFETY: the caller promises the locked object and its page.
    unsafe { vm_fault::cleanup(object, top_page) };
}

/// `vm_fault_wire_fast()` in C.
///
/// # Safety
///
/// `map` must point at the live, referenced, read-locked map `entry` belongs
/// to; `entry` must be a live entry of it and `va` an address it covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_fault_wire_fast(
    map: *mut VmMap,
    va: VmOffset,
    entry: *mut VmMapEntry,
) -> c_int {
    // SAFETY: the caller promises the live map and entry.
    unsafe { vm_fault::wire_fast(&*map, va, entry) }
}

/// `vm_fault_unwire()` in C.
///
/// # Safety
///
/// `map` must point at a live, referenced map and `entry` at a live entry of
/// it whose pages are wired down.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_fault_unwire(
    map: *mut VmMap,
    entry: *mut VmMapEntry,
) {
    // SAFETY: the caller promises the live map and entry.
    unsafe { vm_fault::unwire(&*map, NonNull::new_unchecked(entry)) };
}

/// `vm_fault_copy()` in C.
///
/// # Safety
///
/// The caller must hold a reference, but not a lock, to each object and to
/// `dst_map`; `src_size` must be writable and name the bytes to copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_fault_copy(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    src_size: *mut VmSize,
    dst_object: *mut VmObject,
    dst_offset: VmOffset,
    dst_map: *mut VmMap,
    dst_version: *mut VmMapVersion,
    interruptible: c_int,
) -> c_int {
    // SAFETY: the caller promises the live objects, map and version, and the
    // writable size.
    unsafe {
        vm_fault::copy(
            src_object,
            src_offset,
            &mut *src_size,
            dst_object,
            dst_offset,
            NonNull::new_unchecked(dst_map),
            &*dst_version,
            interruptible != 0,
        )
    }
}
