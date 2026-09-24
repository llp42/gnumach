// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! IPC facilities; mirrors `ipc/`.

use crate::glue;
use crate::kern::lock::SimpleLock;
use core::ffi::{c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::NonNull;

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

/// The `struct ipc_object` header every IPC object begins with, read through
/// a port pointer.
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

/// The byte offset of `ip_kobject` in `struct ipc_port`, read from the built
/// kernels: the prefix is `ipc_target` (40 bytes on x86_64, 32 on i686),
/// `ip_cur_target` (8/4) and the `data` union (8/4).
#[cfg(target_pointer_width = "64")]
const IP_KOBJECT_OFFSET: usize = 56;
#[cfg(target_pointer_width = "32")]
const IP_KOBJECT_OFFSET: usize = 36;

/// The `struct ipc_port` prefix through `ip_kobject`, with the C's own
/// bytes in between left opaque.
#[repr(C)]
struct IpcPortPrefix {
    object: IpcObject,
    _through_data: [u8; IP_KOBJECT_OFFSET - size_of::<IpcObject>()],
    kobject: *mut c_void,
}

const _: () = assert!(offset_of!(IpcPortPrefix, kobject) == IP_KOBJECT_OFFSET);

/// `ipc_port_t`: a send right to a kernel port.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcPort(NonNull<c_void>);

impl IpcPort {
    /// A port from the C side, or `None` for `IP_NULL`.
    pub(crate) fn new(port: *mut c_void) -> Option<Self> {
        NonNull::new(port).map(Self)
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

    /// `ip_lock()` of <ipc/ipc_port.h>: the port's `io_lock_data`, the first
    /// member of `struct ipc_port`.
    ///
    /// # Safety
    ///
    /// The port must be live, as `IP_VALID()` asserts, and this call must not
    /// already hold the port lock.
    pub(crate) unsafe fn lock(self) {
        // SAFETY: the caller promises a live port; `struct ipc_port` begins
        // with the `struct ipc_object` header, whose first member is the
        // lock.
        unsafe { (*self.0.as_ptr().cast::<IpcObject>()).lock.lock() };
    }

    /// `ip_unlock()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live and this call must hold its lock.
    pub(crate) unsafe fn unlock(self) {
        // SAFETY: the caller promises a live port and the held lock; the
        // header is at offset zero as in [`IpcPort::lock()`].
        unsafe { (*self.0.as_ptr().cast::<IpcObject>()).lock.unlock() };
    }

    /// `ip_active()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn is_active(self) -> bool {
        // SAFETY: the caller promises a live port.
        let bits = unsafe { (*self.0.as_ptr().cast::<IpcObject>()).bits };
        bits & IO_BITS_ACTIVE != 0
    }

    /// `ip_kotype()` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn kotype(self) -> c_uint {
        // SAFETY: the caller promises a live port.
        unsafe { (*self.0.as_ptr().cast::<IpcObject>()).bits & IO_BITS_KOTYPE }
    }

    /// `port->ip_kobject` of <ipc/ipc_port.h>.
    ///
    /// # Safety
    ///
    /// The port must be live.
    pub(crate) unsafe fn kobject(self) -> *mut c_void {
        // SAFETY: the caller promises a live port, whose prefix the caller
        // also pins against reallocation for the duration.
        unsafe { (*self.0.as_ptr().cast::<IpcPortPrefix>()).kobject }
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
        unsafe {
            (*self.0.as_ptr().cast::<IpcPortPrefix>()).kobject = kobject
        };
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
