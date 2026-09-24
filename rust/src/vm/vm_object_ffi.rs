// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_object.c and vm/vm_object.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the virtual-memory object module, one adapter per
//! symbol `vm/vm_object.c` used to define and `vm/vm_object.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::vm::error::{KERN_SUCCESS, MACH_SEND_INTERRUPTED, kern_return};
use crate::vm::types::{Pmap, VmObject};
use crate::vm::vm_object::{self, StrategicResult};
use core::ffi::{c_int, c_void};
use core::ptr::{NonNull, null_mut};

/// `memory_object_release()` in C.
///
/// # Safety
///
/// `pager` must be null or a live memory-object port; the three ports are
/// consumed by the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_release(
    pager: *mut c_void,
    pager_request: *mut c_void,
    pager_name: *mut c_void,
) {
    // SAFETY: the caller promises the ports.
    unsafe {
        vm_object::memory_object_release(pager, pager_request, pager_name)
    };
}

/// `memory_object_destroy()` in C.
///
/// # Safety
///
/// `object` must be null or a live object the caller holds a reference to;
/// `_reason` is unused, as in the C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_object_destroy(
    object: *mut VmObject,
    _reason: c_int,
) -> c_int {
    // SAFETY: the caller promises the object and the reference.
    unsafe { vm_object::memory_object_destroy(object) };
    KERN_SUCCESS
}

/// `vm_object_bootstrap()` in C.
///
/// # Safety
///
/// Must run once in the bootstrap sequence, after the slab and IPC packages
/// are up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_bootstrap() {
    vm_object::bootstrap();
}

/// `vm_object_init()` in C.
///
/// # Safety
///
/// Must run after [`vm_object_bootstrap()`], as the C's sequence did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_init() {
    vm_object::init();
}

/// `vm_object_collect()` in C.
///
/// # Safety
///
/// The object must be live and locked, and the caller must own a reference
/// to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_collect(object: *mut VmObject) {
    // SAFETY: the caller promises the locked object and the reference.
    unsafe { vm_object::collect(object) };
}

/// `vm_object_terminate()` in C.
///
/// # Safety
///
/// The object and cache locks must be held, and the object must have no
/// references, as the C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_terminate(object: *mut VmObject) {
    // SAFETY: the caller promises both locks and the dead object.
    unsafe { vm_object::terminate(object) };
}

/// `vm_object_allocate()` in C.
///
/// # Safety
///
/// The slab and IPC packages must be initialized, as the C assumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_allocate(size: VmSize) -> *mut VmObject {
    // SAFETY: the caller promises the initialized packages.
    unsafe { vm_object::allocate(size) }.map_or(null_mut(), NonNull::as_ptr)
}

/// `vm_object_reference()` in C.
///
/// # Safety
///
/// `object` must be null or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_reference(object: *mut VmObject) {
    // SAFETY: the caller promises the object.
    unsafe { vm_object::reference(object) };
}

/// `vm_object_deallocate()` in C.
///
/// # Safety
///
/// `object` must be null or a live object the caller holds a reference to,
/// and no lock of the caller's may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_deallocate(object: *mut VmObject) {
    // SAFETY: the caller promises the object, its reference and the unlocked
    // context.
    unsafe { vm_object::deallocate(object) };
}

/// `vm_object_pmap_protect()` in C.
///
/// # Safety
///
/// `object` must be null or a live object; `pmap` must be null or a live
/// physical map over the range the C's caller checked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_pmap_protect(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    pmap: *mut Pmap,
    pmap_start: VmOffset,
    prot: c_int,
) {
    // SAFETY: the caller promises the object, the map and the range.
    unsafe {
        vm_object::pmap_protect(
            object,
            offset,
            size,
            pmap,
            pmap_start,
            crate::vm::types::VmProt::from_bits(prot),
        )
    };
}

/// `vm_object_pmap_remove()` in C.
///
/// # Safety
///
/// `object` must be null or a live object, and the caller must hold no lock
/// of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_pmap_remove(
    object: *mut VmObject,
    start: VmOffset,
    end: VmOffset,
) {
    // SAFETY: the caller promises the object and the unlocked context.
    unsafe { vm_object::pmap_remove(object, start, end) };
}

