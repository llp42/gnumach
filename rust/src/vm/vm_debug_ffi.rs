// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_debug.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Derived from mach_debug/vm_info.h and mach_debug/hash_info.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the VM debugging calls, one adapter per symbol
//! `vm/vm_debug.c` used to define and `kern/mach_debug.server.h` declares.

use crate::arch::types::VmOffset;
use crate::ipc::HashInfoBucket;
use crate::kern::host::Host;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::types::VmObject;
use crate::vm::vm_debug::{
    self, VmObjectInfo, VmPageInfo, VmPagePhysInfo, VmRegionInfo,
};
use crate::vm::vm_map::VmMap;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::NonNull;

/// `mach_vm_region_info()` of vm/vm_debug.c.
///
/// # Safety
///
/// `map` must be null or a live, unlocked map; `regionp` and `portp` must be
/// writable storage, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_vm_region_info(
    map: *mut VmMap,
    address: VmOffset,
    regionp: *mut VmRegionInfo,
    portp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { vm_debug::region_info(map, address) } {
        Ok((info, port)) => {
            // SAFETY: the caller promises the writable out-pointers; the C
            // writes them only on success.
            unsafe {
                regionp.write(info);
                portp.write(port);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `mach_vm_object_info()` of vm/vm_debug.c.
///
/// # Safety
///
/// `object` must be null or a live object; `infop`, `shadowp` and `copyp`
/// must be writable storage, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_vm_object_info(
    object: *mut VmObject,
    infop: *mut VmObjectInfo,
    shadowp: *mut *mut c_void,
    copyp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { vm_debug::object_info(object) } {
        Ok((info, shadow, copy)) => {
            // SAFETY: the caller promises the writable out-pointers; the C
            // writes them only on success.
            unsafe {
                infop.write(info);
                shadowp.write(shadow);
                copyp.write(copy);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `mach_vm_object_pages()` of vm/vm_debug.c.
///
/// # Safety
///
/// `object` must be null or a live object; `pagesp` must be writable storage
/// for one array pointer and `countp` for one count; the caller permits an
/// allocation and a kernel-map copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_vm_object_pages(
    object: *mut VmObject,
    pagesp: *mut *mut VmPageInfo,
    countp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        vm_debug::object_pages_info(
            object,
            pagesp.cast::<*mut c_void>(),
            countp,
        )
    } {
        Ok(()) => KERN_SUCCESS,
        Err(error) => error.as_kern_return(),
    }
}

/// `mach_vm_object_pages_phys()` of vm/vm_debug.c.
///
/// # Safety
///
/// `object` must be null or a live object; `pagesp` must be writable storage
/// for one array pointer and `countp` for one count; the caller permits an
/// allocation and a kernel-map copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_vm_object_pages_phys(
    object: *mut VmObject,
    pagesp: *mut *mut VmPagePhysInfo,
    countp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        vm_debug::object_pages_phys(
            object,
            pagesp.cast::<*mut c_void>(),
            countp,
        )
    } {
        Ok(()) => KERN_SUCCESS,
        Err(error) => error.as_kern_return(),
    }
}

/// `host_virtual_physical_table_info()` of vm/vm_debug.c.
///
/// # Safety
///
/// `host` must be null or the live host pointer the generated server
/// converted the request port into; `infop` and `countp` must be writable
/// storage, and the caller permits an allocation and a kernel-map copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_virtual_physical_table_info(
    host: *mut Host,
    infop: *mut *mut HashInfoBucket,
    countp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        vm_debug::virtual_physical_table_info(
            NonNull::new(host),
            infop,
            countp,
        )
    } {
        Ok(()) => KERN_SUCCESS,
        Err(error) => error.as_kern_return(),
    }
}
