// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the Rust VM map, one adapter per symbol
//! `vm/vm_map.c` used to define and `vm/vm_map.h` declares.
//!
//! This is the only place in the port that speaks C: pointers,
//! out-parameters and `kern_return_t` stop here, and the native core
//! in `vm_map.rs` takes over behind them.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::{vm_map_glue_task_map, vm_map_glue_task_space};
use crate::ipc::{IpcPort, IpcSpace};
use crate::vm::error::{KERN_INVALID_ARGUMENT, KERN_SUCCESS, kern_return};
use crate::vm::types::{Pmap, VmInherit, VmObject, VmProt};
use crate::vm::vm_map::{
    EnterRequest, VmMap, VmMapCopy, VmMapCopyContFn, VmMapCopyinArgs,
    VmMapEntry, VmMapHeader, VmMapVersion,
};
use core::ffi::{c_int, c_uint, c_void};
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

/// Find the object, offset and protection backing a virtual address.
/// `vm_map_lookup()` in C.
///
/// # Safety
///
/// `var_map` must point at a valid map pointer and every out-pointer
/// at writable storage.  On success the map is left read-locked when
/// `keep_map_locked` is set and unlocked otherwise, and the returned
/// object is locked; the caller releases both.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_lookup(
    var_map: *mut *mut VmMap,
    vaddr: VmOffset,
    fault_type: VmProt,
    keep_map_locked: c_int,
    out_version: *mut VmMapVersion,
    object: *mut *mut VmObject,
    offset: *mut VmOffset,
    out_prot: *mut VmProt,
    wired: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises a valid map pointer.
    let mut map = unsafe { NonNull::new_unchecked(*var_map) };
    match VmMap::lookup(&mut map, vaddr, fault_type, keep_map_locked != 0) {
        Ok(result) => {
            // SAFETY: the caller promises writable out-pointers.  The
            // map pointer is updated even on a submap descent, which
            // is what the C leaves behind.
            unsafe {
                *var_map = map.as_ptr();
                (*out_version).main_timestamp = result.timestamp;
                object.write(result.object);
                offset.write(result.offset);
                out_prot.write(result.protection);
                wired.write(c_int::from(result.wired));
            }
            KERN_SUCCESS
        }
        Err(error) => {
            // SAFETY: the caller promises a valid map pointer.
            unsafe { *var_map = map.as_ptr() };
            error.as_kern_return()
        }
    }
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

/// Enter a mapping into a map.  `vm_map_enter()` in C.
///
/// # Safety
///
/// `map` must be a valid map and `address` a writable slot the caller
/// owns; `object`, when non-null, must be a valid object the caller
/// holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_enter(
    map: *mut VmMap,
    address: *mut VmOffset,
    size: VmSize,
    mask: VmOffset,
    anywhere: c_int,
    object: *mut VmObject,
    offset: VmOffset,
    needs_copy: c_int,
    cur_protection: VmProt,
    max_protection: VmProt,
    inheritance: VmInherit,
) -> c_int {
    // SAFETY: the caller promises a valid map and address slot.
    kern_return(unsafe {
        (*map).enter(EnterRequest {
            address: &mut *address,
            size,
            mask,
            anywhere: anywhere != 0,
            object,
            offset,
            needs_copy: needs_copy != 0,
            cur_protection,
            max_protection,
            inheritance,
        })
    })
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

/// Create a map holding the same regions as `old_map`, obeying each
/// region's inheritance.  `vm_map_fork()` in C.
///
/// # Safety
///
/// `old_map` must be a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_fork(old_map: *mut VmMap) -> *mut VmMap {
    // SAFETY: the caller promises a valid map.
    let old_map = unsafe { NonNull::new_unchecked(old_map) };
    VmMap::fork(old_map).map_or(ptr::null_mut(), NonNull::as_ptr)
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

/// Split an entry at the start of a range.  `_vm_map_clip_start()` in
/// C.
///
/// # Safety
///
/// `map_header` must belong to a locked map or copy, `entry` must be
/// a live entry of it, and `start` must lie inside the entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vm_map_clip_start(
    map_header: *mut VmMapHeader,
    entry: *mut VmMapEntry,
    start: VmOffset,
    link_gap: c_int,
) {
    // SAFETY: the caller promises a live entry and header.
    unsafe {
        (*map_header).clip_start(
            NonNull::new_unchecked(entry),
            start,
            link_gap != 0,
        )
    };
}

