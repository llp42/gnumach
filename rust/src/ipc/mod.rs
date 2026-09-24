// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! IPC facilities; mirrors `ipc/`.

use crate::glue;
use crate::kern::lock::SimpleLock;
use core::ffi::{c_uint, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};

pub mod ipc_init;
pub mod ipc_object;
pub mod ipc_port;
pub mod ipc_table;
pub mod ipc_target;
pub mod ipc_thread;
pub mod mach_port;

/// `IO_BITS_KOTYPE` of <ipc/ipc_object.h>: the low half of `io_bits` names
/// the kobject type.
const IO_BITS_KOTYPE: u32 = 0x0000_ffff;
/// `IO_BITS_ACTIVE` of <ipc/ipc_object.h>: the sign bit marks a live object.
const IO_BITS_ACTIVE: u32 = 0x8000_0000;
/// `IO_DEAD` of <ipc/ipc_object.h>: the one non-null pointer `IP_VALID()`
/// rejects.
const IO_DEAD: *mut c_void = usize::MAX as *mut c_void;

/// The `struct ipc_object` header every IPC object begins with.
#[repr(C)]
struct IpcObject {
    lock: SimpleLock,
    references: u32,
    bits: u32,
}

const _: () = {
    assert!(size_of::<IpcObject>() == 12);
    assert!(align_of::<IpcObject>() == 4);
    assert!(offset_of!(IpcObject, lock) == 0);
    assert!(offset_of!(IpcObject, references) == 4);
    assert!(offset_of!(IpcObject, bits) == 8);
};

/// `struct ipc_mqueue` of <ipc/ipc_mqueue.h>: a target's message and blocked
/// thread stacks, each one pointer.
#[repr(C)]
struct IpcMqueue {
    lock: SimpleLock,
    messages: *mut c_void,
    threads: *mut c_void,
}

/// `struct ipc_target` of <ipc/ipc_target.h>: the common part of ports and
/// port sets.
#[repr(C)]
struct IpcTarget {
    object: IpcObject,
    name: u32,
    messages: IpcMqueue,
}

/// The `data` union of `struct ipc_port`: whichever of the receiver, the
/// destination and the death timestamp the port's state holds.
#[repr(C)]
union IpcPortData {
    receiver: *mut c_void,
    destination: *mut c_void,
    timestamp: u32,
}

