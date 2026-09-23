// SPDX-License-Identifier: CMU-Mach
// Derived from device/chario.c:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The tty delayed-reply completion, which `device/chario.c` used to
//! define and <device/tty.h> declares.
//!
//! `tty_queue_completion()` drains a tty's delayed request queue and
//! completes every request it finds.  It is the step the line
//! discipline takes when a condition that blocked the request clears:
//! the entries are `struct io_req` records whose queue chain is their
//! first field, so the entry address is the request [`glue::iodone()`]
//! takes.  The rest of the file -- the character buffers and the tty
//! status entries -- stays C.

use crate::glue;
use crate::kern::queue::QueueEntry;
use core::ffi::c_void;
use core::pin::Pin;
use core::ptr::NonNull;

/// Complete every request waiting on `head`.
///
/// # Safety
///
/// `head` must be an initialized queue head that stays at its address
/// while entries are linked, every entry must be an `io_req` record,
/// and nothing else may access the queue during the call.  The tty
/// layer holds `spltty` and the tty lock when it calls.
unsafe fn complete(mut head: Pin<&mut QueueEntry>) {
    loop {
        // SAFETY: the caller's contract holds on every iteration, and
        // `head` is the same address throughout.
        let Some(ior) = (unsafe { head.as_mut().pop_front() }) else {
            return;
        };
        // SAFETY: every entry of these queues is an `io_req`, whose
        // chain is its first two fields.
        unsafe { glue::iodone(ior.as_ptr().cast::<c_void>()) };
    }
}

/// Complete every request waiting on `queue`.  `tty_queue_completion()`
/// in C.
///
/// # Safety
///
/// `queue` must be an initialized queue head that stays at its address
/// while entries are linked, every entry must be an `io_req` record,
/// and nothing else may access the queue during the call.  A null
/// pointer drains nothing, where the C would fault.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_queue_completion(queue: *mut QueueEntry) {
    let Some(queue) = NonNull::new(queue) else {
        return;
    };
    // SAFETY: the caller promises `queue` is a valid, stable queue
    // head; every entry is an `io_req`.
    let head = unsafe { QueueEntry::pin_in_place(queue) };
    // SAFETY: as above; `complete` keeps the same contract.
    unsafe { complete(head) };
}
