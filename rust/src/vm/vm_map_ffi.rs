// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the Rust VM map, one adapter per symbol
//! `vm/vm_map.c` used to define and `vm/vm_map.h` declares.
//!
//! This is the only place in the port that speaks C: pointers,
//! out-parameters and `kern_return_t` stop here, and the native core
//! in `vm_map.rs` takes over behind them.

use crate::arch::types::{VmOffset, VmSize};
use crate::vm::error::{KERN_SUCCESS, kern_return};
use crate::vm::types::{Pmap, VmObject, VmProt};
use crate::vm::vm_map::{VmMap, VmMapEntry, VmMapVersion};
use core::ffi::{c_int, c_uint};
use core::ptr::{self, NonNull};

/// Lock a map for writing.  `vm_map_lock()` in C.
///
/// # Safety
///
/// `map` must point at a valid, initialized VM map, and the caller
/// must unlock it with `vm_map_unlock()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_lock(map: *mut VmMap) {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    VmMap::lock(map);
}

/// Unlock a map locked by `vm_map_lock()`.  `vm_map_unlock()` in C.
///
/// # Safety
///
/// `map` must be a valid map that the caller holds the write lock on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_unlock(map: *mut VmMap) {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    VmMap::unlock(map);
}

/// Copy the VM limits from `src` to `dst`.  `vm_map_copy_limits()` in
/// C.
///
/// # Safety
///
/// Both arguments must point at valid maps, and the caller must hold
/// the source map's lock as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_limits(dst: *mut VmMap, src: *mut VmMap) {
    // SAFETY: the caller promises both maps are valid and distinct.
    let (dst, src) =
        unsafe { (NonNull::new_unchecked(dst), NonNull::new_unchecked(src)) };
    VmMap::copy_limits(dst, src);
}

/// Validate a lookup against the map timestamp.
/// `vm_map_verify()` in C.
///
/// # Safety
///
/// Both arguments must be valid; on a `true` result the caller must
/// release the read lock with `vm_map_verify_done()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_verify(
    map: *mut VmMap,
    version: *mut VmMapVersion,
) -> c_int {
    // SAFETY: the caller promises both pointers are valid.
    let (map, version) = unsafe { (NonNull::new_unchecked(map), &*version) };
    c_int::from(VmMap::verify(map, version))
}

/// Find the entry containing (or immediately preceding) `address`.
/// `vm_map_lookup_entry()` in C.
///
/// # Safety
///
/// `map` must be valid and locked for read or write, and `entry` must
/// point at writable storage for one entry pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_lookup_entry(
    map: *mut VmMap,
    address: VmOffset,
    entry: *mut *mut VmMapEntry,
) -> c_int {
    // SAFETY: the caller promises a valid map, held stable by its
    // lock, and a writable out-pointer.
    let (found, found_entry) = unsafe { (*map).lookup_entry(address) };
    // SAFETY: as above.
    unsafe { entry.write(found_entry.as_ptr()) };
    c_int::from(found)
}

/// Allocate a range and an entry for it.  `vm_map_find_entry()` in C.
///
/// # Safety
///
/// `map` must be a valid map locked for writing, and both out-pointers
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_find_entry(
    map: *mut VmMap,
    address: *mut VmOffset,
    size: VmSize,
    mask: VmOffset,
    object: *mut VmObject,
    o_entry: *mut *mut VmMapEntry,
    protection: VmProt,
    max_protection: VmProt,
) -> c_int {
    // SAFETY: the caller promises a valid, write-locked map.
    let map = unsafe { &mut *map };
    match VmMap::find_entry(
        map,
        size,
        mask,
        object,
        protection,
        max_protection,
    ) {
        Ok((start, entry)) => {
            // SAFETY: the caller promises writable out-pointers.
            unsafe {
                address.write(start);
                o_entry.write(entry.as_ptr());
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Add a reference to a map, if it is not null.
/// `vm_map_reference()` in C.
///
/// # Safety
///
/// A non-null `map` must point at a valid map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_reference(map: *mut VmMap) {
    if let Some(map) = NonNull::new(map) {
        VmMap::reference(map);
    }
}

/// Drop a reference to a map, if it is not null, destroying the map
/// when the last one goes.  `vm_map_deallocate()` in C.
///
/// # Safety
///
/// A non-null `map` must point at a valid map, and the caller must
/// hold a reference to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_deallocate(map: *mut VmMap) {
    if let Some(map) = NonNull::new(map) {
        VmMap::deallocate(map);
    }
}

/// Initialize an empty map in caller storage.  `vm_map_setup()` in C.
///
/// # Safety
///
/// `map` must point at writable storage for a `struct vm_map`, and
/// `pmap` at a valid physical map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_setup(
    map: *mut VmMap,
    pmap: *mut Pmap,
    min: VmOffset,
    max: VmOffset,
) {
    // SAFETY: the caller promises a valid map and pmap.
    VmMap::setup(unsafe { &mut *map }, pmap, min, max);
}

/// Allocate and initialize an empty map, or return null.
/// `vm_map_create()` in C.
///
/// # Safety
///
/// `pmap` must be a valid physical map, and `vm_map_init()` must have
/// run so the map cache exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_create(
    pmap: *mut Pmap,
    min: VmOffset,
    max: VmOffset,
) -> *mut VmMap {
    VmMap::create(pmap, min, max).map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// Apply a machine attribute to the map's pmap.
/// `vm_map_machine_attribute()` in C.
///
/// # Safety
///
/// `map` must be a valid map and `value` a valid attribute value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_machine_attribute(
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    attribute: c_uint,
    value: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises a valid, non-null map.
    let map = unsafe { NonNull::new_unchecked(map) };
    kern_return(VmMap::machine_attribute(
        map, address, size, attribute, value,
    ))
}

/// Synchronize a map region out to its memory manager.
/// `vm_map_msync()` in C.
///
/// # Safety
///
/// A non-null `map` must point at a valid map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_msync(
    map: *mut VmMap,
    address: VmOffset,
    size: VmSize,
    sync_flags: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid map when non-null.
    let map = NonNull::new(map);
    kern_return(VmMap::msync(map, address, size, sync_flags))
}
