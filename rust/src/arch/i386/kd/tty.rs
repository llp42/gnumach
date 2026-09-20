// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct tty` and the kd device entry points: open/close/read/write,
//! get/set status, mmap and the line-discipline start.
//!
//! The mirror is `#[repr(C)]` and its offsets are pinned below, because
//! the C tty layer (`device/chario.c`) reads and writes the same bytes;
//! `kd_tty` itself is Rust storage now.  The locks, the line-discipline
//! switch and `ttlowat[]` are reached through the shims in
//! `i386/i386at/kd_glue.c`.

use super::*;
use crate::arch::i386::io_req::{DevT, IoReq};
use crate::glue;
use crate::kern::queue::QueueEntry;
use core::ffi::{c_char, c_int, c_short, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::NonNull;
use core::sync::atomic::AtomicU32;

/// `TS_*` of <device/tty.h>.
const TS_WOPEN: c_int = 0x0000_0004;
const TS_ISOPEN: c_int = 0x0000_0008;
const TS_CARR_ON: c_int = 0x0000_0020;
const TS_BUSY: c_int = 0x0000_0040;
const TS_TTSTOP: c_int = 0x0000_0100;

/// `B115200` of <device/tty_status.h>.
const B115200: u8 = 17;

/// The default console flags `kdopen()` sets: `TF_ODDP|TF_EVENP|TF_ECHO`
/// `|TF_CRMOD|TF_XTABS|TF_LITOUT`.
const KD_TTY_FLAGS: c_int = 0x2 | 0x4 | 0x8 | 0x80 | 0x100 | 0x200;

/// `KDGSTATE` of <i386at/kd.h>.
const KDGSTATE: c_uint = 0x4004_6b03;
/// `KDGKBENT` of <i386at/kd.h>.
const KDGKBENT: c_uint = 0xc005_6b01;
/// `KDSKBENT` of <i386at/kd.h>.
const KDSKBENT: c_uint = 0x8005_6b02;
/// `KDSETBELL` of <i386at/kd.h>.
const KDSETBELL: c_uint = 0x8004_6b04;

use crate::arch::i386::io_req::{D_INVALID_OPERATION, D_SUCCESS};

/// `kdmmap()` refuses offsets past this.
const MAP_LIMIT: usize = 128 * 1024;
/// `kdmmap()`'s failure value, as `(vm_offset_t)-1`.
const MAP_FAILED: usize = usize::MAX;

/// `struct slock` of <kern/lock.h>: one natural word.
///
/// The `struct {} is_a_simple_lock` member occupies no space, so this
/// is the whole `simple_lock_irq_data_t` as well.
#[repr(C)]
pub struct SimpleLock {
    lock_data: AtomicU32,
}

/// `struct cirbuf` of <device/cirbuf.h>.
#[repr(C)]
#[allow(dead_code)]
pub struct Cirbuf {
    c_start: *mut c_char,
    c_end: *mut c_char,
    c_cf: *mut c_char,
    c_cl: *mut c_char,
    c_cc: c_short,
    c_hog: c_short,
}

/// `struct tty` of <device/tty.h>, field for field.
#[repr(C)]
#[allow(dead_code)]
pub struct Tty {
    t_lock: SimpleLock,
    t_inq: Cirbuf,
    t_outq: Cirbuf,
    t_addr: Option<NonNull<c_char>>,
    t_dev: c_int,
    t_start: Option<unsafe extern "C" fn(*mut Tty)>,
    t_stop: Option<unsafe extern "C" fn(*mut Tty, c_int)>,
    t_mctl: Option<unsafe extern "C" fn(*mut Tty, c_int, c_int) -> c_int>,
    t_ispeed: u8,
    t_ospeed: u8,
    t_breakc: c_char,
    t_flags: c_int,
    t_state: c_int,
    t_line: c_int,
    t_delayed_read: QueueEntry,
    t_delayed_write: QueueEntry,
    t_delayed_open: QueueEntry,
    t_timeout: Option<NonNull<c_void>>,
    t_getstat: Option<
        unsafe extern "C" fn(u16, c_uint, *mut c_int, *mut u32) -> c_int,
    >,
    t_setstat:
        Option<unsafe extern "C" fn(u16, c_uint, *mut c_int, u32) -> c_int>,
    t_tops: Option<NonNull<c_void>>,
}

// The C layout, as both configured kernels see it: the lock first, the
// two buffers, and the state/line pair before the delayed queues.
const _: () = assert!(offset_of!(Tty, t_lock) == 0);
const _: () = assert!(size_of::<SimpleLock>() == size_of::<u32>());

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(Tty, t_inq) == 4);
    assert!(offset_of!(Tty, t_outq) == 24);
    assert!(offset_of!(Tty, t_addr) == 44);
    assert!(offset_of!(Tty, t_state) == 72);
    assert!(offset_of!(Tty, t_line) == 76);
    assert!(offset_of!(Tty, t_delayed_read) == 80);
    assert!(offset_of!(Tty, t_timeout) == 104);
    assert!(size_of::<Tty>() == 120);
};
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(Tty, t_inq) == 8);
    assert!(offset_of!(Tty, t_outq) == 48);
    assert!(offset_of!(Tty, t_addr) == 88);
    assert!(offset_of!(Tty, t_state) == 136);
    assert!(offset_of!(Tty, t_line) == 140);
    assert!(offset_of!(Tty, t_delayed_read) == 144);
    assert!(offset_of!(Tty, t_timeout) == 192);
    assert!(size_of::<Tty>() == 224);
};