/// `vm_object_page_remove()` in C.
///
/// # Safety
///
/// The object must be live and locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_page_remove(
    object: *mut VmObject,
    start: VmOffset,
    end: VmOffset,
) {
    // SAFETY: the caller promises the locked object.
    unsafe { vm_object::page_remove(object, start, end) };
}

/// `vm_object_shadow()` in C.
///
/// # Safety
///
/// `object` and `offset` must be writable slots for a live-in object and an
/// offset; the object's reference moves to the shadow created here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_shadow(
    object: *mut *mut VmObject,
    offset: *mut VmOffset,
    length: VmSize,
) {
    // SAFETY: the caller promises both slots writable.
    unsafe {
        let source = *object;
        let shadow = vm_object::shadow(source, *offset, length);
        object.write(shadow.as_ptr());
        offset.write(0);
    }
}

/// `vm_object_collapse()` in C.
///
/// # Safety
///
/// The object must be live and locked, and the caller must own a reference
/// to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_collapse(object: *mut VmObject) {
    // SAFETY: the caller promises the locked object and the reference.
    unsafe { vm_object::collapse(object) };
}

/// `vm_object_lookup()` in C.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_lookup(port: *mut c_void) -> *mut VmObject {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::lookup(port) }.map_or(null_mut(), NonNull::as_ptr)
}

/// `vm_object_lookup_name()` in C.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_lookup_name(
    port: *mut c_void,
) -> *mut VmObject {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::lookup_name(port) }.map_or(null_mut(), NonNull::as_ptr)
}

/// `vm_object_name()` in C.
///
/// # Safety
///
/// `object` must be null or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_name(object: *mut VmObject) -> *mut c_void {
    // SAFETY: the caller promises the object.
    unsafe { vm_object::name(object) }
}

/// `vm_object_remove()` in C.
///
/// # Safety
///
/// The cache lock must be held and the object must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_remove(object: *mut VmObject) {
    // SAFETY: the caller promises the cache lock and the live object.
    unsafe { vm_object::remove(object) };
}

/// `vm_object_copy_temporary()` in C.
///
/// # Safety
///
/// `object` and `offset` must be the caller's writable slots; on success the
/// other two slots are written too, as the C wrote them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_copy_temporary(
    object: *mut *mut VmObject,
    _offset: *mut VmOffset,
    src_needs_copy: *mut c_int,
    dst_needs_copy: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises the slots; the object is live or null.
    match unsafe { vm_object::copy_temporary(*object) } {
        Some(copy) => {
            // SAFETY: the caller promises all four slots writable.
            unsafe {
                object.write(copy.object);
                src_needs_copy.write(c_int::from(copy.src_needs_copy));
                dst_needs_copy.write(c_int::from(copy.dst_needs_copy));
            }
            c_int::from(true)
        }
        None => c_int::from(false),
    }
}

/// `vm_object_copy_strategically()` in C.
///
/// # Safety
///
/// The source object must be live and unlocked; the three slots are written
/// as the C wrote them, which is not on every failure path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_copy_strategically(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    size: VmSize,
    dst_object: *mut *mut VmObject,
    dst_offset: *mut VmOffset,
    dst_needs_copy: *mut c_int,
) -> c_int {
    // SAFETY: the caller promises the live, unlocked source.
    match unsafe {
        vm_object::copy_strategically(src_object, src_offset, size)
    } {
        StrategicResult::Copied {
            object,
            offset,
            needs_copy,
        } => {
            // SAFETY: the caller promises the three slots writable.
            unsafe {
                dst_object.write(object.as_ptr());
                dst_offset.write(offset);
                dst_needs_copy.write(c_int::from(needs_copy));
            }
            KERN_SUCCESS
        }
        StrategicResult::Interrupted => {
            // SAFETY: the caller promises the three slots writable.
            unsafe {
                dst_object.write(null_mut());
                dst_offset.write(0);
                dst_needs_copy.write(c_int::from(false));
            }
            MACH_SEND_INTERRUPTED
        }
        StrategicResult::NullObject(error) => {
            // SAFETY: the caller promises the slot writable.
            unsafe { dst_object.write(null_mut()) };
            error.as_kern_return()
        }
        StrategicResult::Failed(error) => error.as_kern_return(),
        StrategicResult::Unchanged => KERN_SUCCESS,
    }
}

