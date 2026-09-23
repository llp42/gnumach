// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_user.c and vm/vm_user.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the user-exported VM calls, one adapter per
//! symbol `vm/vm_user.c` used to define and `vm/vm_user.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::vm::error::{
    KERN_INVALID_ARGUMENT, KERN_INVALID_TASK, KERN_SUCCESS, kern_return,
};
use crate::vm::types::{VmInherit, VmObject, VmProt};
use crate::vm::vm_map::{VmMap, VmMapCopy};
use crate::vm::vm_user;
use core::ffi::{c_int, c_uint};
use core::ptr::{NonNull, with_exposed_provenance_mut};

/// `vm_allocate()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid map, and `addr` at writable storage
/// for one address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_allocate(
    map: *mut VmMap,
    addr: *mut VmOffset,
    size: VmSize,
    anywhere: c_int,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map and a writable
    // address slot.
    kern_return(unsafe {
        vm_user::allocate(&mut *map.as_ptr(), &mut *addr, size, anywhere != 0)
    })
}

/// `vm_deallocate()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_deallocate(
    map: *mut VmMap,
    start: VmOffset,
    size: VmSize,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::deallocate(&mut *map.as_ptr(), start, size)
    })
}

/// `vm_inherit()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_inherit(
    map: *mut VmMap,
    start: VmOffset,
    size: VmSize,
    new_inheritance: VmInherit,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::inherit(&mut *map.as_ptr(), start, size, new_inheritance)
    })
}

/// `vm_protect()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_protect(
    map: *mut VmMap,
    start: VmOffset,
    size: VmSize,
    set_maximum: c_int,
    new_protection: VmProt,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::protect(
            &mut *map.as_ptr(),
            start,
            size,
            set_maximum != 0,
            new_protection,
        )
    })
}

/// `vm_machine_attribute()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.  `value` is not read
/// while the pmap attribute walk is a stub.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_machine_attribute(
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    _attribute: c_uint,
    _value: *mut c_int,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::machine_attribute(&*map.as_ptr(), address, size)
    })
}

/// `vm_read()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map, and both out-pointers
/// writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_read(
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    data: *mut VmOffset,
    data_size: *mut c_uint,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    match unsafe { vm_user::read(&mut *map.as_ptr(), address, size) } {
        Ok(copy) => {
            // SAFETY: the caller promises writable out-pointers.  The C
            // stores a `vm_size_t` through a `mach_msg_type_number_t *`, so
            // only the low 32 bits of `size` reach the reply.
            unsafe {
                data.write(
                    copy.map_or(0, |copy| copy.as_ptr().expose_provenance()),
                );
                data_size.write(size as c_uint);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `vm_write()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map, and `data` must be
/// null or the live copy `vm_read()` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_write(
    map: *mut VmMap,
    address: VmOffset,
    data: VmOffset,
    _size: c_uint,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    let copy = NonNull::new(with_exposed_provenance_mut::<VmMapCopy>(data));
    // SAFETY: the caller promises a valid, unlocked map and, when non-null,
    // a live copy.
    kern_return(unsafe { vm_user::write(&mut *map.as_ptr(), address, copy) })
}

/// `vm_copy()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_copy(
    map: *mut VmMap,
    source_address: VmOffset,
    size: VmSize,
    dest_address: VmOffset,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::copy(&mut *map.as_ptr(), source_address, size, dest_address)
    })
}

/// `vm_object_sync()` in C.
///
/// # Safety
///
/// `object` must be null or the live object the caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_sync(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    should_flush: c_int,
    should_return: c_int,
    _should_iosync: c_int,
) -> c_int {
    let Some(object) = NonNull::new(object) else {
        return KERN_INVALID_ARGUMENT;
    };
    kern_return(vm_user::object_sync(
        object,
        offset,
        size,
        should_flush != 0,
        should_return != 0,
    ))
}

/// `vm_msync()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_msync(
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    sync_flags: c_int,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe {
        vm_user::msync(&mut *map.as_ptr(), address, size, sync_flags)
    })
}

/// `vm_get_size_limit()` in C.
///
/// # Safety
///
/// `map` must be null or point at a valid map, and both out-pointers writable
/// storage for one size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_get_size_limit(
    map: *mut VmMap,
    current_limit: *mut VmSize,
    max_limit: *mut VmSize,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_TASK;
    };
    // SAFETY: the caller promises a valid map.
    let (current, max) = vm_user::get_size_limit(unsafe { &*map.as_ptr() });
    // SAFETY: the caller promises writable out-pointers.
    unsafe {
        current_limit.write(current);
        max_limit.write(max);
    }
    KERN_SUCCESS
}