/// Split an entry at the end of a range.  `_vm_map_clip_end()` in C.
///
/// # Safety
///
/// Same contract as `_vm_map_clip_start()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vm_map_clip_end(
    map_header: *mut VmMapHeader,
    entry: *mut VmMapEntry,
    end: VmOffset,
    link_gap: c_int,
) {
    // SAFETY: the caller promises a live entry and header.
    unsafe {
        (*map_header).clip_end(
            NonNull::new_unchecked(entry),
            end,
            link_gap != 0,
        )
    };
}

/// Deallocate one entry from a locked map.  `vm_map_entry_delete()` in
/// C.
///
/// # Safety
///
/// `map` must be valid and write-locked, and `entry` a live entry of
/// it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_entry_delete(
    map: *mut VmMap,
    entry: *mut VmMapEntry,
) {
    // SAFETY: the caller promises a locked map and a linked entry.
    unsafe { (*map).entry_delete(NonNull::new_unchecked(entry)) };
}

/// Deallocate a range from a map.  `vm_map_delete()` in C.
///
/// # Safety
///
/// `map` must be valid, locked unless its refcount is zero, and the
/// range must be within its bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_delete(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
) -> c_int {
    // SAFETY: the caller promises a valid map in the stated state.
    kern_return(unsafe { (*map).delete(start, end) })
}

/// Remove a range from a map, clamping it and taking the lock.
/// `vm_map_remove()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_remove(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe { (*map).remove(start, end) })
}

/// Mark a range as handled by a subordinate map.
/// `vm_map_submap()` in C.
///
/// # Safety
///
/// `map` and a non-null `submap` must be valid maps, and the caller
/// must not hold the map's lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_submap(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
    submap: *mut VmMap,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe { (*map).submap(start, end, submap) })
}

/// Force the resident pages of an object into a map's pmap, stopping
/// at the first page that is not present.  `vm_map_pmap_enter()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map and `object` a valid object
/// with `[addr, end_addr)` mapped at `offset`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_pmap_enter(
    map: *mut VmMap,
    addr: VmOffset,
    end_addr: VmOffset,
    object: *mut VmObject,
    offset: VmOffset,
    protection: VmProt,
) {
    // SAFETY: the caller promises a valid map and object.
    unsafe { (*map).pmap_enter(addr, end_addr, object, offset, protection) };
}

/// Try to coalesce an entry with its predecessor.
/// `vm_map_coalesce_entry()` in C.
///
/// # Safety
///
/// `map` must be valid and write-locked, and `entry` a live entry of
/// it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_coalesce_entry(
    map: *mut VmMap,
    entry: *mut VmMapEntry,
) -> c_int {
    // SAFETY: the caller promises a locked map and a live entry.
    let coalesced =
        unsafe { (*map).coalesce_entry(NonNull::new_unchecked(entry)) };
    c_int::from(coalesced)
}

/// Set the protection of a range.  `vm_map_protect()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_protect(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
    new_prot: VmProt,
    set_max: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe { (*map).protect(start, end, new_prot, set_max != 0) })
}

/// Set the inheritance of a range.  `vm_map_inherit()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_inherit(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
    new_inheritance: VmInherit,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe { (*map).inherit(start, end, new_inheritance) })
}

/// Set the pageability of a range.  `vm_map_pageable()` in C.
///
/// # Safety
///
/// `map` must be valid and, when `lock_map` is false, locked by the
/// caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_pageable(
    map: *mut VmMap,
    start: VmOffset,
    end: VmOffset,
    access_type: VmProt,
    lock_map: c_int,
    check_range: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid map in the stated state.
    kern_return(unsafe {
        (*map).pageable(
            start,
            end,
            access_type,
            lock_map != 0,
            check_range != 0,
        )
    })
}

