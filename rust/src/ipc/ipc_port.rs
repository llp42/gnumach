// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The port timestamp and the two port allocators, which `ipc/ipc_port.c` used
//! to define and `ipc/ipc_port.h` declares.

use crate::glue;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::lock::SimpleLock;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU32, Ordering};

/// `ipc_port_timestamp_lock_data` of ipc/ipc_port.c: serializes the counter's
/// read and post-increment.
#[unsafe(export_name = "ipc_port_timestamp_lock_data")]
static TIMESTAMP_LOCK: SimpleLock = SimpleLock::new();

/// `ipc_bootstrap()` writes the initial zero before any other thread can reach
/// it; after that every access is a `Relaxed` read or write under
/// [`TIMESTAMP_LOCK`], whose acquire and release order the counter against
/// other callers.
#[unsafe(export_name = "ipc_port_timestamp_data")]
static TIMESTAMP_DATA: AtomicU32 = AtomicU32::new(0);

/// `IOT_PORT` of <ipc/ipc_object.h>: the object type the two allocators draw
/// from, the port cache.
const IOT_PORT: c_uint = 0;

/// `MACH_PORT_TYPE_RECEIVE` of <mach/port.h>: `1 << (right + 16)` for the
/// receive right, the entry bit both allocators install.
const MACH_PORT_TYPE_RECEIVE: c_uint = 1 << 17;

/// `ipc_port_timestamp()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_port_timestamp() -> c_uint {
    TIMESTAMP_LOCK.lock();

    let timestamp = TIMESTAMP_DATA.load(Ordering::Relaxed);
    TIMESTAMP_DATA.store(timestamp.wrapping_add(1), Ordering::Relaxed);

    TIMESTAMP_LOCK.unlock();

    timestamp
}

/// The [`KernError`] a C `kern_return_t` stands for.
fn kern_error(code: c_int) -> Result<(), KernError> {
    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code),
        Err(_) => Err(KernError::Failure),
    }
}

/// `ipc_port_alloc()` in C.
fn alloc(space: IpcSpace) -> Result<(c_uint, IpcPort), KernError> {
    let mut name: c_uint = 0;
    let mut port: *mut c_void = ptr::null_mut();

    // SAFETY: the caller promises a live space; both out-pointers are this
    // function's live locals, and `ipc_object_alloc` writes them only on
    // success.
    kern_error(unsafe {
        glue::ipc_object_alloc(
            space.as_ptr(),
            IOT_PORT,
            MACH_PORT_TYPE_RECEIVE,
            0,
            &mut name,
            &mut port,
        )
    })?;

    // SAFETY: `ipc_object_alloc` reports success only after `io_alloc`
    // returned a live object, so the pointer is non-null; the C dereferenced
    // it in the same place.
    let port = IpcPort(unsafe { NonNull::new_unchecked(port) });

    // SAFETY: `ipc_object_alloc` returns the port live and locked, and
    // `ipc_port_init` initializes its fields in place without unlocking; the
    // caller keeps the unlock.
    unsafe { glue::ipc_port_init(port.as_ptr(), space.as_ptr(), name) };

    Ok((name, port))
}

/// `ipc_port_alloc_name()` in C.
fn alloc_name(space: IpcSpace, name: c_uint) -> Result<IpcPort, KernError> {
    let mut port: *mut c_void = ptr::null_mut();

    // SAFETY: the caller promises a live space; `name` is a plain value and
    // the out-pointer is this function's live local, written only on success.
    kern_error(unsafe {
        glue::ipc_object_alloc_name(
            space.as_ptr(),
            IOT_PORT,
            MACH_PORT_TYPE_RECEIVE,
            0,
            name,
            &mut port,
        )
    })?;

    // SAFETY: `ipc_object_alloc_name` reports success only after `io_alloc`
    // returned a live object, as `alloc` explains.
    let port = IpcPort(unsafe { NonNull::new_unchecked(port) });

    // SAFETY: `ipc_object_alloc_name` returns the port live and locked, and
    // `ipc_port_init` initializes its fields in place without unlocking; the
    // caller keeps the unlock.
    unsafe { glue::ipc_port_init(port.as_ptr(), space.as_ptr(), name) };

    Ok(port)
}

/// `ipc_port_alloc()` in C.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, and `namep` and `portp` must be
/// writable storage for one name and one port pointer, written only on
/// success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_alloc(
    space: *mut c_void,
    namep: *mut c_uint,
    portp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises a live space; the C callers all check
    // `IS_NULL` before this.
    let space = IpcSpace(unsafe { NonNull::new_unchecked(space) });

    match alloc(space) {
        Ok((name, port)) => {
            // SAFETY: the caller promises both out-pointers are writable; this
            // is the C's success path.
            unsafe {
                namep.write(name);
                portp.write(port.as_ptr());
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_port_alloc_name()` in C.
///
/// # Safety
///
/// `space` must be a live `ipc_space`, and `portp` must be writable storage
/// for one port pointer, written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_alloc_name(
    space: *mut c_void,
    name: c_uint,
    portp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller promises a live space; the C callers all check
    // `IS_NULL` before this.
    let space = IpcSpace(unsafe { NonNull::new_unchecked(space) });

    match alloc_name(space, name) {
        Ok(port) => {
            // SAFETY: the caller promises `portp` is writable; this is the C's
            // success path.
            unsafe { portp.write(port.as_ptr()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}
