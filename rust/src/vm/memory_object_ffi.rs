// SPDX-License-Identifier: CMU-Mach
// Derived from vm/memory_object.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the external memory management interface, one
//! adapter per symbol `vm/memory_object.c` used to define and the MIG
//! `mach.server.h` prototypes declare.

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::host::Host;
use crate::vm::error::kern_return;
use crate::vm::memory_object::{self, Return};
use crate::vm::types::{VmObject, VmProt};
use core::ffi::{c_int, c_uint, c_void};

/// `memory_object_data_supply()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate;
/// `data` must name a live page-list copy of `data_cnt` bytes; `reply_to`
/// must be `IP_NULL` or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_data_supply(
    object: *mut VmObject,
    offset: VmOffset,
    data: VmOffset,
    data_cnt: c_uint,
    lock_value: c_int,
    precious: c_int,
    reply_to: *mut c_void,
    reply_to_type: c_uint,
) -> c_int {
    // SAFETY: the caller promises the live object, copy and reply port.
    kern_return(unsafe {
        memory_object::data_supply(
            object,
            &memory_object::SupplyRequest {
                offset,
                data,
                data_cnt,
                lock_value: VmProt::from_bits(lock_value),
                precious: precious != 0,
                reply_to,
                reply_to_type,
            },
        )
    })
}

/// `memory_object_data_error()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_data_error(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    _error_value: c_int,
) -> c_int {
    // SAFETY: the caller promises the live object.
    kern_return(unsafe { memory_object::data_error(object, offset, size) })
}

/// `memory_object_data_unavailable()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_data_unavailable(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller promises the live object.
    kern_return(unsafe {
        memory_object::data_unavailable(object, offset, size)
    })
}

/// `memory_object_lock_request()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate;
/// `reply_to` must be `IP_NULL` or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_lock_request(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    should_return: c_int,
    should_flush: c_int,
    lock_value: c_int,
    reply_to: *mut c_void,
    reply_to_type: c_uint,
) -> c_int {
    // SAFETY: the caller promises the live object and reply port.
    kern_return(unsafe {
        memory_object::lock_request(
            object,
            &memory_object::LockRequest {
                offset,
                size,
                should_return: Return::from_c(should_return),
                should_flush: should_flush != 0,
                prot: VmProt::from_bits(lock_value),
                reply_to,
                reply_to_type,
            },
        )
    })
}

/// `memory_object_change_attributes()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate;
/// `reply_to` must be `IP_NULL` or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_change_attributes(
    object: *mut VmObject,
    may_cache: c_int,
    copy_strategy: c_int,
    reply_to: *mut c_void,
    reply_to_type: c_uint,
) -> c_int {
    // SAFETY: the caller promises the live object and reply port.
    kern_return(unsafe {
        memory_object::change_attributes(
            object,
            may_cache != 0,
            copy_strategy,
            reply_to,
            reply_to_type,
        )
    })
}

/// `memory_object_ready()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_ready(
    object: *mut VmObject,
    may_cache: c_int,
    copy_strategy: c_int,
) -> c_int {
    // SAFETY: the caller promises the live object.
    kern_return(unsafe {
        memory_object::ready(object, may_cache != 0, copy_strategy)
    })
}

/// `memory_object_get_attributes()` in C.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate, and the
/// three out-pointers must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_get_attributes(
    object: *mut VmObject,
    object_ready: *mut c_int,
    may_cache: *mut c_int,
    copy_strategy: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises the live object.
    match unsafe { memory_object::get_attributes(object) } {
        Ok(attributes) => {
            // SAFETY: the caller promises the three writable out-pointers.
            unsafe {
                object_ready.write(c_int::from(attributes.ready));
                may_cache.write(c_int::from(attributes.may_cache));
                copy_strategy.write(attributes.copy_strategy);
            }
            0
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `vm_set_default_memory_manager()` in C.
///
/// # Safety
///
/// A non-null `host` must be a live host, and `default_manager` must be
/// writable for one port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_set_default_memory_manager(
    host: *mut Host,
    default_manager: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the live host and the writable slot.
    match unsafe { memory_object::default_manager::set(host, default_manager) }
    {
        Ok(()) => 0,
        Err(error) => error.as_kern_return(),
    }
}

/// `memory_manager_default_reference()` in C.
///
/// # Safety
///
/// The caller must not hold the default-manager lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_manager_default_reference() -> *mut c_void {
    // SAFETY: the caller promises the lock is not held.
    unsafe { memory_object::default_manager::reference() }.as_ptr()
}

/// `memory_manager_default_port()` in C.
///
/// # Safety
///
/// `port` must be `IP_NULL` or a live port; the caller must not hold the
/// default-manager lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_manager_default_port(
    port: *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the port contract.
    c_int::from(unsafe { memory_object::default_manager::port(port) })
}

/// `memory_manager_default_init()` in C.
///
/// # Safety
///
/// Must be called once during the VM bootstrap, before any other routine of
/// this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_manager_default_init() {
    memory_object::default_manager::init();
}
