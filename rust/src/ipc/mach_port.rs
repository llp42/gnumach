// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/mach_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The three port-right server routines, which `ipc/mach_port.c` used
//! to define and `ipc/mach_port.h` belongs to.
//!
//! Their prototypes are frozen in the generated
//! `ipc/mach_port.server.h`, so the MIG server goes on calling the same
//! symbol names.  The rest of `ipc/mach_port.c` stays C until its space
//! and right locks have Rust homes.

use crate::glue;
use crate::ipc::IpcSpace;
use crate::ipc::ipc_object::ipc_object_copyin_type;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};

/// `MACH_PORT_NAME_VALID(name)` of <mach/port.h>: a name is valid when
/// it is neither null nor dead.
const fn port_name_valid(name: c_uint) -> bool {
    name != 0 && name != c_uint::MAX
}

/// `MACH_MSG_TYPE_MOVE_RECEIVE`: the first port type name.
const MOVE_RECEIVE: c_uint = 16;
/// `MACH_MSG_TYPE_MOVE_SEND_ONCE`: the last name
/// `MACH_MSG_TYPE_PORT_ANY_RIGHT` accepts.
const MOVE_SEND_ONCE: c_uint = 18;
/// `MACH_MSG_TYPE_MAKE_SEND_ONCE`: the last name
/// `MACH_MSG_TYPE_PORT_ANY` accepts.
const MAKE_SEND_ONCE: c_uint = 21;

/// `IO_DEAD` of <ipc/ipc_object.h>: the dead-object pointer, all bits
/// set.
const IO_DEAD: *mut c_void = usize::MAX as *mut c_void;

/// `IO_VALID(io)` of <ipc/ipc_object.h>: an object is valid when it is
/// neither null nor dead.
fn io_valid(object: *mut c_void) -> bool {
    !object.is_null() && !core::ptr::eq(object, IO_DEAD)
}

/// The [`KernError`] a C `kern_return_t` stands for.
///
/// A `kern_return_t` is an `int`, and every code this module receives
/// fits a byte; one that does not cannot name a defined error and
/// becomes [`KernError::Failure`].
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// Changes the name denoting a right, from `oname` to `nname`.
/// `mach_port_rename()` in C.
///
/// `None` is the C `IS_NULL`, and the checks run in the C order: the
/// null space first, then the new name, then the callee.
fn rename(
    space: Option<IpcSpace>,
    oname: c_uint,
    nname: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !port_name_valid(nname) {
        return Err(KernError::InvalidValue);
    }

    // SAFETY: the adapter's caller promises a live space, and the two
    // names are plain values; `ipc_object_rename` takes no reference.
    kern_error(unsafe {
        glue::ipc_object_rename(space.as_ptr(), oname, nname)
    })
}

/// Inserts a right into a space under a specific `name`.
/// `mach_port_insert_right()` in C.
///
/// `poly` must be a live object, not `IO_NULL` or `IO_DEAD`, and
/// `poly_poly` one of the three move names, 16 through 18.  `None` is
/// the C `IS_NULL`, and the checks run in the C order.
fn insert_right(
    space: Option<IpcSpace>,
    name: c_uint,
    poly: *mut c_void,
    poly_poly: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // `MACH_PORT_NAME_VALID(name)` and `MACH_MSG_TYPE_PORT_ANY_RIGHT`,
    // which is the name range 16 through 18.
    if !port_name_valid(name)
        || !(MOVE_RECEIVE..=MOVE_SEND_ONCE).contains(&poly_poly)
    {
        return Err(KernError::InvalidValue);
    }

    if !io_valid(poly) {
        return Err(KernError::InvalidCapability);
    }

    // SAFETY: the adapter's caller promises a live space; `poly` is
    // valid by the check above, and on success
    // `ipc_object_copyout_name` consumes one reference to it, which
    // the caller owns.  The zero is the C `FALSE` for `overflow`.
    kern_error(unsafe {
        glue::ipc_object_copyout_name(space.as_ptr(), poly, poly_poly, 0, name)
    })
}

/// Extracts a right from a space, as if the space voluntarily sent it.
/// `mach_port_extract_right()` in C.
///
/// On success the returned object owns the reference
/// `ipc_object_copyin` acquired, and the returned name is the type the
/// receiver ends up holding.  `None` is the C `IS_NULL`, and the checks
/// run in the C order.
fn extract_right(
    space: Option<IpcSpace>,
    name: c_uint,
    msgt_name: c_uint,
) -> Result<(*mut c_void, c_uint), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    // `MACH_MSG_TYPE_PORT_ANY`, the six names 16 through 21.
    if !(MOVE_RECEIVE..=MAKE_SEND_ONCE).contains(&msgt_name) {
        return Err(KernError::InvalidValue);
    }

    let mut object: *mut c_void = core::ptr::null_mut();
    // SAFETY: the adapter's caller promises a live space; the name is
    // a plain value and the out-pointer is this live local.
    kern_error(unsafe {
        glue::ipc_object_copyin(space.as_ptr(), name, msgt_name, &mut object)
    })?;

    // The range check above is what makes the name one the conversion
    // accepts; `ipc_object_copyin_type` would halt on any other.
    Ok((object, ipc_object_copyin_type(msgt_name)))
}

/// The C `kern_return_t` of a core result: zero, or the error's code.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Changes the name denoting a right.
/// `mach_port_rename()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_rename(
    task: *mut c_void,
    old_name: c_uint,
    new_name: c_uint,
) -> c_int {
    kern_return(rename(IpcSpace::new(task), old_name, new_name))
}

/// Inserts a right into a space under a specific name.
/// `mach_port_insert_right()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`, and `poly` must be
/// `IO_NULL`, `IO_DEAD` or a live `ipc_object` the caller holds one
/// reference to, which the call consumes on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_insert_right(
    task: *mut c_void,
    name: c_uint,
    poly: *mut c_void,
    poly_poly: c_uint,
) -> c_int {
    kern_return(insert_right(IpcSpace::new(task), name, poly, poly_poly))
}

/// Extracts a right from a space, as if the space voluntarily sent it.
/// `mach_port_extract_right()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`; `poly` must be writable
/// storage for one object pointer and `poly_poly` for one type name.
/// Both are written only on success, when `poly` takes the reference
/// `ipc_object_copyin` acquired.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_extract_right(
    task: *mut c_void,
    name: c_uint,
    msgt_name: c_uint,
    poly: *mut *mut c_void,
    poly_poly: *mut c_uint,
) -> c_int {
    match extract_right(IpcSpace::new(task), name, msgt_name) {
        Ok((object, received)) => {
            // SAFETY: the caller promises both out-pointers are
            // writable; this is the success path the C writes on.
            unsafe {
                poly.write(object);
                poly_poly.write(received);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}