impl Tty {
    pub(crate) const fn new() -> Self {
        Self {
            t_lock: SimpleLock {
                lock_data: AtomicU32::new(0),
            },
            t_inq: Cirbuf {
                c_start: core::ptr::null_mut(),
                c_end: core::ptr::null_mut(),
                c_cf: core::ptr::null_mut(),
                c_cl: core::ptr::null_mut(),
                c_cc: 0,
                c_hog: 0,
            },
            t_outq: Cirbuf {
                c_start: core::ptr::null_mut(),
                c_end: core::ptr::null_mut(),
                c_cf: core::ptr::null_mut(),
                c_cl: core::ptr::null_mut(),
                c_cc: 0,
                c_hog: 0,
            },
            t_addr: None,
            t_dev: 0,
            t_start: None,
            t_stop: None,
            t_mctl: None,
            t_ispeed: 0,
            t_ospeed: 0,
            t_breakc: 0,
            t_flags: 0,
            t_state: 0,
            t_line: 0,
            t_delayed_read: QueueEntry::unlinked(),
            t_delayed_write: QueueEntry::unlinked(),
            t_delayed_open: QueueEntry::unlinked(),
            t_timeout: None,
            t_getstat: None,
            t_setstat: None,
            t_tops: None,
        }
    }
}

/// `kd_tty` of <i386at/kd.c>.
fn tty() -> &'static mut Tty {
    &mut super::kd().tty
}

fn lock() -> *mut c_void {
    core::ptr::addr_of_mut!(tty().t_lock).cast()
}

fn outq() -> *mut c_void {
    core::ptr::addr_of_mut!(tty().t_outq).cast()
}

/// Feed one character to the line discipline: the `linesw` shim.
pub(crate) fn line_rint(c: u8) {
    let tp = tty();
    // SAFETY: the tty is up once the console is open.
    unsafe { glue::kd_ldisc_rint(tp.t_line, c as c_uint, ptr(tp)) };
}

/// Allocate the input buffer: the `ttychars()` shim.
pub(crate) fn ttychars_init() {
    // SAFETY: called from kdinit() at SPLKD.
    unsafe { glue::ttychars(ptr(tty())) };
}

fn ptr(tp: &mut Tty) -> *mut c_void {
    (tp as *mut Tty).cast()
}

/// Open the console.  `kdopen()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdopen(
    dev: DevT,
    flag: c_int,
    ior: *mut IoReq,
) -> c_int {
    let tp = tty();
    // SAFETY: the tty lock is the driver's.
    let o_pri = unsafe { glue::kd_simple_lock_irq(lock()) };
    if tp.t_state & (TS_ISOPEN | TS_WOPEN) == 0 {
        // SAFETY: ttychars allocates the character buffers.
        unsafe { glue::kd_simple_unlock(lock()) };
        unsafe { glue::ttychars(ptr(tp)) };
        unsafe { glue::kd_simple_lock(lock()) };
        // Special support for boot-time rc scripts, which do not stty
        // the console.
        tp.t_start = Some(kdstart);
        tp.t_stop = Some(kdstop);
        tp.t_ospeed = B115200;
        tp.t_ispeed = B115200;
        tp.t_flags = KD_TTY_FLAGS;
        kdinit();
    }
    tp.t_state |= TS_CARR_ON;
    unsafe { glue::kd_simple_unlock_irq(o_pri, lock()) };
    // SAFETY: the request and tty are the caller's.
    unsafe { glue::char_open(dev as c_int, ptr(tp), flag, ior.cast()) }
}

/// Close the console.  `kdclose()` in C.
///
/// # Safety
///
/// The device layer calls this for an open console.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdclose(_dev: DevT, _flag: c_int) {
    let tp = tty();
    // SAFETY: the tty lock is the driver's.
    let s = unsafe { glue::kd_simple_lock_irq(lock()) };
    unsafe { glue::ttyclose(ptr(tp)) };
    unsafe { glue::kd_simple_unlock_irq(s, lock()) };
}

