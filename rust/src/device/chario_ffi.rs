// SPDX-License-Identifier: CMU-Mach
// Derived from device/chario.c, device/tty.h and include/device/tty_status.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
//   Copyright (c) 1993-1990 Carnegie Mellon University.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `device/chario.c`, which the C device drivers
//! and the Rust console driver call.
//!
//! Every adapter here hands its raw arguments to the matching core in
//! [`chario`] and converts the answer back.

use crate::arch::i386::io_req::{IoDone, IoReq};
use crate::device::chario::{
    self, D_READ, D_WRITE, LdiscSwitch, Tty, TtyStatus, io_return,
};
use crate::device::r#return::DeviceError;
use crate::kern::queue::QueueEntry;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::slice;

/// `TTY_STATUS` of <device/tty_status.h>: `('t'<<16) + 1`.
const TTY_STATUS: c_uint = 0x0074_0001;
/// `TTY_STATUS_COUNT`: the four integers of `struct tty_status`.
const TTY_STATUS_COUNT: u32 = 4;
/// `TTY_FLUSH` of <device/tty_status.h>: `('t'<<16) + 3`.
const TTY_FLUSH: c_uint = 0x0074_0003;
/// `TTY_FLUSH_COUNT`: one flags integer.
const TTY_FLUSH_COUNT: u32 = 1;
/// `TTY_STOP` of <device/tty_status.h>: `('t'<<16) + 4`.
const TTY_STOP: c_uint = 0x0074_0004;
/// `TTY_START` of <device/tty_status.h>: `('t'<<16) + 5`.
const TTY_START: c_uint = 0x0074_0005;

/// `linesw[]` of <device/tty.h>: the fake line discipline the console
/// driver calls through.
#[unsafe(export_name = "linesw")]
pub(crate) static LINESW: [LdiscSwitch; 1] = [LdiscSwitch {
    l_read: Some(char_read),
    l_write: Some(char_write),
    l_rint: Some(ttyinput),
    l_modem: Some(ttymodem),
    l_start: Some(tty_output),
}];

/// `chario_init()` of device/chario.c.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during
/// boot before any tty is opened.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chario_init() {
    chario::chario_init();
}

/// `tty_queue_completion()` of device/chario.c.
///
/// # Safety
///
/// `queue` must be an initialized queue head that stays at its address while
/// entries are linked, every entry must be an `io_req`, and nothing else may
/// access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_queue_completion(queue: *mut QueueEntry) {
    // SAFETY: the caller's contract is the core's.
    unsafe { chario::complete_queue(queue) };
}

/// `char_open()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty and `ior` at a live open request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_open(
    dev: c_int,
    tp: *mut Tty,
    mode: c_uint,
    ior: *mut IoReq,
) -> c_int {
    // SAFETY: the caller promises the live tty and request.
    io_return(unsafe { chario::open(&mut *tp, dev, mode, &mut *ior) })
}

/// `char_open_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live open request `char_open()` queued on a tty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_open_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live queued request.
    unsafe { chario::char_open_done(ior) }
}

/// `char_write()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty and `ior` at a live write request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_write(tp: *mut Tty, ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live tty and request.
    io_return(unsafe { chario::write(&mut *tp, &mut *ior) })
}

/// `char_write_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live write request `char_write()` queued on a tty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_write_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live queued request.
    unsafe { chario::char_write_done(ior) }
}

/// `char_read()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty and `ior` at a live read request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_read(tp: *mut Tty, ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live tty and request.
    io_return(unsafe { chario::read(&mut *tp, &mut *ior) })
}

/// `char_read_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live read request `char_read()` queued on a tty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_read_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live queued request.
    unsafe { chario::char_read_done(ior) }
}

/// `ttyclose()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttyclose(tp: *mut Tty) {
    // SAFETY: the caller promises the live, locked tty.
    chario::close(unsafe { &mut *tp });
}

/// `tty_portdeath()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty and `port` is the reply port that died.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_portdeath(
    tp: *mut Tty,
    port: *mut c_void,
) -> c_int {
    // SAFETY: the caller promises the live tty.
    c_int::from(chario::port_death(unsafe { &mut *tp }, port))
}

/// `tty_get_status()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty; `data` must be writable for `*count`
/// integers, and `count` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_get_status(
    tp: *mut Tty,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut u32,
) -> c_int {
    match flavor {
        TTY_STATUS => {
            // SAFETY: the caller promises `count` writable.
            if unsafe { *count } < TTY_STATUS_COUNT {
                return DeviceError::InvalidOperation as c_int;
            }
            // SAFETY: the caller promises a live tty.
            let status = chario::status(unsafe { &*tp });
            // SAFETY: the caller promises four writable integers at `data`,
            // the `struct tty_status` the C filled field by field.
            unsafe { data.cast::<TtyStatus>().write(status) };
            // SAFETY: as the count check above.
            unsafe { *count = TTY_STATUS_COUNT };
            0
        }
        _ => DeviceError::InvalidOperation as c_int,
    }
}

