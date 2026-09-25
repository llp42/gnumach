// SPDX-License-Identifier: CMU-Mach
// Derived from device/io_req.h, include/device/device_types.h and the
// read loops of i386/i386at/kd_event.c and i386/i386at/kd_mouse.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct io_req` of <device/io_req.h>, the request the device layer and the
//! x86 drivers share.

use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use crate::utils::kd_queue::{KdEvent, KdEventQueue};
use crate::vm::vm_map::VmMapCopy;
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::pin::Pin;
use core::ptr::{self, NonNull};

/// `dev_t` of <sys/types.h>.
pub type DevT = u16;

/// `boolean_t (*)(io_req_t)`, the C type of `io_done`.
pub type IoDone = unsafe extern "C" fn(*mut IoReq) -> c_int;

/// `struct io_req` of <device/io_req.h>: the IO request a driver is handed,
/// and the queue node its first two fields form.
#[repr(C)]
pub struct IoReq {
    pub next: *mut IoReq,
    pub prev: *mut IoReq,
    pub device: *mut c_void,
    pub dev_ptr: *mut c_char,
    pub unit: c_int,
    pub op: c_int,
    pub mode: c_uint,
    pub recnum: c_ulong,
    pub data: *mut c_char,
    pub count: c_long,
    pub alloc_size: usize,
    pub residual: c_long,
    pub error: c_int,
    pub done: Option<IoDone>,
    pub reply_port: *mut c_void,
    pub reply_port_type: c_uint,
    pub link: *mut IoReq,
    pub rlink: *mut IoReq,
    pub copy: *mut VmMapCopy,
    pub total: c_long,
    pub lock: SimpleLock,
    pub physrec: c_long,
    pub rectotal: c_long,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IoReq>() == 176);
    assert!(align_of::<IoReq>() == 8);
    assert!(offset_of!(IoReq, next) == 0);
    assert!(offset_of!(IoReq, prev) == 8);
    assert!(offset_of!(IoReq, device) == 16);
    assert!(offset_of!(IoReq, dev_ptr) == 24);
    assert!(offset_of!(IoReq, unit) == 32);
    assert!(offset_of!(IoReq, op) == 36);
    assert!(offset_of!(IoReq, mode) == 40);
    assert!(offset_of!(IoReq, recnum) == 48);
    assert!(offset_of!(IoReq, data) == 56);
    assert!(offset_of!(IoReq, count) == 64);
    assert!(offset_of!(IoReq, alloc_size) == 72);
    assert!(offset_of!(IoReq, residual) == 80);
    assert!(offset_of!(IoReq, error) == 88);
    assert!(offset_of!(IoReq, done) == 96);
    assert!(offset_of!(IoReq, reply_port) == 104);
    assert!(offset_of!(IoReq, reply_port_type) == 112);
    assert!(offset_of!(IoReq, link) == 120);
    assert!(offset_of!(IoReq, rlink) == 128);
    assert!(offset_of!(IoReq, copy) == 136);
    assert!(offset_of!(IoReq, total) == 144);
    assert!(offset_of!(IoReq, lock) == 152);
    assert!(offset_of!(IoReq, physrec) == 160);
    assert!(offset_of!(IoReq, rectotal) == 168);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IoReq>() == 92);
    assert!(align_of::<IoReq>() == 4);
    assert!(offset_of!(IoReq, next) == 0);
    assert!(offset_of!(IoReq, prev) == 4);
    assert!(offset_of!(IoReq, device) == 8);
    assert!(offset_of!(IoReq, dev_ptr) == 12);
    assert!(offset_of!(IoReq, unit) == 16);
    assert!(offset_of!(IoReq, op) == 20);
    assert!(offset_of!(IoReq, mode) == 24);
    assert!(offset_of!(IoReq, recnum) == 28);
    assert!(offset_of!(IoReq, data) == 32);
    assert!(offset_of!(IoReq, count) == 36);
    assert!(offset_of!(IoReq, alloc_size) == 40);
    assert!(offset_of!(IoReq, residual) == 44);
    assert!(offset_of!(IoReq, error) == 48);
    assert!(offset_of!(IoReq, done) == 52);
    assert!(offset_of!(IoReq, reply_port) == 56);
    assert!(offset_of!(IoReq, reply_port_type) == 60);
    assert!(offset_of!(IoReq, link) == 64);
    assert!(offset_of!(IoReq, rlink) == 68);
    assert!(offset_of!(IoReq, copy) == 72);
    assert!(offset_of!(IoReq, total) == 76);
    assert!(offset_of!(IoReq, lock) == 80);
    assert!(offset_of!(IoReq, physrec) == 84);
    assert!(offset_of!(IoReq, rectotal) == 88);
};

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
    /// The request must stay at its address until it is unlinked: the queue
    /// links point at it.
    pub unsafe fn queue_entry(&mut self) -> Pin<&mut QueueEntry> {
        let p = (self as *mut IoReq).cast::<QueueEntry>();
        // SAFETY: the caller promises the address is stable.
        unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(p)) }
    }
}

/// Drain up to `ior`'s byte count of queued events into its data buffer, and
/// return the bytes copied.
pub fn drain(queue: &mut KdEventQueue, ior: &mut IoReq) -> c_long {
    let mut count: c_long = 0;
    while !queue.is_empty() && count < ior.count {
        let Some(ev) = queue.pop_front() else {
            break;
        };
        let src = (ev as *const KdEvent).cast::<u8>();
        // SAFETY: `device_read_alloc()` allocated `io_count` bytes for the
        // request, and the loop condition keeps this copy inside.
        let dst = unsafe { ior.data.add(count as usize).cast::<u8>() };
        // SAFETY: as above; `src` is the popped event.
        unsafe { ptr::copy_nonoverlapping(src, dst, size_of::<KdEvent>()) };
        count += size_of::<KdEvent>() as c_long;
    }
    count
}

pub const D_NOWAIT: c_uint = 0x8;
pub const DEV_GET_SIZE: c_uint = 0;
pub const DEV_GET_SIZE_DEVICE_SIZE: usize = 0;
pub const DEV_GET_SIZE_RECORD_SIZE: usize = 1;
pub const DEV_GET_SIZE_COUNT: u32 = 2;
pub const KERN_SUCCESS: c_int = 0;
