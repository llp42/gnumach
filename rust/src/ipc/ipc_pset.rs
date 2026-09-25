// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_pset.c and ipc/ipc_pset.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The port-set routines, which `ipc/ipc_pset.c` used to define and
//! `ipc/ipc_pset.h` declares.

use crate::ipc::ipc_kmsg::MsgReturn;
use crate::ipc::ipc_mqueue;
use crate::ipc::ipc_object;
use crate::ipc::ipc_target;
use crate::ipc::{
    IOT_PORT_SET, IpcPort, IpcSpace, IpcTarget, MACH_PORT_TYPE_PORT_SET,
};
use crate::kern::types::KernError;
use core::ffi::c_uint;
use core::ptr;

/// `IOT_PORT_SET` of <ipc/ipc_object.h> as `ipc_object_alloc()` takes it.
const IOT_PORT_SET_OBJECT: c_uint = IOT_PORT_SET as c_uint;

/// `ipc_pset_alloc()` in C.
///
/// # Safety
///
/// `space` must be live and nothing may be locked; may allocate memory.  On
/// success the returned set is locked and the caller does not hold a
/// reference.
pub(crate) unsafe fn alloc(
    space: IpcSpace,
) -> Result<(c_uint, *mut IpcTarget), KernError> {
    // SAFETY: the caller promises a live space with nothing locked.
    let (name, object) = unsafe {
        ipc_object::alloc(
            space,
            IOT_PORT_SET_OBJECT,
            MACH_PORT_TYPE_PORT_SET,
            0,
        )
    }?;
    let pset = object.cast::<IpcTarget>();

    // SAFETY: the fresh, locked object is a port set; this initializes it as
    // the C did.
    unsafe { ipc_target::init(pset, name) };
    Ok((name, pset))
}

/// `ipc_pset_alloc_name()` in C.
///
/// # Safety
///
/// `space` must be live and nothing may be locked; may allocate memory.  On
/// success the returned set is locked and the caller does not hold a
/// reference.
pub(crate) unsafe fn alloc_name(
    space: IpcSpace,
    name: c_uint,
) -> Result<*mut IpcTarget, KernError> {
    // SAFETY: the caller promises a live space with nothing locked.
    let object = unsafe {
        ipc_object::alloc_name(
            space,
            IOT_PORT_SET_OBJECT,
            MACH_PORT_TYPE_PORT_SET,
            0,
            name,
        )
    }?;
    let pset = object.cast::<IpcTarget>();

    // SAFETY: the fresh, locked object is a port set; this initializes it as
    // the C did.
    unsafe { ipc_target::init(pset, name) };
    Ok(pset)
}

/// `ipc_pset_add()` in C.
///
/// # Safety
///
/// `pset` and `port` must be live and locked, the port must not be in a set,
/// and the set's owner must be the port's receiver.
pub(crate) unsafe fn add(pset: *mut IpcTarget, port: IpcPort) {
    // SAFETY: the caller promises both objects live and locked.
    unsafe {
        port.set_pset(pset.cast());
        port.set_cur_target(pset);
        IpcTarget::increment_references(pset);

        let port_queue = port.messages();
        let pset_queue = (*pset).messages();

        (*port_queue).lock();
        (*pset_queue).lock();

        ipc_mqueue::move_messages(pset_queue, port_queue, port);

        (*pset_queue).unlock();
        ipc_mqueue::changed(port_queue, MsgReturn::RCV_PORT_CHANGED);
        (*port_queue).unlock();
    }
}