/// Wire a whole map, now and/or in the future.
/// `vm_map_pageable_all()` in C.
///
/// # Safety
///
/// `map` must be a valid, unlocked map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_pageable_all(
    map: *mut VmMap,
    flags: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    kern_return(unsafe { (*map).pageable_all(flags) })
}

/// Place a copy into newly-allocated space in a map.
/// `vm_map_copyout()` in C.
///
/// The address is written only on success, where the C wrote it just
/// before the last failure point of its entry-list path; every
/// in-tree caller ignores the slot on failure.
///
/// # Safety
///
/// `dst_map` must be a valid, unlocked map and `dst_addr` writable
/// storage for one address.  A non-null `copy` must be a live copy
/// the caller owns; the call consumes it on success, and on failure
/// the caller still owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copyout(
    dst_map: *mut VmMap,
    dst_addr: *mut VmOffset,
    copy: *mut VmMapCopy,
) -> c_int {
    let Some(copy) = NonNull::new(copy) else {
        // The C treats a null copy as a success with address 0.
        // SAFETY: the caller promises a writable out-pointer.
        unsafe { dst_addr.write(0) };
        return KERN_SUCCESS;
    };
    // SAFETY: the caller promises a valid, unlocked map and a live
    // copy.
    let map = unsafe { &mut *dst_map };
    match unsafe { map.copyout(copy) } {
        Ok(address) => {
            // SAFETY: as above; the C writes the address on success.
            unsafe { dst_addr.write(address) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Copy a region of a map into a new copy object.
/// `vm_map_copyin()` in C.
///
/// A zero-length copy is answered here with a null copy object, as
/// the C does; the core does not model it.
///
/// # Safety
///
/// `src_map` must point at a valid, unlocked map and `copy_result` at
/// writable storage for one copy pointer.  `vm_map_init()` must have
/// initialized the copy cache, and the source region must be
/// readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copyin(
    src_map: *mut VmMap,
    src_addr: VmOffset,
    len: VmSize,
    src_destroy: c_int,
    copy_result: *mut *mut VmMapCopy,
) -> c_int {
    if len == 0 {
        // SAFETY: the caller promises writable storage.
        unsafe { copy_result.write(ptr::null_mut()) };
        return KERN_SUCCESS;
    }
    // SAFETY: the caller promises a valid, unlocked map.
    let map = unsafe { &mut *src_map };
    match map.copyin(src_addr, len, src_destroy != 0) {
        Ok(copy) => {
            // SAFETY: the caller promises writable storage.
            unsafe { copy_result.write(copy.as_ptr()) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Copy a region of a map into a page-list copy object.
/// `vm_map_copyin_page_list()` in C.
///
/// A zero-length copy is answered here with a null copy object, as
/// the C does.
///
/// # Safety
///
/// `src_map` must point at a valid, unlocked map and `copy_result` at
/// writable storage for one copy pointer.  `vm_map_init()` must have
/// initialized the copy cache, and the source region must be
/// readable.  With `src_destroy`, the caller must own the region.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copyin_page_list(
    src_map: *mut VmMap,
    src_addr: VmOffset,
    len: VmSize,
    src_destroy: c_int,
    steal_pages: c_int,
    copy_result: *mut *mut VmMapCopy,
    is_cont: c_int,
) -> c_int {
    // SAFETY: the caller promises a valid, unlocked map.
    let map = unsafe { &mut *src_map };
    match map.copyin_page_list(
        src_addr,
        len,
        src_destroy != 0,
        steal_pages != 0,
        is_cont != 0,
    ) {
        Ok(copy) => {
            // SAFETY: the caller promises writable storage; a
            // zero-length copy leaves it null.
            unsafe {
                copy_result
                    .write(copy.map_or(ptr::null_mut(), NonNull::as_ptr))
            };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Create a copy object around a donated object reference.
/// `vm_map_copyin_object()` in C.
///
/// # Safety
///
/// `object` must be a live object whose reference the caller donates,
/// and `copy_result` writable storage for one copy pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copyin_object(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    copy_result: *mut *mut VmMapCopy,
) -> c_int {
    // SAFETY: the caller donates the reference and promises writable
    // storage; `vm_map_init()` initialized the copy cache.
    unsafe {
        copy_result
            .write(VmMapCopy::copyin_object(object, offset, size).as_ptr());
    }
    KERN_SUCCESS
}

/// Place a page-list copy into newly-allocated space in a map.
/// `vm_map_copyout_page_list()` in C.
///
/// # Safety
///
/// `dst_map` must be a valid, unlocked map and `dst_addr` writable
/// storage for one address.  `copy` must be a live page-list copy the
/// caller owns; the call consumes the original copy on success, and
/// on failure the caller still owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copyout_page_list(
    dst_map: *mut VmMap,
    dst_addr: *mut VmOffset,
    copy: *mut VmMapCopy,
) -> c_int {
    // SAFETY: the caller promises a valid map and a live copy.
    let map = unsafe { &mut *dst_map };
    // SAFETY: the caller promises a live, non-null copy.
    let copy = unsafe { NonNull::new_unchecked(copy) };
    match unsafe { map.copyout_page_list(copy) } {
        Ok(address) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe { dst_addr.write(address) };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Get rid of the pages of a page-list copy.
/// `vm_map_copy_page_discard()` in C.
///
/// # Safety
///
/// `copy` must point at a live page-list copy the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_page_discard(copy: *mut VmMapCopy) {
    // SAFETY: the caller promises a live, non-null page-list copy.
    unsafe { VmMapCopy::page_discard(NonNull::new_unchecked(copy)) };
}

/// Dispose of a map copy object.  `vm_map_copy_discard()` in C.
///
/// # Safety
///
/// A non-null `copy` must be a live copy the caller owns; the call
/// frees it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_discard(copy: *mut VmMapCopy) {
    if let Some(copy) = NonNull::new(copy) {
        // SAFETY: the caller owns the live copy.
        unsafe { VmMapCopy::discard(copy) };
    }
}

/// Move the contents of a copy into a fresh copy object, leaving the
/// original empty.  `vm_map_copy_copy()` in C.
///
/// # Safety
///
/// A non-null `copy` must be a live copy the caller owns; on return
/// the caller owns the empty original and the new copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_copy(
    copy: *mut VmMapCopy,
) -> *mut VmMapCopy {
    let Some(copy) = NonNull::new(copy) else {
        return ptr::null_mut();
    };
    // SAFETY: the caller owns the live copy.
    unsafe { VmMapCopy::duplicate(copy).as_ptr() }
}

/// Overwrite previously-mapped memory with the contents of a copy.
/// `vm_map_copy_overwrite()` in C.
///
/// # Safety
///
/// A non-null `copy` must be a live `ENTRY_LIST` copy the caller
/// owns; on success it is consumed, and on failure the caller still
/// owns it, possibly with some entries already consumed (the C
/// `vm_copy` discards it on error).  `dst_map` must be a valid,
/// unlocked map, and the destination must lie inside it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_overwrite(
    dst_map: *mut VmMap,
    dst_addr: VmOffset,
    copy: *mut VmMapCopy,
    _interruptible: c_int,
) -> c_int {
    let Some(copy) = NonNull::new(copy) else {
        // The C treats a null copy as nothing to do.
        return KERN_SUCCESS;
    };
    // The C overwrites `interruptible` with FALSE before its first
    // use, so the caller's value never reaches the logic and the
    // adapter drops it.
    // SAFETY: the caller promises a valid, unlocked map and a live
    // copy.
    kern_return(unsafe { (*dst_map).copy_overwrite(dst_addr, copy) })
}

/// Whether `cont` is `vm_map_copy_discard_cont()` below, which
/// `vm_map_copy_discard()` recognizes and follows iteratively instead
/// of recursing once per link of a page-list chain.  The C compares
/// the function addresses.
pub(crate) fn is_discard_cont(cont: VmMapCopyContFn) -> bool {
    ptr::fn_addr_eq(cont, vm_map_copy_discard_cont as VmMapCopyContFn)
}

/// Discard a page-list copy from a continuation.
/// `vm_map_copy_discard_cont()` in C.
///
/// # Safety
///
/// `cont_args` must be the copy a continuation chain names, and
/// `copy_result` must be null or point at writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_map_copy_discard_cont(
    cont_args: *mut VmMapCopyinArgs,
    copy_result: *mut *mut VmMapCopy,
) -> c_int {
    if let Some(copy) = NonNull::new(cont_args.cast::<VmMapCopy>()) {
        // SAFETY: the continuation contract makes its argument the
        // live copy to discard.
        unsafe { VmMapCopy::discard(copy) };
    }
    if let Some(copy_result) = NonNull::new(copy_result) {
        // SAFETY: the caller promises writable storage.
        unsafe { copy_result.as_ptr().write(ptr::null_mut()) };
    }
    KERN_SUCCESS
}

/// Describe the region `address` falls in, or the first one above it.
/// `vm_region()` in C.
///
/// # Safety
///
/// `map` must be a valid map or null, every out-pointer must be
/// writable, and `address` must point at readable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_region(
    map: *mut VmMap,
    address: *mut VmOffset,
    size: *mut VmSize,
    protection: *mut VmProt,
    max_protection: *mut VmProt,
    inheritance: *mut VmInherit,
    is_shared: *mut c_int,
    object_name: *mut *mut c_void,
    offset_in_object: *mut VmOffset,
) -> c_int {
    let Some(map) = NonNull::new(map) else {
        return KERN_INVALID_ARGUMENT;
    };

    // SAFETY: the caller promises a readable address slot.
    let start = unsafe { *address };
    // SAFETY: the caller promises a valid map.
    match unsafe { map.as_ref() }.region(start) {
        Ok(region) => {
            // SAFETY: the caller promises writable out-pointers.
            unsafe {
                address.write(region.address);
                size.write(region.size);
                protection.write(region.protection);
                max_protection.write(region.max_protection);
                inheritance.write(region.inheritance);
                is_shared.write(c_int::from(region.is_shared));
                object_name.write(
                    region
                        .object_name
                        .map_or(ptr::null_mut(), IpcPort::as_ptr),
                );
                offset_in_object.write(region.offset);
            }
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}

/// Create a proxy to the memory region `address` falls in.
/// `vm_region_create_proxy()` in C.
///
/// # Safety
///
/// `task` must be a valid task or null, the task's map must be valid,
/// and `port` must be writable storage for one port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_region_create_proxy(
    task: *mut c_void,
    address: VmOffset,
    max_protection: VmProt,
    len: VmSize,
    port: *mut *mut c_void,
) -> c_int {
    if task.is_null() {
        return KERN_INVALID_ARGUMENT;
    }

    // SAFETY: the caller promises a valid task; the two shims read
    // the map and the IPC space the C body reads from it.  The map
    // comes back as an opaque handle until `VmMap` is FFI-safe.
    let (map, space) = unsafe {
        (
            NonNull::new(vm_map_glue_task_map(task).cast::<VmMap>()),
            IpcSpace::new(vm_map_glue_task_space(task)),
        )
    };
    // A live task always has a map; keep the C's argument check for
    // the impossible null.
    let Some(map) = map else {
        return KERN_INVALID_ARGUMENT;
    };

    // SAFETY: the caller promises a valid map.
    match unsafe { map.as_ref() }.region_create_proxy(
        space,
        address,
        max_protection,
        len,
    ) {
        Ok(proxy) => {
            // SAFETY: the caller promises a writable out-pointer.
            unsafe {
                port.write(proxy.map_or(ptr::null_mut(), IpcPort::as_ptr))
            };
            KERN_SUCCESS
        }
        Err(error) => error.as_kern_return(),
    }
}