/// `struct ipc_port` of <ipc/ipc_port.h>: the whole record `ipc_port_t`
/// points at, its embedded `struct ipc_target` included.  The C's
/// `ip_object`, `ip_receiver`, `ip_messages`, `ip_references` and
/// `ip_receiver_name` names are members of that target.
#[repr(C)]
struct IpcPortRecord {
    target: IpcTarget,
    cur_target: *mut IpcTarget,
    data: IpcPortData,
    kobject: *mut c_void,
    mscount: u32,
    srights: u32,
    sorights: u32,
    nsrequest: *mut c_void,
    pdrequest: *mut c_void,
    dnrequests: *mut c_void,
    pset: *mut c_void,
    seqno: u32,
    msgcount: u32,
    qlimit: u32,
    blocked: *mut c_void,
    protected_payload: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IpcMqueue>() == 24);
    assert!(align_of::<IpcMqueue>() == 8);
    assert!(offset_of!(IpcMqueue, lock) == 0);
    assert!(offset_of!(IpcMqueue, messages) == 8);
    assert!(offset_of!(IpcMqueue, threads) == 16);

    assert!(size_of::<IpcTarget>() == 40);
    assert!(align_of::<IpcTarget>() == 8);
    assert!(offset_of!(IpcTarget, object) == 0);
    assert!(offset_of!(IpcTarget, name) == 12);
    assert!(offset_of!(IpcTarget, messages) == 16);

    assert!(size_of::<IpcPortData>() == 8);
    assert!(align_of::<IpcPortData>() == 8);

    assert!(size_of::<IpcPortRecord>() == 144);
    assert!(align_of::<IpcPortRecord>() == 8);
    assert!(offset_of!(IpcPortRecord, target) == 0);
    assert!(offset_of!(IpcPortRecord, cur_target) == 40);
    assert!(offset_of!(IpcPortRecord, data) == 48);
    assert!(offset_of!(IpcPortRecord, kobject) == 56);
    assert!(offset_of!(IpcPortRecord, mscount) == 64);
    assert!(offset_of!(IpcPortRecord, srights) == 68);
    assert!(offset_of!(IpcPortRecord, sorights) == 72);
    assert!(offset_of!(IpcPortRecord, nsrequest) == 80);
    assert!(offset_of!(IpcPortRecord, pdrequest) == 88);
    assert!(offset_of!(IpcPortRecord, dnrequests) == 96);
    assert!(offset_of!(IpcPortRecord, pset) == 104);
    assert!(offset_of!(IpcPortRecord, seqno) == 112);
    assert!(offset_of!(IpcPortRecord, msgcount) == 116);
    assert!(offset_of!(IpcPortRecord, qlimit) == 120);
    assert!(offset_of!(IpcPortRecord, blocked) == 128);
    assert!(offset_of!(IpcPortRecord, protected_payload) == 136);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IpcMqueue>() == 12);
    assert!(align_of::<IpcMqueue>() == 4);
    assert!(offset_of!(IpcMqueue, lock) == 0);
    assert!(offset_of!(IpcMqueue, messages) == 4);
    assert!(offset_of!(IpcMqueue, threads) == 8);

    assert!(size_of::<IpcTarget>() == 28);
    assert!(align_of::<IpcTarget>() == 4);
    assert!(offset_of!(IpcTarget, object) == 0);
    assert!(offset_of!(IpcTarget, name) == 12);
    assert!(offset_of!(IpcTarget, messages) == 16);

    assert!(size_of::<IpcPortData>() == 4);
    assert!(align_of::<IpcPortData>() == 4);

    assert!(size_of::<IpcPortRecord>() == 88);
    assert!(align_of::<IpcPortRecord>() == 4);
    assert!(offset_of!(IpcPortRecord, target) == 0);
    assert!(offset_of!(IpcPortRecord, cur_target) == 28);
    assert!(offset_of!(IpcPortRecord, data) == 32);
    assert!(offset_of!(IpcPortRecord, kobject) == 36);
    assert!(offset_of!(IpcPortRecord, mscount) == 40);
    assert!(offset_of!(IpcPortRecord, srights) == 44);
    assert!(offset_of!(IpcPortRecord, sorights) == 48);
    assert!(offset_of!(IpcPortRecord, nsrequest) == 52);
    assert!(offset_of!(IpcPortRecord, pdrequest) == 56);
    assert!(offset_of!(IpcPortRecord, dnrequests) == 60);
    assert!(offset_of!(IpcPortRecord, pset) == 64);
    assert!(offset_of!(IpcPortRecord, seqno) == 68);
    assert!(offset_of!(IpcPortRecord, msgcount) == 72);
    assert!(offset_of!(IpcPortRecord, qlimit) == 76);
    assert!(offset_of!(IpcPortRecord, blocked) == 80);
    assert!(offset_of!(IpcPortRecord, protected_payload) == 84);
};

/// `ipc_port_t`: a send right to a kernel port.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcPort(NonNull<c_void>);

impl IpcPort {
    /// A port from the C side, or `None` for `IP_NULL`.
    pub(crate) fn new(port: *mut c_void) -> Option<Self> {
        NonNull::new(port).map(Self)
    }

    /// `IP_VALID(port)` of <ipc/ipc_port.h>: a port that is neither `IP_NULL`
    /// nor `IP_DEAD`.
    pub(crate) fn valid(port: *mut c_void) -> Option<Self> {
        if port.is_null() || ptr::eq(port, IO_DEAD) {
            return None;
        }
        // SAFETY: neither null nor dead, as `IP_VALID()` requires.
        Some(unsafe { Self::from_raw(port) })
    }

    /// A port from the C side where `IP_VALID()` already established that the
    /// pointer is live.
    ///
    /// # Safety
    ///
    /// `port` must be non-null and not `IP_DEAD`.
    pub(crate) unsafe fn from_raw(port: *mut c_void) -> Self {
        // SAFETY: the caller promises the live pointer.
        Self(unsafe { NonNull::new_unchecked(port) })
    }

    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// The full `struct ipc_port` behind the handle.
    fn record(self) -> *mut IpcPortRecord {
        self.0.as_ptr().cast()
    }

