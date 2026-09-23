// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_fault.c and vm/vm_fault.h:
//   Copyright (c) 1994,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and
//   the Computer Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Page faults on a map entry, which `vm/vm_fault.c` used to define and
//! `vm/vm_fault.h` declares.

use crate::arch::i386::pmap::pmap_pageable;
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{vm_fault, vm_fault_wire_fast};
use crate::vm::error::KERN_SUCCESS;
use crate::vm::types::VmProt;
use crate::vm::vm_map::{VmMap, VmMapEntry};
use core::ffi::c_int;
use core::ptr::{self, NonNull};

/// `vm_fault_wire()` in C: wire down every page of `entry` in `map`.
///
/// # Safety
///
/// `entry` must be a live entry of `map`, and `map` must be referenced and
/// read-locked (or otherwise stable) for the whole call, as the C requires.
pub(crate) unsafe fn wire(map: &VmMap, entry: NonNull<VmMapEntry>) {
    // SAFETY: the caller promises a live entry; the links are read once,
    // before the faults below can change the entry's wiring.
    let (start, end) = unsafe {
        let links = &(*entry.as_ptr()).links;
        (links.start, links.end)
    };

    pmap_pageable(map.pmap, start, end, c_int::from(false));

    let map = ptr::from_ref(map).cast_mut();
    let mut va = start;
    while va < end {
        // SAFETY: `map` and `entry` are live and read-locked by the caller,
        // and `va` is an address the entry covers.
        let wired = unsafe { vm_fault_wire_fast(map, va, entry.as_ptr()) };
        if wired != KERN_SUCCESS {
            // SAFETY: as above; the C fault path takes the same map and
            // address, wires the page and passes no continuation.
            unsafe {
                vm_fault(
                    map,
                    va,
                    VmProt::NONE,
                    c_int::from(true),
                    c_int::from(false),
                    None,
                )
            };
        }
        va = va.wrapping_add(PAGE_SIZE);
    }
}
