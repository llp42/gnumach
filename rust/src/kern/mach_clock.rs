// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/mach_clock.h:
//   Copyright (C) 2006, 2007 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel timeout element, which `kern/mach_clock.h` declares.
//!
//! Only `struct timeout` and its `TIMEOUT_*` bits move here.
//! `kern/mach_clock.c` is still C and keeps touching the same record
//! through the C macro spellings; the scheduler reaches it through
//! [`reset_timeout_check()`].

use crate::glue;
use crate::kern::queue::QueueEntry;
use core::ffi::c_void;
use core::mem::offset_of;

/// `struct timeout` of <kern/mach_clock.h>: a kernel timeout element.
#[repr(C)]
pub struct Timeout {
    /// `chain`: links the element into the timeout queue.
    pub chain: QueueEntry,
    /// `fcn`: the routine called at expiry.
    pub fcn: Option<unsafe extern "C" fn(*mut c_void)>,
    /// `param`: the argument passed to `fcn`.
    pub param: *mut c_void,
    /// `t_time`: the expiration time, in ticks since boot.
    pub t_time: usize,
    /// `set`: the `TIMEOUT_*` bits.
    pub set: u8,
}

/// `TIMEOUT_ALLOC` in <kern/mach_clock.h>: allocated from the pool.
pub const TIMEOUT_ALLOC: u8 = 0x1;
/// `TIMEOUT_ACTIVE`: the timeout is active.
pub const TIMEOUT_ACTIVE: u8 = 0x2;
/// `TIMEOUT_PENDING`: the timeout waits for expiry.
pub const TIMEOUT_PENDING: u8 = 0x4;

/// `reset_timeout_check()` of <kern/mach_clock.h>: cancel the timeout
/// if one is active.
///
/// # Safety
///
/// The caller holds the lock protecting `t`, so it is stable and only
/// its owner can have set it.
pub(crate) unsafe fn reset_timeout_check(t: *mut Timeout) {
    // SAFETY: the caller's contract; `set` is a plain byte field.
    if unsafe { (*t).set } & TIMEOUT_ACTIVE != 0 {
        // SAFETY: the same contract, and `reset_timeout()` takes the
        // element off the timeout queue at splsched.
        unsafe { glue::reset_timeout(t.cast()) };
    }
}

// `struct timeout`: the queue chain, the callback pair, the expiry and
// the state byte.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Timeout>() == 48);
    assert!(offset_of!(Timeout, chain) == 0);
    assert!(offset_of!(Timeout, fcn) == 16);
    assert!(offset_of!(Timeout, param) == 24);
    assert!(offset_of!(Timeout, t_time) == 32);
    assert!(offset_of!(Timeout, set) == 40);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Timeout>() == 24);
    assert!(offset_of!(Timeout, chain) == 0);
    assert!(offset_of!(Timeout, fcn) == 8);
    assert!(offset_of!(Timeout, param) == 12);
    assert!(offset_of!(Timeout, t_time) == 16);
    assert!(offset_of!(Timeout, set) == 20);
};
