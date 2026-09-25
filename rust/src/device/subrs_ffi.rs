// SPDX-License-Identifier: CMU-Mach
// Derived from device/subrs.c and device/subrs.h:
//   Copyright (c) 1993,1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries of `device/subrs.c`, declared in
//! <device/subrs.h> and <device/if_ether.h>.

use crate::arch::types::VmOffset;
use crate::device::net_io::IfNet;
use crate::device::subrs;
use core::ffi::{c_char, c_int};

/// `ether_sprintf()` in C.
///
/// # Safety
///
/// `ap` must be readable for six bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ether_sprintf(ap: *const u8) -> *mut c_char {
    // SAFETY: the caller promises six readable bytes.
    unsafe { subrs::ether_sprintf(ap) }
}

/// `sleep()` in C.
///
/// # Safety
///
/// `channel` must be the event the matching `wakeup()` names, and the current
/// thread must not already be waiting on an event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sleep(channel: VmOffset, priority: c_int) {
    // SAFETY: the caller names the matching event.
    unsafe { subrs::sleep(channel, priority) }
}

/// `wakeup()` in C.
///
/// # Safety
///
/// `channel` must be the event the matching `sleep()` or `assert_wait()`
/// names.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wakeup(channel: VmOffset) {
    // SAFETY: the caller names the matching wait's event.
    unsafe { subrs::wakeup(channel) }
}

/// `if_init_queues()` in C.
///
/// # Safety
///
/// `ifp` must be a live interface header that nothing else initializes at the
/// same time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn if_init_queues(ifp: *mut IfNet) {
    // SAFETY: the caller promises the live header.
    unsafe { subrs::if_init_queues(ifp) }
}
