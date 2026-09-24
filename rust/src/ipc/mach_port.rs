// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/mach_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The four port-right server routines, which `ipc/mach_port.c` used to define
//! and `ipc/mach_port.h` belongs to.

use crate::glue;
use crate::ipc::ipc_object::ipc_object_copyin_type;
use crate::ipc::ipc_port;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};

/// `MACH_PORT_NAME_VALID(name)` of <mach/port.h>: a name is valid when it is
/// neither null nor dead.
const fn port_name_valid(name: c_uint) -> bool {
    name != 0 && name != c_uint::MAX
}

/// `MACH_MSG_TYPE_MOVE_RECEIVE`: the first port type name.
const MOVE_RECEIVE: c_uint = 16;
/// `MACH_MSG_TYPE_MOVE_SEND_ONCE`: the last name
/// `MACH_MSG_TYPE_PORT_ANY_RIGHT` accepts.
const MOVE_SEND_ONCE: c_uint = 18;
/// `MACH_MSG_TYPE_MAKE_SEND_ONCE`: the last name `MACH_MSG_TYPE_PORT_ANY`
/// accepts.
const MAKE_SEND_ONCE: c_uint = 21;

/// `MACH_PORT_RIGHT_RECEIVE` of <mach/port.h>: the right
/// `ipc_port_translate_receive` asks `ipc_object_translate` for.
const MACH_PORT_RIGHT_RECEIVE: c_uint = 1;

/// `MACH_NOTIFY_PORT_DESTROYED` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 5`, a
/// receive right was deallocated.
const MACH_NOTIFY_PORT_DESTROYED: c_int = 0o100 + 5;
/// `MACH_NOTIFY_NO_SENDERS` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 6`, a
/// receive right has no extant send rights.
const MACH_NOTIFY_NO_SENDERS: c_int = 0o100 + 6;
/// `MACH_NOTIFY_DEAD_NAME` of <mach/notify.h>: `MACH_NOTIFY_FIRST + 010`, a
/// send or send-once right died, leaving a dead name.
const MACH_NOTIFY_DEAD_NAME: c_int = 0o100 + 0o10;

/// `IO_DEAD` of <ipc/ipc_object.h>: the dead-object pointer, all bits set.
const IO_DEAD: *mut c_void = usize::MAX as *mut c_void;

/// `IO_VALID(io)` of <ipc/ipc_object.h>: an object is valid when it is neither
/// null nor dead.
fn io_valid(object: *mut c_void) -> bool {
    !object.is_null() && !core::ptr::eq(object, IO_DEAD)
}

/// The [`KernError`] a C `kern_return_t` stands for.
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// `mach_port_rename()` in C.
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

    // SAFETY: the adapter's caller promises a live space, and the two names
    // are plain values; `ipc_object_rename` takes no reference.
    kern_error(unsafe {
        glue::ipc_object_rename(space.as_ptr(), oname, nname)
    })
}

/// `mach_port_insert_right()` in C.
fn insert_right(
    space: Option<IpcSpace>,
    name: c_uint,
    poly: *mut c_void,
    poly_poly: c_uint,
) -> Result<(), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !port_name_valid(name)
        || !(MOVE_RECEIVE..=MOVE_SEND_ONCE).contains(&poly_poly)
    {
        return Err(KernError::InvalidValue);
    }

    if !io_valid(poly) {
        return Err(KernError::InvalidCapability);
    }

    // SAFETY: the adapter's caller promises a live space; `poly` is valid by
    // the check above, and on success `ipc_object_copyout_name` consumes one
    // reference to it, which the caller owns.
    kern_error(unsafe {
        glue::ipc_object_copyout_name(space.as_ptr(), poly, poly_poly, 0, name)
    })
}

/// `mach_port_extract_right()` in C.
fn extract_right(
    space: Option<IpcSpace>,
    name: c_uint,
    msgt_name: c_uint,
) -> Result<(*mut c_void, c_uint), KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if !(MOVE_RECEIVE..=MAKE_SEND_ONCE).contains(&msgt_name) {
        return Err(KernError::InvalidValue);
    }

    let mut object: *mut c_void = core::ptr::null_mut();
    // SAFETY: the adapter's caller promises a live space; the name is a plain
    // value and the out-pointer is this live local.
    kern_error(unsafe {
        glue::ipc_object_copyin(space.as_ptr(), name, msgt_name, &mut object)
    })?;

    Ok((object, ipc_object_copyin_type(msgt_name)))
}