/// `vm_object_copy_slowly()` in C.
///
/// # Safety
///
/// The source object must be live and locked on entry and holds a reference;
/// `result_object` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_copy_slowly(
    src_object: *mut VmObject,
    src_offset: VmOffset,
    size: VmSize,
    interruptible: c_int,
    result_object: *mut *mut VmObject,
) -> c_int {
    // SAFETY: the caller promises the locked source and the writable slot.
    let result = unsafe {
        vm_object::copy_slowly(
            src_object,
            src_offset,
            size,
            interruptible != 0,
        )
    };

    match result {
        Ok(object) => {
            // SAFETY: the caller promises the slot writable.
            unsafe { result_object.write(object.as_ptr()) };
            KERN_SUCCESS
        }
        Err(error) => {
            // SAFETY: the caller promises the slot writable; the C wrote a
            // null object on every failure path.
            unsafe { result_object.write(null_mut()) };
            error.as_kern_return()
        }
    }
}

/// `vm_object_enter()` in C.
///
/// # Safety
///
/// `pager` must be null, dead, or a live port; the C panicked when the
/// allocator failed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_enter(
    pager: *mut c_void,
    size: VmSize,
    internal: c_int,
) -> *mut VmObject {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::enter(pager, size, internal != 0) }
        .map_or(null_mut(), NonNull::as_ptr)
}

/// `vm_object_pager_create()` in C.
///
/// # Safety
///
/// The object must be live and locked on entry and is locked on exit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_pager_create(object: *mut VmObject) {
    // SAFETY: the caller promises the locked object.
    unsafe { vm_object::pager_create(object) };
}

/// `vm_object_destroy()` in C.
///
/// # Safety
///
/// `pager` must be null, dead, or a live memory-object port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_destroy(pager: *mut c_void) {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::destroy(pager) };
}

/// `vm_object_page_map()` in C.
///
/// # Safety
///
/// The object must be live, and `map_fn` and its data must be the C's
/// callback pair.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_page_map(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
    map_fn: Option<unsafe extern "C" fn(*mut c_void, VmOffset) -> VmOffset>,
    map_fn_data: *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the object and the callback.
    kern_return(unsafe {
        vm_object::page_map(object, offset, size, map_fn, map_fn_data)
    })
}

/// `vm_object_request_object()` in C.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_request_object(
    port: *mut c_void,
) -> *mut VmObject {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::lookup(port) }.map_or(null_mut(), NonNull::as_ptr)
}

/// `vm_object_coalesce()` in C.
///
/// # Safety
///
/// Both objects must be null or live, and their references move as the C's
/// did; `new_object` and `new_offset` must be writable, and are written only
/// when the C wrote them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_coalesce(
    prev_object: *mut VmObject,
    next_object: *mut VmObject,
    prev_offset: VmOffset,
    next_offset: VmOffset,
    prev_size: VmSize,
    next_size: VmSize,
    new_object: *mut *mut VmObject,
    new_offset: *mut VmOffset,
) -> c_int {
    // SAFETY: the caller promises both objects and both slots.
    match unsafe {
        vm_object::coalesce(
            prev_object,
            next_object,
            prev_offset,
            next_offset,
            prev_size,
            next_size,
        )
    } {
        Some((object, offset)) => {
            // SAFETY: the caller promises both slots writable.
            unsafe {
                new_object.write(object);
                new_offset.write(offset);
            }
            c_int::from(true)
        }
        None => c_int::from(false),
    }
}

/// `vm_object_pager_wakeup()` in C.
///
/// # Safety
///
/// `pager` must be null or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_pager_wakeup(pager: *mut c_void) {
    // SAFETY: the caller promises the port.
    unsafe { vm_object::pager_wakeup(pager) };
}

/// `vm_object_copy_delayed()` in C.
///
/// # Safety
///
/// The source object must be live and unlocked; the returned object is owned
/// by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_object_copy_delayed(
    src_object: *mut VmObject,
) -> *mut VmObject {
    // SAFETY: the caller promises the live, unlocked source.
    unsafe { vm_object::copy_delayed(src_object) }.as_ptr()
}