/// `ipc_pset_remove()` in C.
///
/// # Safety
///
/// `pset` and `port` must be live and locked, and the port must be active and
/// a member of the set.
pub(crate) unsafe fn remove(pset: *mut IpcTarget, port: IpcPort) {
    // SAFETY: the caller promises both objects live and locked.
    unsafe {
        port.set_pset(ptr::null_mut());
        port.set_cur_target(port.record().cast::<IpcTarget>());
        IpcTarget::decrement_references(pset);

        let port_queue = port.messages();
        let pset_queue = (*pset).messages();

        (*port_queue).lock();
        (*pset_queue).lock();

        ipc_mqueue::move_messages(port_queue, pset_queue, port);

        (*pset_queue).unlock();
        (*port_queue).unlock();
    }
}

/// `ipc_pset_move()` in C.
///
/// # Safety
///
/// `space` must be live and read-locked, and `port` live with `nset` either
/// `IPS_NULL` or a live port set.
pub(crate) unsafe fn move_between(
    space: IpcSpace,
    port: IpcPort,
    nset: *mut IpcTarget,
) -> Result<(), KernError> {
    // SAFETY: the caller promises a live port with nothing locked.
    unsafe { port.lock() };

    // SAFETY: the port is live and locked.
    let mut oset = unsafe { port.pset().cast::<IpcTarget>() };

    if oset == nset {
        // SAFETY: the caller holds the space read lock.
        unsafe { space.lock_done() };
    } else if oset.is_null() {
        // SAFETY: a non-null `nset` names a live port set.
        unsafe { (*nset).lock() };
        // SAFETY: the caller holds the space read lock.
        unsafe { space.lock_done() };

        // SAFETY: both are live, active, and locked.
        unsafe { add(nset, port) };

        // SAFETY: the set is live and locked.
        unsafe { (*nset).unlock() };
    } else if nset.is_null() {
        // SAFETY: the caller holds the space read lock.
        unsafe { space.lock_done() };
        // SAFETY: the old set holds a reference for the port.
        unsafe { (*oset).lock() };

        // SAFETY: both are live and locked.
        unsafe { remove(oset, port) };

        // SAFETY: the set is live and locked after the removal.
        if unsafe { (*oset).is_active() } {
            // SAFETY: the set is live and locked.
            unsafe { (*oset).unlock() };
        } else {
            // SAFETY: the set is live and locked, and `remove` consumed the
            // port's reference to it.
            unsafe { IpcTarget::check_unlock(oset) };
            oset = ptr::null_mut();
        }
    } else {
        // The C locks the two sets in address order so concurrent moves
        // cannot deadlock.
        if oset.addr() < nset.addr() {
            // SAFETY: both are live port sets with a reference held through
            // the space or the port.
            unsafe {
                (*oset).lock();
                (*nset).lock();
            }
        } else {
            // SAFETY: as above.
            unsafe {
                (*nset).lock();
                (*oset).lock();
            }
        }

        // SAFETY: the caller holds the space read lock.
        unsafe { space.lock_done() };

        // SAFETY: both sets and the port are live and locked; the port
        // cannot be inactive, so the old set stays live through the
        // reference the port holds.
        unsafe {
            remove(oset, port);
            add(nset, port);
            (*nset).unlock();
            IpcTarget::check_unlock(oset);
        }
    }

    // SAFETY: the port is live and locked.
    unsafe { port.unlock() };

    if nset.is_null() && oset.is_null() {
        Err(KernError::NotInSet)
    } else {
        Ok(())
    }
}

/// `ipc_pset_destroy()` in C.
///
/// # Safety
///
/// `pset` must be a live, locked, active port set, and the caller's reference
/// is consumed; on return the set is unlocked and dead.
pub(crate) unsafe fn destroy(pset: *mut IpcTarget) {
    // SAFETY: the caller promises a live, locked set with a reference.
    unsafe {
        IpcTarget::clear_active(pset);

        let mqueue = (*pset).messages();
        (*mqueue).lock();
        ipc_mqueue::changed(mqueue, MsgReturn::RCV_PORT_DIED);
        (*mqueue).unlock();

        ipc_target::terminate(pset);

        IpcTarget::decrement_references(pset);
        IpcTarget::check_unlock(pset);
    }
}
