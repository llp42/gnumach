// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/kd.c and i386/i386at/kd.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
//   Copyright 1988, 1989 by Intel Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct tty` and the kd device entry points: open/close/read/write,
//! get/set status, mmap and the line-discipline start.
//!
//! The mirror is `#[repr(C)]` and its offsets are pinned below, because
//! the C tty layer (`device/chario.c`) reads and writes the same bytes;
//! `kd_tty` itself is Rust storage now.  `t_lock` is taken through the
//! Rust [`SimpleLock`], and the line-discipline switch and
//! `ttlowat[]` are read straight out of the C statics that
//! `device/chario.c` builds.

use super::*;
use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::vm_param::PAGE_SHIFT;
use crate::device::chario::tty_queue_completion;
use crate::device::cirbuf::Cirbuf;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResultExt};
use crate::glue;
use crate::kern::lock::SimpleLock;
use crate::kern::queue::QueueEntry;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::NonNull;

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

/// `kdmmap()` refuses offsets past this.
const MAP_LIMIT: usize = 128 * 1024;
/// `kdmmap()`'s failure value, as `(vm_offset_t)-1`.
const MAP_FAILED: usize = usize::MAX;

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
            t_lock: SimpleLock::new(),
            t_inq: Cirbuf::new(),
            t_outq: Cirbuf::new(),
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

/// The line discipline `tp.t_line` names, or [`None`] when the tty
/// names one this kernel does not have.
///
/// `t_line` is the "fake line discipline number" of <device/tty.h>
/// and is zero on every tty here, so `linesw[]`'s single entry always
/// answers; the fallible form is what keeps the index inside it.
fn ldisc(tp: &Tty) -> Option<&'static glue::LdiscSwitch> {
    let line = usize::try_from(tp.t_line).ok()?;
    // SAFETY: `linesw` is a C static that device/chario.c's
    // initializer builds before any device is open and nothing
    // writes afterwards, so a shared reference outlives the kernel.
    unsafe { glue::linesw.get(line) }
}

/// Feed one character to the line discipline.
pub(crate) fn line_rint(c: u8) {
    let tp = tty();
    let Some(rint) = ldisc(tp).and_then(|d| d.l_rint) else {
        return;
    };
    // SAFETY: the discipline is device/chario.c's `ttyinput()`, and
    // the tty is up once the console is open.
    unsafe { rint(c_uint::from(c), ptr(tp)) };
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
    // SAFETY: `splhigh()` is the asm entry of <machine/spl.h>.  It
    // and the lock below are the two halves of the C
    // `simple_lock_irq()` macro, in its order.
    let o_pri = unsafe { glue::splhigh() };
    tp.t_lock.lock();
    if tp.t_state & (TS_ISOPEN | TS_WOPEN) == 0 {
        tp.t_lock.unlock();
        // SAFETY: ttychars allocates the character buffers, and must
        // not run under the tty lock.
        unsafe { glue::ttychars(ptr(tp)) };
        tp.t_lock.lock();
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
    tp.t_lock.unlock();
    // SAFETY: `o_pri` is the level `splhigh()` returned above.
    unsafe { glue::splx(o_pri) };
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
    // SAFETY: `splhigh()` is the asm entry of <machine/spl.h>; the
    // tty lock is taken at that level, as `simple_lock_irq()` did.
    let s = unsafe { glue::splhigh() };
    tp.t_lock.lock();
    // SAFETY: the tty is the driver's own.
    unsafe { glue::ttyclose(ptr(tp)) };
    tp.t_lock.unlock();
    // SAFETY: `s` is the level `splhigh()` returned above.
    unsafe { glue::splx(s) };
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
    let Some(read) = ldisc(tp).and_then(|d| d.l_read) else {
        return Err(DeviceError::InvalidOperation).as_io_return();
    };
    // SAFETY: the discipline is device/chario.c's `char_read()`, and
    // the tty and the request are the device layer's.
    unsafe { read(ptr(tp), uio.cast()) }
}

/// Write to the console.  `kdwrite()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdwrite(_dev: DevT, uio: *mut IoReq) -> c_int {
    let tp = tty();
    let Some(write) = ldisc(tp).and_then(|d| d.l_write) else {
        return Err(DeviceError::InvalidOperation).as_io_return();
    };
    // SAFETY: the discipline is device/chario.c's `char_write()`, and
    // the tty and the request are the device layer's.
    unsafe { write(ptr(tp), uio.cast()) }
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
    (base.wrapping_add(off)) >> PAGE_SHIFT
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
            return Err(DeviceError::InvalidOperation).as_io_return();
        }
        unsafe {
            *data = super::kd().state_bits();
            *count = 1;
        }
        Ok(DeviceSuccess::Success).as_io_return()
    } else if flavor == KDGKBENT {
        // SAFETY: the caller passes a `struct kbentry`.
        let kb = unsafe { &mut *data.cast::<super::KbEntry>() };
        super::keyboard::entry_get(kb);
        unsafe { *count = 1 };
        Ok(DeviceSuccess::Success).as_io_return()
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
            return Err(DeviceError::InvalidOperation).as_io_return();
        }
        // SAFETY: the caller passes a `struct kbentry`.
        let kb = unsafe { &*data.cast::<super::KbEntry>() };
        super::keyboard::entry_set(kb);
        Ok(DeviceSuccess::Success).as_io_return()
    } else if flavor == KDSETBELL {
        if count < 1 {
            return Err(DeviceError::InvalidOperation).as_io_return();
        }
        // SAFETY: one integer behind `data`.
        let val = unsafe { *data };
        super::console::set_bell(val, 0).as_io_return()
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
        let Some(ch) = tp.t_outq.get() else {
            break;
        };
        // Drop priority for long screen updates.
        // SAFETY: the clock's soft interrupt level is the driver's.
        let o_pri = unsafe { glue::splsoftclock() };
        super::esc::putc_esc(ch);
        unsafe { glue::splx(o_pri) };
    }
    // SAFETY: `ttlowat[]` is a C static of `NSPEEDS` shorts, written
    // only by device/chario.c's initializer.
    let lowat = match unsafe { glue::ttlowat.get(usize::from(tp.t_ospeed)) } {
        Some(&w) => w,
        // `tty_set_status()` rejects a speed past `NSPEEDS`, so no
        // tty reaches this; a zero mark wakes the writer at once.
        None => 0,
    };
    if tp.t_outq.count() <= lowat {
        // tt_write_wakeup(tp)
        // SAFETY: the delayed write queue is the tty's and stays at
        // its address; `kdstart` runs at spltty with the tty lock
        // held, so nothing else touches the queue.
        unsafe {
            tty_queue_completion(core::ptr::addr_of_mut!(tp.t_delayed_write))
        };
    }
}

/// Stop output: nothing to do.  `kdstop()` in C.
unsafe extern "C" fn kdstop(_tp: *mut Tty, _flags: c_int) {}