/// The `ipc_port_translate_receive()` macro of <ipc/ipc_port.h> in C.
fn translate_receive(
    space: IpcSpace,
    name: c_uint,
) -> Result<*mut c_void, KernError> {
    let mut port: *mut c_void = core::ptr::null_mut();

    // SAFETY: the caller promises a live space; `name` is a plain value and
    // the out-pointer is this live local.
    kern_error(unsafe {
        glue::ipc_object_translate(
            space.as_ptr(),
            name,
            MACH_PORT_RIGHT_RECEIVE,
            &mut port,
        )
    })?;

    Ok(port)
}

/// `mach_port_request_notification()` in C.
fn request_notification(
    space: Option<IpcSpace>,
    name: c_uint,
    id: c_int,
    sync: c_uint,
    notify: *mut c_void,
) -> Result<Option<IpcPort>, KernError> {
    let Some(space) = space else {
        return Err(KernError::InvalidTask);
    };

    if core::ptr::eq(notify, IO_DEAD) {
        return Err(KernError::InvalidCapability);
    }

    match id {
        MACH_NOTIFY_PORT_DESTROYED => {
            if sync != 0 {
                return Err(KernError::InvalidValue);
            }

            let port = translate_receive(space, name)?;

            // SAFETY: `translate_receive` returned the live, locked port, and
            // `pdrequest` owns the unlock; `notify` is the caller's
            // send-once right.
            let previous = unsafe {
                ipc_port::pdrequest(IpcPort::from_raw(port), notify)
            };

            Ok(IpcPort::new(previous))
        }
        MACH_NOTIFY_NO_SENDERS => {
            let port = translate_receive(space, name)?;

            // SAFETY: `translate_receive` returned the live, locked port, and
            // `nsrequest` owns the unlock; `notify` is the caller's
            // send-once right.
            let previous = unsafe {
                ipc_port::nsrequest(IpcPort::from_raw(port), sync, notify)
            };

            Ok(IpcPort::new(previous))
        }
        MACH_NOTIFY_DEAD_NAME => {
            let mut previous: *mut c_void = core::ptr::null_mut();
            // SAFETY: the caller promises a live space; the name, flag and
            // notify port are plain values; this live local is the out-slot
            // `ipc_right_dnrequest` writes on success only.
            kern_error(unsafe {
                glue::ipc_right_dnrequest(
                    space.as_ptr(),
                    name,
                    c_int::from(sync != 0),
                    notify,
                    &mut previous,
                )
            })?;

            Ok(IpcPort::new(previous))
        }
        _ => Err(KernError::InvalidValue),
    }
}

/// The C `kern_return_t` of a core result: zero, or the error's code.
fn kern_return(result: Result<(), KernError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

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

/// `mach_port_insert_right()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`, and `poly` must be `IO_NULL`,
/// `IO_DEAD` or a live `ipc_object` the caller holds one reference to, which
/// the call consumes on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_insert_right(
    task: *mut c_void,
    name: c_uint,
    poly: *mut c_void,
    poly_poly: c_uint,
) -> c_int {
    kern_return(insert_right(IpcSpace::new(task), name, poly, poly_poly))
}

/// `mach_port_extract_right()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`; `poly` must be writable storage
/// for one object pointer and `poly_poly` for one type name.
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
            // SAFETY: the caller promises both out-pointers are writable; this
            // is the success path the C writes on.
            unsafe {
                poly.write(object);
                poly_poly.write(received);
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `mach_port_request_notification()` in C.
///
/// # Safety
///
/// `task` must be null or a live `ipc_space`; `notify` must be `IP_NULL`,
/// `IP_DEAD` or a live `ipc_port`; and `previous` must be writable storage for
/// one port pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_port_request_notification(
    task: *mut c_void,
    name: c_uint,
    id: c_int,
    sync: c_uint,
    notify: *mut c_void,
    previous: *mut *mut c_void,
) -> c_int {
    match request_notification(IpcSpace::new(task), name, id, sync, notify) {
        Ok(previous_port) => {
            let previous_port =
                previous_port.map_or(core::ptr::null_mut(), IpcPort::as_ptr);

            // SAFETY: the caller promises `previous` is writable; this is the
            // success path the C writes on.
            unsafe { previous.write(previous_port) };

            0
        }
        Err(error) => c_int::from(error),
    }
}