/// Read from the console.  `kdread()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdread(_dev: DevT, uio: *mut IoReq) -> c_int {
    let tp = tty();
    tp.t_state |= TS_CARR_ON;
    // SAFETY: the line discipline is the tty layer's.
    unsafe { glue::kd_ldisc_read(tp.t_line, ptr(tp), uio.cast()) }
}

/// Write to the console.  `kdwrite()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdwrite(_dev: DevT, uio: *mut IoReq) -> c_int {
    let tp = tty();
    // SAFETY: the line discipline is the tty layer's.
    unsafe { glue::kd_ldisc_write(tp.t_line, ptr(tp), uio.cast()) }
}

/// Map the bitmap frame buffer.  `kdmmap()` in C.
///
/// # Safety
///
/// The device layer calls this for /dev/console mappings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdmmap(
    _dev: DevT,
    off: usize,
    _prot: c_int,
) -> usize {
    if off >= MAP_LIMIT {
        return MAP_FAILED;
    }
    // i386_btop(): shift by I386_PGSHIFT.
    let base = super::kd().bitmap_start;
    (base.wrapping_add(off)) >> 12
}

/// Clean up reply ports.  `kdportdeath()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdportdeath(dev: DevT, port: u32) -> c_int {
    let _ = dev;
    // SAFETY: the tty layer owns the request queues.
    unsafe { glue::tty_portdeath(ptr(tty()), port as usize as *mut c_void) }
}

/// Device status query.  `kdgetstat()` in C.
///
/// # Safety
///
/// The device layer calls this with `data` holding `*count` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdgetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut u32,
) -> c_int {
    if flavor == KDGSTATE {
        if unsafe { *count } < 1 {
            return D_INVALID_OPERATION;
        }
        unsafe {
            *data = super::kd().state_bits();
            *count = 1;
        }
        D_SUCCESS
    } else if flavor == KDGKBENT {
        // SAFETY: the caller passes a `struct kbentry`.
        let kb = unsafe { &mut *data.cast::<super::KbEntry>() };
        super::keyboard::entry_get(kb);
        unsafe { *count = 1 };
        D_SUCCESS
    } else {
        // SAFETY: the tty layer handles its own flavors.
        unsafe { glue::tty_get_status(ptr(tty()), flavor, data, count) }
    }
}

/// Device status set.  `kdsetstat()` in C.
///
/// # Safety
///
/// The device layer calls this with `data` holding `count` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdsetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: u32,
) -> c_int {
    if flavor == KDSKBENT {
        if count < 1 {
            return D_INVALID_OPERATION;
        }
        // SAFETY: the caller passes a `struct kbentry`.
        let kb = unsafe { &*data.cast::<super::KbEntry>() };
        super::keyboard::entry_set(kb);
        D_SUCCESS
    } else if flavor == KDSETBELL {
        if count < 1 {
            return D_INVALID_OPERATION;
        }
        // SAFETY: one integer behind `data`.
        let val = unsafe { *data };
        super::console::set_bell(val, 0)
    } else {
        // SAFETY: the tty layer handles its own flavors.
        unsafe { glue::tty_set_status(ptr(tty()), flavor, data, count) }
    }
}

/// Start output.  `kdstart()` in C; the tty layer calls this at
/// `spltty`.
unsafe extern "C" fn kdstart(tp: *mut Tty) {
    // SAFETY: the tty layer passes the driver's own tty.
    let tp = unsafe { &mut *tp };
    if tp.t_state & TS_TTSTOP != 0 {
        return;
    }
    loop {
        tp.t_state &= !TS_BUSY;
        if tp.t_state & TS_TTSTOP != 0 {
            break;
        }
        let ch = if tp.t_outq.c_cc <= 0 {
            -1
        } else {
            // SAFETY: `t_outq` is the tty's output buffer.
            unsafe { glue::getc(outq()) }
        };
        if ch == -1 {
            break;
        }
        // Drop priority for long screen updates.
        // SAFETY: the clock's soft interrupt level is the driver's.
        let o_pri = unsafe { glue::splsoftclock() };
        super::esc::putc_esc(ch as u8);
        unsafe { glue::splx(o_pri) };
    }
    // SAFETY: `ttlowat[]` is the tty layer's.
    let lowat = unsafe { glue::kd_ttlowat(tp.t_ospeed as c_int) };
    if tp.t_outq.c_cc <= lowat {
        // tt_write_wakeup(tp)
        // SAFETY: the delayed write queue is the tty's.
        unsafe {
            glue::tty_queue_completion(
                core::ptr::addr_of_mut!(tp.t_delayed_write).cast(),
            )
        };
    }
}

/// Stop output: nothing to do.  `kdstop()` in C.
unsafe extern "C" fn kdstop(_tp: *mut Tty, _flags: c_int) {}
