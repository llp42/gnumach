// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from vm/memory_object_proxy.c and vm/memory_object_proxy.h:
//   Copyright (C) 2005, 2011 Free Software Foundation, Inc.
//   Written by Marcus Brinkmann.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the proxy memory objects, one adapter per symbol
//! `vm/memory_object_proxy.c` used to define and `vm/memory_object_proxy.h`
//! or `kern/mach4.server.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::ipc::IpcSpace;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::memory_object_proxy;
use crate::vm::types::VmProt;
use core::ffi::{c_int, c_uint, c_void};

/// A MIG array as a slice; an absent array is an empty one.
///
/// # Safety
///
/// `ptr` must be readable for `count` records when `count` is nonzero.
unsafe fn slice<'a, T>(ptr: *mut T, count: c_uint) -> &'a [T] {
    if count == 0 {
        return &[];
    }
    // SAFETY: the caller promises the readable run.
    unsafe { core::slice::from_raw_parts(ptr, count as usize) }
}

/// `memory_object_proxy_init()` of vm/memory_object_proxy.c.
#[unsafe(no_mangle)]
pub extern "C" fn memory_object_proxy_init() {
    memory_object_proxy::init();
}

/// `memory_object_proxy_notify()` of vm/memory_object_proxy.c.
///
/// # Safety
///
/// `msg` must point at a readable `mach_msg_header_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_proxy_notify(
    msg: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { memory_object_proxy::notify(msg.cast()) })
}

/// `memory_object_create_proxy()` of vm/memory_object_proxy.c.
///
/// # Safety
///
/// `object`, `offset`, `start` and `len` must be readable for their own
/// counts, and `proxy` must be writable storage for one port, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_create_proxy(
    task: *mut c_void,
    max_protection: VmProt,
    object: *mut *mut c_void,
    object_count: c_uint,
    offset: *mut VmOffset,
    offset_count: c_uint,
    start: *mut VmOffset,
    start_count: c_uint,
    len: *mut VmSize,
    len_count: c_uint,
    proxy: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises each array readable for its count.
    let result = unsafe {
        memory_object_proxy::create_proxy(
            IpcSpace::new(task),
            max_protection,
            slice(object, object_count),
            slice(offset, offset_count),
            slice(start, start_count),
            slice(len, len_count),
        )
    };

    match result {
        Ok(port) => {
            // SAFETY: the caller promises the writable out-pointer; the C
            // writes it only on success.
            unsafe { proxy.write(port.as_ptr()) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `memory_object_proxy_lookup()` of vm/memory_object_proxy.c.
///
/// # Safety
///
/// `port` must be `IP_NULL`, `IP_DEAD` or a live port the caller holds a
/// reference to, and all four out-pointers must be writable storage, written
/// only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_proxy_lookup(
    port: *mut c_void,
    object: *mut *mut c_void,
    max_protection: *mut VmProt,
    start: *mut VmOffset,
    len: *mut VmOffset,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { memory_object_proxy::lookup(port) } {
        Ok(target) => {
            // SAFETY: the caller promises the four writable out-pointers; the
            // C writes them only on success.
            unsafe {
                object.write(target.object);
                max_protection.write(target.max_protection);
                start.write(target.start);
                len.write(target.len);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}
