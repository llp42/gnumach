// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/mach_debug.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the mach-debug calls, one adapter per symbol
//! `ipc/mach_debug.c` used to define and `kern/mach_debug.server.h`
//! declares.

use crate::arch::types::VmOffset;
use crate::ipc::mach_debug;
use crate::ipc::{HashInfoBucket, IpcSpace};
use crate::kern::host::Host;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::NonNull;

/// `mach_port_get_srights()` of ipc/mach_debug.c.
///
/// # Safety
///
/// `space` must be null or a live `ipc_space`; `srightsp` must be writable
/// storage for one right count, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_get_srights(
    space: *mut c_void,
    name: c_uint,
    srightsp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { mach_debug::get_srights(IpcSpace::new(space), name) } {
        Ok(srights) => {
            // SAFETY: the caller promises the writable out-pointer; the C
            // writes it only on success.
            unsafe { srightsp.write(srights) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_ipc_marequest_info()` of ipc/mach_debug.c.
///
/// # Safety
///
/// `host` must be null or the live host pointer the generated server
/// converted the request port into; `maxp` and `countp` must be writable
/// storage for one count, and `infop` for one bucket-array pointer.  The
/// caller permits an allocation and a kernel-map copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_ipc_marequest_info(
    host: *mut Host,
    maxp: *mut c_uint,
    infop: *mut *mut HashInfoBucket,
    countp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        mach_debug::marequest_info(NonNull::new(host), maxp, infop, countp)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `mach_port_dnrequest_info()` of ipc/mach_debug.c.
///
/// # Safety
///
/// `space` must be null or a live `ipc_space`; `totalp` and `usedp` must be
/// writable storage for one count each, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_dnrequest_info(
    space: *mut c_void,
    name: c_uint,
    totalp: *mut c_uint,
    usedp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { mach_debug::dnrequest_info(IpcSpace::new(space), name) } {
        Ok((total, used)) => {
            // SAFETY: the caller promises the writable out-pointers; the C
            // writes them only on success.
            unsafe {
                totalp.write(total);
                usedp.write(used);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `mach_port_kernel_object()` of ipc/mach_debug.c.
///
/// # Safety
///
/// `space` must be null or a live `ipc_space`; `typep` must be writable
/// storage for one kobject type and `addrp` for one address, both written
/// only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_kernel_object(
    space: *mut c_void,
    name: c_uint,
    typep: *mut c_uint,
    addrp: *mut VmOffset,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { mach_debug::kernel_object(IpcSpace::new(space), name) } {
        Ok((object_type, object_addr)) => {
            // SAFETY: the caller promises the writable out-pointers; the C
            // writes them only on success.
            unsafe {
                typep.write(object_type);
                addrp.write(object_addr);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}