/// `tty_set_status()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and `data` must be readable for `count`
/// integers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_set_status(
    tp: *mut Tty,
    flavor: c_uint,
    data: *mut c_int,
    count: u32,
) -> c_int {
    match flavor {
        TTY_FLUSH => {
            if count < TTY_FLUSH_COUNT {
                return DeviceError::InvalidOperation as c_int;
            }
            // SAFETY: the caller promises one readable integer.
            let flags = unsafe { *data };
            let flags = if flags == 0 { D_READ | D_WRITE } else { flags };
            // SAFETY: the caller promises the live tty.
            chario::set_flush(unsafe { &mut *tp }, flags);
            0
        }
        TTY_STOP => {
            // SAFETY: the caller promises the live tty.
            chario::stop_output(unsafe { &mut *tp });
            0
        }
        TTY_START => {
            // SAFETY: the caller promises the live tty.
            chario::start_output(unsafe { &mut *tp });
            0
        }
        TTY_STATUS => {
            if count < TTY_STATUS_COUNT {
                return DeviceError::InvalidOperation as c_int;
            }
            // SAFETY: the caller promises four readable integers.
            let status = unsafe { data.cast::<TtyStatus>().read() };
            // SAFETY: the caller promises the live tty.
            match chario::apply_status(unsafe { &mut *tp }, &status) {
                Ok(()) => 0,
                Err(error) => error as c_int,
            }
        }
        _ => DeviceError::InvalidOperation as c_int,
    }
}

/// `queue_delayed_reply()` of device/chario.c.
///
/// # Safety
///
/// `qh` must be an initialized queue head that stays at its address, `ior`
/// must stay at its address until the queue completes it, and the caller must
/// hold the tty lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn queue_delayed_reply(
    qh: *mut QueueEntry,
    ior: *mut IoReq,
    io_done: IoDone,
) {
    // SAFETY: the caller promises the live request.
    let ior = unsafe { &mut *ior };
    // SAFETY: the caller's contract is the core's.
    unsafe { chario::queue_delayed_reply(qh, ior, io_done) };
}

/// `ttychars()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and nothing else may use the tty during the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttychars(tp: *mut Tty) {
    // SAFETY: the caller promises the live tty.
    chario::chars(unsafe { &mut *tp });
}

/// `tty_flush()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_flush(tp: *mut Tty, rw: c_int) {
    // SAFETY: the caller promises the live, locked tty.
    chario::flush(unsafe { &mut *tp }, rw);
}

/// `ttrstrt()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttrstrt(tp: *mut Tty) {
    // SAFETY: the caller promises the live tty.
    chario::restart(unsafe { &mut *tp });
}

/// `ttstart()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttstart(tp: *mut Tty) {
    // SAFETY: the caller promises the live, locked tty.
    chario::start(unsafe { &mut *tp });
}

/// `tty_output()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_output(tp: *mut Tty) {
    // SAFETY: the caller promises the live, locked tty.
    chario::start(unsafe { &mut *tp });
}

/// `ttyinput()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttyinput(c: c_uint, tp: *mut Tty) {
    // SAFETY: the caller promises the live, locked tty.
    chario::input(unsafe { &mut *tp }, c);
}

/// `ttyinput_many()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty whose lock the caller holds, and `chars`
/// must be readable for `count` bytes when it is positive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttyinput_many(
    tp: *mut Tty,
    chars: *mut c_char,
    count: c_int,
) {
    let chars = if count <= 0 {
        &[]
    } else {
        // SAFETY: the caller promises `count` readable bytes at `chars`.
        unsafe { slice::from_raw_parts(chars.cast::<u8>(), count as usize) }
    };
    // SAFETY: the caller promises the live, locked tty.
    chario::input_many(unsafe { &mut *tp }, chars);
}

/// `ttymodem()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty, and the caller must hold its lock at
/// `spltty`, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttymodem(tp: *mut Tty, carrier_up: c_int) -> c_int {
    // SAFETY: the caller promises the live, locked tty.
    c_int::from(chario::modem(unsafe { &mut *tp }, carrier_up != 0))
}

/// `tty_cts()` of device/chario.c.
///
/// # Safety
///
/// `tp` must point at a live tty whose lock the caller holds, on the master
/// CPU, as the C contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_cts(tp: *mut Tty, cts_up: c_int) {
    // SAFETY: the caller promises the live, locked tty.
    chario::cts(unsafe { &mut *tp }, cts_up != 0);
}
