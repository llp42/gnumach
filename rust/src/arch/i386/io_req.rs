// SPDX-License-Identifier: CMU-Mach
// Derived from device/io_req.h, include/device/device_types.h and the
// read loops of i386/i386at/kd_event.c and i386/i386at/kd_mouse.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct io_req` of <device/io_req.h>, as the x86 drivers share it.
//! The device return codes live in
//! [`return`](crate::device::return).
//!
//! Only a prefix of the request is mirrored, through `io_done`: the
//! fields after it are never read.  The request's first two fields are
//! its queue chain, which is why the C let an `io_req_t` double as its
//! own `queue_entry_t`.
//!
//! This belongs under a `src/device/` module once the device layer has
//! one; until then the two x86 drivers share it here.

use crate::kern::queue::QueueEntry;
use crate::utils::kd_queue::{KdEvent, KdEventQueue};
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::mem::{offset_of, size_of};
use core::pin::Pin;
use core::ptr::{self, NonNull};

/// `dev_t` of <sys/types.h>.
pub type DevT = u16;

/// `boolean_t (*)(io_req_t)`, the C type of `io_done`.
pub type IoDone = unsafe extern "C" fn(*mut IoReq) -> c_int;

/// The part of `struct io_req` of <device/io_req.h> the drivers touch.
#[repr(C)]
#[allow(dead_code)]
pub struct IoReq {
    next: *mut IoReq,
    prev: *mut IoReq,
    device: *mut c_void,
    dev_ptr: *mut c_char,
    unit: c_int,
    op: c_int,
    mode: c_uint,
    recnum: c_ulong,
    data: *mut c_char,
    count: c_long,
    alloc_size: usize,
    residual: c_long,
    error: c_int,
    done: Option<IoDone>,
}

// The queue cast depends on the chain being the first field.
const _: () = assert!(offset_of!(IoReq, next) == 0);
const _: () = assert!(offset_of!(IoReq, prev) == size_of::<*mut c_void>());

impl IoReq {
    /// `io_count`: the byte count the caller asked for.
    pub fn count(&self) -> c_long {
        self.count
    }

    /// `io_mode`: the open/read/write mode.
    pub fn mode(&self) -> c_uint {
        self.mode
    }

    /// Set `io_done`, the completion callback.
    pub fn set_done(&mut self, done: IoDone) {
        self.done = Some(done);
    }

    /// `io_data`: the buffer `device_read_alloc()` set up.
    pub fn data(&self) -> *mut c_char {
        self.data
    }

    /// Set `io_residual` to the bytes not done.
    pub fn set_residual(&mut self, residual: c_long) {
        self.residual = residual;
    }

    /// The request as the queue entry its first two fields form.
    ///
    /// # Safety
    ///
    /// The request must stay at its address until it is unlinked: the
    /// queue links point at it.  The device layer keeps it so.
    pub unsafe fn queue_entry(&mut self) -> Pin<&mut QueueEntry> {
        let p = (self as *mut IoReq).cast::<QueueEntry>();
        // SAFETY: the caller promises the address is stable.
        unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(p)) }
    }
}

/// Drain up to `ior`'s byte count of queued events into its data
/// buffer, and return the bytes copied.  The caller rejected a byte
/// count that is not a multiple of the event size and holds `SPLKD`.
pub fn drain(queue: &mut KdEventQueue, ior: &mut IoReq) -> c_long {
    let mut count: c_long = 0;
    while !queue.is_empty() && count < ior.count {
        let Some(ev) = queue.pop_front() else {
            break;
        };
        let src = (ev as *const KdEvent).cast::<u8>();
        // SAFETY: `device_read_alloc()` allocated `io_count` bytes for
        // the request, and the loop condition keeps this copy inside.
        let dst = unsafe { ior.data.add(count as usize).cast::<u8>() };
        // SAFETY: as above; `src` is the popped event.
        unsafe { ptr::copy_nonoverlapping(src, dst, size_of::<KdEvent>()) };
        count += size_of::<KdEvent>() as c_long;
    }
    count
}

// The mode flag of <device/device_types.h>, the ioctl flavors, and
// the `kern_return_t` of <mach/kern_return.h>; the device return codes
// moved to [`crate::device::r#return`].
pub const D_NOWAIT: c_uint = 0x8;
pub const DEV_GET_SIZE: c_uint = 0;
pub const DEV_GET_SIZE_DEVICE_SIZE: usize = 0;
pub const DEV_GET_SIZE_RECORD_SIZE: usize = 1;
pub const DEV_GET_SIZE_COUNT: u32 = 2;
pub const KERN_SUCCESS: c_int = 0;
