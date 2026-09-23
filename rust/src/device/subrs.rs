// SPDX-License-Identifier: CMU-Mach
// Derived from device/subrs.c:
//   Copyright (c) 1993,1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The random device subroutines of `device/subrs.c`, declared in
//! <device/subrs.h> and <device/if_ether.h>.
//!
//! `ether_sprintf()` renders a six-byte Ethernet address into one
//! static buffer, and `sleep()`/`wakeup()` are the BSD compatibility
//! wrappers over the scheduler's wait and wake primitives.  The fourth
//! function of the C file, `if_init_queues()`, stays C.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_wakeup_prim,
};
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;

/// `digits[]` of `ether_sprintf()`.
const DIGITS: [u8; 16] = *b"0123456789abcdef";

/// The `static char etherbuf[18]` of `ether_sprintf()`, the one
/// buffer every call returns.
static ETHERBUF: SyncCell<[u8; 18]> = SyncCell(UnsafeCell::new([0; 18]));

/// Render `address` into `buf` as `xx:xx:xx:xx:xx:xx`, with the
/// terminator the C writes by stepping back over the last colon.
fn format_into(address: &[u8; 6], buf: &mut [u8; 18]) {
    let mut cp = 0;
    for &byte in address {
        // `cp` advances three per byte, so the last write is index 17,
        // the buffer's last byte.
        buf[cp] = DIGITS[usize::from(byte >> 4)];
        buf[cp + 1] = DIGITS[usize::from(byte & 0xf)];
        buf[cp + 2] = b':';
        cp += 3;
    }
    // The C's `*--cp = 0`: the terminator replaces the last colon.
    buf[17] = 0;
}

/// Convert an Ethernet address to printable (loggable) form, as
/// `ether_sprintf()` of device/subrs.c does.
///
/// The result names one static 18-byte buffer, which every call
/// overwrites, exactly as the C's single buffer does.
///
/// # Safety
///
/// `ap` must be readable for six bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ether_sprintf(ap: *const u8) -> *mut c_char {
    // SAFETY: the caller promises six readable bytes; a `[u8; 6]`
    // needs no alignment a byte pointer does not already have.
    let address = unsafe { &*ap.cast::<[u8; 6]>() };
    // SAFETY: `ETHERBUF` is the C's one buffer; the callers' interrupt
    // level serializes its use, as it did in C.
    let buf = unsafe { &mut *ETHERBUF.0.get() };
    format_into(address, buf);
    buf.as_mut_ptr().cast::<c_char>()
}

/// The event a wait and its matching wake share: an opaque
/// `vm_offset_t` that nothing ever dereferences, the C `(event_t)
/// channel` cast.
fn event(channel: VmOffset) -> *mut c_void {
    ptr::with_exposed_provenance_mut(channel)
}

/// Suspend the current thread on `channel` until a matching
/// [`wakeup()`].  `sleep()` of device/subrs.c.
///
/// The C's `priority` argument was unused, and is ignored here too.
///
/// # Safety
///
/// `channel` must be the event the matching [`wakeup()`] names, and
/// the current thread must not already be waiting on an event; the
/// call blocks until the wakeup.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sleep(channel: VmOffset, _priority: c_int) {
    // SAFETY: the caller names the matching event; `0` is the C's
    // `FALSE` non-interruptible wait, and the null continuation
    // resumes the caller with nothing to run.
    unsafe {
        assert_wait(event(channel), 0);
        glue::thread_block(None);
    }
}

/// Wake every thread waiting on `channel`.  `wakeup()` of
/// device/subrs.c, the BSD compatibility name for the
/// [`thread_wakeup_prim()`] call the C's `thread_wakeup` macro
/// expands to.
///
/// # Safety
///
/// `channel` must be the event the matching [`sleep()`] or
/// `assert_wait()` names.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wakeup(channel: VmOffset) {
    // SAFETY: the caller names the matching wait's event; `0` is the
    // macro's `FALSE`, so every waiter is woken, normally.
    unsafe {
        thread_wakeup_prim(event(channel), 0, THREAD_AWAKENED);
    }
}
