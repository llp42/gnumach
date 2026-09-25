// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_user.c and vm/vm_user.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the user-exported VM calls, one adapter per
//! symbol `vm/vm_user.c` used to define and `vm/vm_user.h` declares.

use crate::arch::types::{RpcPhysAddr, VmOffset, VmSize};
use crate::kern::host::Host;
use crate::vm::error::{
    KERN_INVALID_ARGUMENT, KERN_INVALID_TASK, KERN_SUCCESS, kern_return,
};
use crate::vm::types::{VmInherit, VmObject, VmProt, VmStatistics};
use crate::vm::vm_map::{VmMap, VmMapCopy};
use crate::vm::vm_user::{self, VmCacheStatistics};
use core::ffi::{c_int, c_uint, c_void};
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

/// `vm_statistics()` of vm/vm_user.c.
///
/// # Safety
///
/// `map` must be null or a live map, and `stat` must be writable storage for
/// one statistics record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_statistics(
    map: *mut VmMap,
    stat: *mut VmStatistics,
) -> c_int {
    if map.is_null() {
        return KERN_INVALID_ARGUMENT;
    }
    // SAFETY: the caller promises the writable record.
    unsafe { stat.write(vm_user::statistics()) };
    KERN_SUCCESS
}

/// `vm_cache_statistics()` of vm/vm_user.c.
///
/// # Safety
///
/// `map` must be null or a live map, and `stats` must be writable storage
/// for one cache-statistics record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_cache_statistics(
    map: *mut VmMap,
    stats: *mut VmCacheStatistics,
) -> c_int {
    if map.is_null() {
        return KERN_INVALID_ARGUMENT;
    }
    // SAFETY: the caller promises the writable record.
    unsafe { stats.write(vm_user::cache_statistics()) };
    KERN_SUCCESS
}

/// `vm_map()` of vm/vm_user.c.
///
/// # Safety
///
/// `target_map` must be null or a live, unlocked map, `address` must point
/// at writable storage for one address, and `memory_object` must be
/// `IP_NULL`, `IP_DEAD` or a live port the caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map(
    target_map: *mut VmMap,
    address: *mut VmOffset,
    size: VmSize,
    mask: VmOffset,
    anywhere: c_int,
    memory_object: *mut c_void,
    offset: VmOffset,
    copy: c_int,
    cur_protection: VmProt,
    max_protection: VmProt,
    inheritance: VmInherit,
) -> c_int {
    let Some(target_map) = NonNull::new(target_map) else {
        return KERN_INVALID_ARGUMENT;
    };
    // SAFETY: the caller promises a live map and a writable address slot.
    kern_return(unsafe {
        vm_user::map(
            &mut *target_map.as_ptr(),
            vm_user::MapRequest {
                address: &mut *address,
                size,
                mask,
                anywhere: anywhere != 0,
                memory_object,
                offset,
                copy: copy != 0,
                cur_protection,
                max_protection,
                inheritance,
            },
        )
    })
}

/// `vm_wire()` of vm/vm_user.c.
///
/// # Safety
///
/// `port` must be `IP_NULL`, `IP_DEAD` or a live port; `map` must be null or
/// a live, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_wire(
    port: *mut c_void,
    map: *mut VmMap,
    start: VmOffset,
    size: VmSize,
    access: VmProt,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { vm_user::wire(port, map, start, size, access) })
}

/// `vm_wire_all()` of vm/vm_user.c.
///
/// # Safety
///
/// `port` must be `IP_NULL`, `IP_DEAD` or a live port; `map` must be null or
/// a live, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_wire_all(
    port: *mut c_void,
    map: *mut VmMap,
    flags: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe { vm_user::wire_all(port, map, flags) })
}

/// `vm_allocate_contiguous()` of vm/vm_user.c.
///
/// # Safety
///
/// `host_priv` must be null or the live host pointer the generated server
/// converted the request port into; `map` must be null or a live, unlocked
/// map; `vaddr` and `paddr` must be writable storage, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_allocate_contiguous(
    host_priv: *mut Host,
    map: *mut VmMap,
    vaddr: *mut VmOffset,
    paddr: *mut RpcPhysAddr,
    size: VmSize,
    pmin: RpcPhysAddr,
    pmax: RpcPhysAddr,
    palign: RpcPhysAddr,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        vm_user::allocate_contiguous(
            NonNull::new(host_priv),
            map,
            size,
            pmin,
            pmax,
            palign,
        )
    } {
        Ok((address, physical)) => {
            // SAFETY: the caller promises both out-pointers writable; the C
            // writes them only on success.
            unsafe {
                vaddr.write(address);
                paddr.write(physical);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// `vm_pages_phys()` of vm/vm_user.c.
///
/// # Safety
///
/// `host` must be null or the live host pointer the generated server
/// converted the request port into; `map` must be null or a live, unlocked
/// map; `pagespp` and `countp` must be writable storage, and the caller
/// permits an allocation and a kernel-map copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_pages_phys(
    host: *mut Host,
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    pagespp: *mut *mut RpcPhysAddr,
    countp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe {
        vm_user::pages_phys(
            NonNull::new(host),
            map,
            address,
            size,
            pagespp,
            countp,
        )
    } {
        Ok(()) => KERN_SUCCESS,
        Err(error) => error.as_kern_return(),
    }
}

/// `vm_set_size_limit()` of vm/vm_user.c.
///
/// # Safety
///
/// `host_port` must be `IP_NULL`, `IP_DEAD` or a live port, and `map` must be
/// null or a live, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_set_size_limit(
    host_port: *mut c_void,
    map: *mut VmMap,
    current_limit: VmSize,
    max_limit: VmSize,
) -> c_int {
    // SAFETY: the caller's contract.
    kern_return(unsafe {
        vm_user::set_size_limit(host_port, map, current_limit, max_limit)
    })
}
