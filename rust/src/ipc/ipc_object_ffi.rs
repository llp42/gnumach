// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_object.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the object module, one adapter per symbol
//! `ipc/ipc_object.c` used to define and `ipc/ipc_object.h` declares.

use crate::ipc::IpcSpace;
use crate::ipc::ipc_object;
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_object_reference()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `object` must be a live IPC object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_reference(object: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_object::reference(object) };
}

/// `ipc_object_release()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `object` must be a live IPC object holding a reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_release(object: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { ipc_object::release(object) };
}

/// `ipc_object_translate()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `objectp` writable storage for one
/// object pointer, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_translate(
    space: *mut c_void,
    name: c_uint,
    right: c_uint,
    objectp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::translate(space, name, right) } {
        Ok(object) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { objectp.write(object) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_alloc_dead()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `namep` writable storage for one
/// name, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_alloc_dead(
    space: *mut c_void,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::alloc_dead(space) } {
        Ok(name) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { namep.write(name) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_alloc_dead_name()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space` and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_alloc_dead_name(
    space: *mut c_void,
    name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::alloc_dead_name(space, name) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_alloc()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`; `namep` and `objectp` must be writable
/// storage for one name and one object pointer, written only on success, and
/// nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_alloc(
    space: *mut c_void,
    otype: c_uint,
    type_: c_uint,
    urefs: c_uint,
    namep: *mut c_uint,
    objectp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::alloc(space, otype, type_, urefs) } {
        Ok((name, object)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                namep.write(name);
                objectp.write(object);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_alloc_name()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `objectp` writable storage for one
/// object pointer, written only on success, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_alloc_name(
    space: *mut c_void,
    otype: c_uint,
    type_: c_uint,
    urefs: c_uint,
    name: c_uint,
    objectp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::alloc_name(space, otype, type_, urefs, name) } {
        Ok(object) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { objectp.write(object) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_copyin_type()` of ipc/ipc_object.c.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_object_copyin_type(msgt_name: c_uint) -> c_uint {
    ipc_object::copyin_type(msgt_name)
}

/// `ipc_object_copyin()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `objectp` writable storage for one
/// object pointer, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_copyin(
    space: *mut c_void,
    name: c_uint,
    msgt_name: c_uint,
    objectp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::copyin(space, name, msgt_name) } {
        Ok(object) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { objectp.write(object) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_copyin_from_kernel()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `object` must be a live IPC object and the caller must own the right
/// `msgt_name` describes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_copyin_from_kernel(
    object: *mut c_void,
    msgt_name: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_object::copyin_from_kernel(object, msgt_name) };
}

/// `ipc_object_destroy()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `object` must be a live `ipc_object` of the port kind, and the caller must
/// own the one reference the destruction consumes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_destroy(
    object: *mut c_void,
    msgt_name: c_uint,
) {
    // SAFETY: the caller's contract.
    unsafe { ipc_object::destroy_object(object, msgt_name) };
}

/// `ipc_object_copyout()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `object` a live IPC object the caller
/// holds one reference to, `namep` writable storage for one name, and nothing
/// may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_copyout(
    space: *mut c_void,
    object: *mut c_void,
    msgt_name: c_uint,
    overflow: c_int,
    namep: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe {
        ipc_object::copyout(space, object, msgt_name, overflow != 0)
    } {
        Ok(name) => {
            // SAFETY: the caller promises the out-pointer is writable; this
            // is the C's success path.
            unsafe { namep.write(name) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_copyout_name()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `object` a live IPC object the caller
/// holds one reference to, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_copyout_name(
    space: *mut c_void,
    object: *mut c_void,
    msgt_name: c_uint,
    overflow: c_int,
    name: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe {
        ipc_object::copyout_name(space, object, msgt_name, overflow != 0, name)
    } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_object_copyout_dest()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, `object` a live IPC object that is
/// locked and active, and `namep` writable storage for one name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_copyout_dest(
    space: *mut c_void,
    object: *mut c_void,
    msgt_name: c_uint,
    namep: *mut c_uint,
) {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    let name = unsafe { ipc_object::copyout_dest(space, object, msgt_name) };

    // SAFETY: the caller promises the out-pointer is writable.
    unsafe { namep.write(name) };
}

/// `ipc_object_rename()` of ipc/ipc_object.c.
///
/// # Safety
///
/// `space` must be a live `ipc_space` and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_rename(
    space: *mut c_void,
    oname: c_uint,
    nname: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    match unsafe { ipc_object::rename(space, oname, nname) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}
