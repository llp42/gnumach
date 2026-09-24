// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_port.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the port module, one adapter per symbol
//! `ipc/ipc_port.c` used to define and `ipc/ipc_port.h` declares.

use crate::ipc::ipc_port;
use crate::ipc::{IpcPort, IpcSpace};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_port_dnrequest()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live and locked, `soright` must be `IP_NULL` or a live
/// send-once right, and `indexp` must be writable storage for one index,
/// written only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_dnrequest(
    port: *mut c_void,
    name: c_uint,
    soright: *mut c_void,
    indexp: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_port::dnrequest(port, name, soright) } {
        Ok(index) => {
            // SAFETY: the caller promises `indexp` is writable; the C wrote
            // it only on success.
            unsafe { indexp.write(index) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_port_dngrow()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked on entry, with the caller holding
/// a reference; it is unlocked on return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_dngrow(port: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    match unsafe { ipc_port::dngrow(port) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `ipc_port_dncancel()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live and locked, with a live dead-name request table, and
/// `index` must name a live request in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_dncancel(
    port: *mut c_void,
    _name: c_uint,
    index: c_uint,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::dncancel(port, index) }
}

/// `ipc_port_pdrequest()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; `notify` must be `IP_NULL` or a
/// live send-once right the call consumes, and `previousp` must be writable
/// storage for one port pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_pdrequest(
    port: *mut c_void,
    notify: *mut c_void,
    previousp: *mut *mut c_void,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    let previous = unsafe { ipc_port::pdrequest(port, notify) };

    // SAFETY: the caller promises `previousp` is writable.
    unsafe { previousp.write(previous) };
}

/// `ipc_port_nsrequest()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; `notify` must be `IP_NULL` or a
/// live send-once right the call consumes, and `previousp` must be writable
/// storage for one port pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_nsrequest(
    port: *mut c_void,
    sync: c_uint,
    notify: *mut c_void,
    previousp: *mut *mut c_void,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    let previous = unsafe { ipc_port::nsrequest(port, sync, notify) };

    // SAFETY: the caller promises `previousp` is writable.
    unsafe { previousp.write(previous) };
}

/// `ipc_port_set_qlimit()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_set_qlimit(
    port: *mut c_void,
    qlimit: c_uint,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::set_qlimit(port, qlimit) };
}

/// `ipc_port_lock_mqueue()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked.  The port set's lock and message
/// queue locks may be taken; the returned queue is locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_lock_mqueue(
    port: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::lock_mqueue(port) }.cast()
}

/// `ipc_port_set_seqno()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; the message queue lock may be
/// taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_set_seqno(port: *mut c_void, seqno: c_uint) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::set_seqno(port, seqno) };
}

/// `ipc_port_set_protected_payload()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; the message queue lock may be
/// taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_set_protected_payload(
    port: *mut c_void,
    payload: usize,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::set_protected_payload(port, payload) };
}

/// `ipc_port_clear_protected_payload()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; the message queue lock may be
/// taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_clear_protected_payload(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::clear_protected_payload(port) };
}

/// `ipc_port_clear_receiver()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked; port set and message queue locks
/// may be taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_clear_receiver(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::clear_receiver(port) };
}

/// `ipc_port_init()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be a fresh, locked port whose fields are unwritten, and
/// `space` must be the live space that will hold it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_init(
    port: *mut c_void,
    space: *mut c_void,
    name: c_uint,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::init(port, space, name) };
}

/// `ipc_port_destroy()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live, active, and locked, and the caller's reference is
/// consumed; on return the port is destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_destroy(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::destroy(port) };
}

/// `ipc_port_check_circularity()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` and `dest` must be live ports and no port locks may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_check_circularity(
    port: *mut c_void,
    dest: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    c_int::from(unsafe { ipc_port::check_circularity(port, dest) })
}

/// `ipc_port_lookup_notify()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `space` must be live, active, and locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_lookup_notify(
    space: *mut c_void,
    name: c_uint,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::lookup_notify(space, name) }
        .map_or(core::ptr::null_mut(), IpcPort::as_ptr)
}

/// `ipc_port_make_send()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live and active, and no lock may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_make_send(port: *mut c_void) -> *mut c_void {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::make_send(port) }.as_ptr()
}

/// `ipc_port_copy_send()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be `IP_NULL`, `IP_DEAD`, or a live port, and no lock may be
/// held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_copy_send(port: *mut c_void) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_port::copy_send(port) }
}

/// `ipc_port_copyout_send()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `sright` must be `IP_NULL`, `IP_DEAD`, or a live send right, `space` must
/// be a live space, and nothing may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_copyout_send(
    sright: *mut c_void,
    space: *mut c_void,
) -> c_uint {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::copyout_send(sright, space) }
}

/// `ipc_port_release_send()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be a live port holding one send right, and nothing may be
/// locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_release_send(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::release_send(port) };
}

/// `ipc_port_make_sonce()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be live and active, and no lock may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_make_sonce(
    port: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::make_sonce(port) }.as_ptr()
}

/// `ipc_port_release_sonce()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be a live port holding one send-once right, and nothing may be
/// locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_release_sonce(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::release_sonce(port) };
}

/// `ipc_port_release_receive()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be a live receive right in limbo or in transit, and nothing
/// may be locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_release_receive(port: *mut c_void) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::release_receive(port) };
}

/// `ipc_port_alloc_special()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `space` must be a live space, and the port cache must be initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_alloc_special(
    space: *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    let space = unsafe { IpcSpace::from_raw(space) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::alloc_special(space) }
        .map_or(core::ptr::null_mut(), IpcPort::as_ptr)
}

/// `ipc_port_dealloc_special()` of ipc/ipc_port.c.
///
/// # Safety
///
/// `port` must be a live port in a special space, and `space` must be that
/// live space.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_port_dealloc_special(
    port: *mut c_void,
    _space: *mut c_void,
) {
    // SAFETY: the caller's contract.
    let port = unsafe { IpcPort::from_raw(port) };

    // SAFETY: the caller's contract.
    unsafe { ipc_port::dealloc_special(port) };
}

/// `ipc_port_alloc()` of ipc/ipc_port.c.
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
    let space = unsafe { IpcSpace::from_raw(space) };

    match ipc_port::alloc(space) {
        Ok((name, port)) => {
            // SAFETY: the caller promises both out-pointers are writable;
            // this is the C's success path.
            unsafe {
                namep.write(name);
                portp.write(port.as_ptr());
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_port_alloc_name()` of ipc/ipc_port.c.
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
    let space = unsafe { IpcSpace::from_raw(space) };

    match ipc_port::alloc_name(space, name) {
        Ok(port) => {
            // SAFETY: the caller promises `portp` is writable; this is the
            // C's success path.
            unsafe { portp.write(port.as_ptr()) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `ipc_port_timestamp()` of ipc/ipc_port.c.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_port_timestamp() -> c_uint {
    ipc_port::timestamp()
}