    /// `ip_lock()` of <ipc/ipc_port.h>: the port's `io_lock_data`, the first
    /// member of `struct ipc_port`.
    ///
    /// # Safety
    ///
    /// The port must be live, as `IP_VALID()` asserts, and this call must not
    /// already hold the port lock.
    pub(crate) unsafe fn lock(self) {
        // SAFETY: the caller promises a live port, whose record begins with
        // the object lock.
        unsafe { (*self.record()).target.object.lock.lock() };
    }

    /// `ip_unlock()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live and this call must hold its lock.
    pub(crate) unsafe fn unlock(self) {
        // SAFETY: the caller promises a live port and the held lock, which
        // sits as in [`IpcPort::lock()`].
        unsafe { (*self.record()).target.object.lock.unlock() };
    }

    /// `ip_active()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn is_active(self) -> bool {
        // SAFETY: the caller promises a live port.
        let bits = unsafe { (*self.record()).target.object.bits };
        bits & IO_BITS_ACTIVE != 0
    }

    /// `ip_kotype()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn kotype(self) -> c_uint {
        // SAFETY: the caller promises a live port.
        unsafe { (*self.record()).target.object.bits & IO_BITS_KOTYPE }
    }

    /// `port->ip_kobject` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn kobject(self) -> *mut c_void {
        // SAFETY: the caller promises a live port.
        unsafe { (*self.record()).kobject }
    }

    /// The `port->ip_kobject = kobject` assignment of `vm_object_enter()`,
    /// which the C makes without `ipc_kobject_set()`.
    ///
    /// # Safety
    ///
    /// The port must be live, and the caller must hold whatever lock makes
    /// the update atomic as the C site did.
    pub(crate) unsafe fn set_kobject(self, kobject: *mut c_void) {
        // SAFETY: the caller promises a live port and the serialization.
        unsafe { (*self.record()).kobject = kobject };
    }

    /// The `ip_reference()` macro of <ipc/ipc_port.h>: the bare
    /// `io_references++` the C makes with the port lock already held, as
    /// opposed to [`IpcPort::reference()`]'s locking function.
    ///
    /// # Safety
    ///
    /// The port must be live and its lock held.
    pub(crate) unsafe fn increment_references(self) {
        // SAFETY: the caller promises a live port and the held lock.
        unsafe {
            let record = self.record();
            (*record).target.object.references =
                (*record).target.object.references.wrapping_add(1);
        }
    }

    /// The `port->ip_srights++` of the retrieval fast paths and the C's port
    /// routines.
    ///
    /// # Safety
    ///
    /// The port must be live and its lock held; the count is a `u32` on both
    /// ends and wraps as the C increment does.
    pub(crate) unsafe fn increment_srights(self) {
        // SAFETY: the caller promises a live port and the held lock.
        unsafe {
            let record = self.record();
            (*record).srights = (*record).srights.wrapping_add(1);
        }
    }

    /// `ip_reference()` of <ipc/ipc_port.h>, the real `ipc_object_reference()`
    /// function.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn reference(self) {
        // SAFETY: the caller promises a live port; `ip_object` is at offset
        // zero, so the port pointer is the object pointer.
        unsafe { glue::ipc_object_reference(self.as_ptr()) };
    }

    /// `ip_release()` of <ipc/ipc_port.h>, the real `ipc_object_release()`
    /// function.
    ///
    /// # Safety
    ///
    /// The port must be live and hold a reference.
    pub(crate) unsafe fn release(self) {
        // SAFETY: the caller promises a live port holding a reference;
        // `ip_object` is at offset zero.
        unsafe { glue::ipc_object_release(self.as_ptr()) };
    }
}

/// `ipc_space_t`: a port namespace, opaque to Rust so far.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcSpace(NonNull<c_void>);

impl IpcSpace {
    /// A space from the C side, or `None` for `IS_NULL`.
    pub(crate) fn new(space: *mut c_void) -> Option<Self> {
        NonNull::new(space).map(Self)
    }

    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
    }
}
