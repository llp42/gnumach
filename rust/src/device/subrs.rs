// SPDX-License-Identifier: CMU-Mach
// Derived from device/subrs.c:
//   Copyright (c) 1993,1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The random device subroutines of `device/subrs.c`, declared in
//! <device/subrs.h> and <device/if_ether.h>.

use crate::arch::types::VmOffset;
use crate::device::net_io::IfNet;
use crate::kern::queue::queue_init;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_wakeup_prim,
};
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;

/// `IFQ_MAXLEN` of <device/if_hdr.h>.
const IFQ_MAXLEN: c_int = 50;

/// `digits[]` of `ether_sprintf()`.
const DIGITS: [u8; 16] = *b"0123456789abcdef";

/// The `static char etherbuf[18]` of `ether_sprintf()`, the one buffer every
/// call returns.
static ETHERBUF: SyncCell<[u8; 18]> = SyncCell(UnsafeCell::new([0; 18]));

/// Render `address` into `buf` as `xx:xx:xx:xx:xx:xx`, with the terminator the
/// C writes by stepping back over the last colon.
fn format_into(address: &[u8; 6], buf: &mut [u8; 18]) {
    let mut cp = 0;
    for &byte in address {
        buf[cp] = DIGITS[usize::from(byte >> 4)];
        buf[cp + 1] = DIGITS[usize::from(byte & 0xf)];
        buf[cp + 2] = b':';
        cp += 3;
    }
    buf[17] = 0;
}

/// Convert an Ethernet address to printable (loggable) form, as
/// `ether_sprintf()` of device/subrs.c does.
///
/// # Safety
///
/// `ap` must be readable for six bytes.
pub(crate) unsafe fn ether_sprintf(ap: *const u8) -> *mut c_char {
    // SAFETY: the caller promises six readable bytes; a `[u8; 6]` needs no
    // alignment a byte pointer does not already have.
    let address = unsafe { &*ap.cast::<[u8; 6]>() };
    // SAFETY: `ETHERBUF` is the C's one buffer; the callers' interrupt level
    // serializes its use, as it did in C.
    let buf = unsafe { &mut *ETHERBUF.0.get() };
    format_into(address, buf);
    buf.as_mut_ptr().cast::<c_char>()
}

/// The event a wait and its matching wake share: an opaque `vm_offset_t` that
/// nothing ever dereferences, the C `(event_t) channel` cast.
fn event(channel: VmOffset) -> *mut c_void {
    ptr::with_exposed_provenance_mut(channel)
}

/// `sleep()` of device/subrs.c.
///
/// # Safety
///
/// `channel` must be the event the matching [`wakeup()`] names, and the
/// current thread must not already be waiting on an event; the call blocks
/// until the wakeup.
pub(crate) unsafe fn sleep(channel: VmOffset, _priority: c_int) {
    // SAFETY: the caller names the matching event; `0` is the C's `FALSE`
    // non-interruptible wait, and the null continuation resumes the caller
    // with nothing to run.
    unsafe {
        assert_wait(event(channel), 0);
        thread_block(None);
    }
}

/// `wakeup()` of device/subrs.c, the BSD compatibility name for the
/// [`thread_wakeup_prim()`] call the C's `thread_wakeup` macro expands to.
///
/// # Safety
///
/// `channel` must be the event the matching [`sleep()`] or `assert_wait()`
/// names.
pub(crate) unsafe fn wakeup(channel: VmOffset) {
    // SAFETY: the caller names the matching wait's event; `0` is the macro's
    // `FALSE`, so every waiter is woken, normally.
    unsafe {
        thread_wakeup_prim(event(channel), 0, THREAD_AWAKENED);
    }
}

/// `if_init_queues()` of device/subrs.c.
///
/// # Safety
///
/// `ifp` must be a live interface header that nothing else initializes at the
/// same time.
pub(crate) unsafe fn if_init_queues(ifp: *mut IfNet) {
    // SAFETY: the caller promises the live header.
    let ifp = unsafe { &mut *ifp };

    // SAFETY: the embedded queue heads are initialized here, once.
    unsafe {
        queue_init(ptr::from_mut(&mut ifp.if_snd.ifq_head).cast());
        queue_init(ptr::from_mut(&mut ifp.if_rcv_port_list).cast());
        queue_init(ptr::from_mut(&mut ifp.if_snd_port_list).cast());
    }
    ifp.if_snd.ifq_lock.init();
    ifp.if_snd.ifq_len = 0;
    ifp.if_snd.ifq_maxlen = IFQ_MAXLEN;
    ifp.if_snd.ifq_drops = 0;
    ifp.if_rcv_port_list_lock.init();
    ifp.if_snd_port_list_lock.init();
}
