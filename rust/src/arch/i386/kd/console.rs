// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/kd.c and i386/i386at/kd.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
//   Copyright 1988, 1989 by Intel Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kd console entry points, which <device/cons.c> calls through
//! `constab`: probe/init and the polled getc/putc the kernel debugger
//! uses, plus the bell ioctl.

use super::keymap::KEY_MAP;
use super::*;
use crate::glue;
use core::ffi::{c_int, c_uint};

/// `KD_BELLON`/`KD_BELLOFF` of <i386at/kd.h>.
const KD_BELLON: c_int = 1;
const KD_BELLOFF: c_int = 0;

use crate::arch::i386::io_req::D_INVALID_OPERATION;

/// Probe the console.  `kdcnprobe()` in C.
///
/// # Safety
///
/// `cp` is the console table's entry; the hardware is assumed present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcnprobe(cp: *mut ConsDev) -> c_int {
    let cp = unsafe { &mut *cp };
    // makedev(0, 0)
    cp.cn_dev = 0;
    cp.cn_pri = CN_INTERNAL;
    0
}

/// Initialize the console.  `kdcninit()` in C.
///
/// # Safety
///
/// Called once from `cninit()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcninit(_cp: *mut ConsDev) -> c_int {
    kdinit();
    0
}

/// Polled console getc.  `kdcngetc()` in C.
///
/// # Safety
///
/// The caller must hold the console lock and interrupts must be off
/// while the debugger polls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcngetc(_dev: u16, wait: c_int) -> c_int {
    if wait != 0 {
        loop {
            let c = maygetc();
            if c >= 0 {
                return c;
            }
        }
    } else {
        maygetc()
    }
}

/// Console putc.  `kdcnputc()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcnputc(_dev: u16, c: c_int) -> c_int {
    if !state().kd_initialized {
        return -1;
    }
    // Tab is handled in kd_putc.
    if c == b'\n' as c_int {
        super::esc::putc(b'\r');
    }
    super::esc::putc_esc(c as u8);
    0
}

/// Polled keyboard getc, ignoring caps lock.  `kdcnmaygetc()` in C.
/// The caller must hold `SPLKD` (interrupts are usually off in the
/// debugger).
pub(crate) fn maygetc() -> c_int {
    if !state().kd_initialized {
        return -1;
    }
    state().kd_extended = false;

    loop {
        if unsafe { glue::pio_inb(K_STATUS) } & K_OBUF_FUL == 0 {
            return -1;
        }

        let mut up = false;
        // We would come here for mouse events in the debugger.
        if unsafe { glue::pio_inb(K_STATUS) } & K_AUX_OBUF_FUL
            == K_AUX_OBUF_FUL
        {
            let sc = unsafe { glue::pio_inb(K_RDWR) };
            // SAFETY: a literal format with one integer.
            unsafe { glue::printf(c"M%xP".as_ptr(), sc as c_int) };
            continue;
        }
        let mut scancode = unsafe { glue::pio_inb(K_RDWR) };
        // Handle the extend modifier and ack/resend, or a key may never
        // arrive.
        if scancode == K_EXTEND {
            state().kd_extended = true;
            continue;
        } else if scancode == K_RESEND {
            // SAFETY: a literal format with no arguments.
            unsafe { glue::printf(c"cngetc: resend".as_ptr()) };
            super::keyboard::resend();
            continue;
        } else if scancode == K_ACKSC {
            // SAFETY: a literal format with no arguments.
            unsafe { glue::printf(c"cngetc: handle_ack".as_ptr()) };
            super::keyboard::handle_ack();
            continue;
        }
        if scancode & K_UP != 0 {
            up = true;
            scancode &= !K_UP;
        }
        if state().kd_kbd_mouse != 0 {
            super::keyboard::kbd_magic(scancode as c_int);
        }
        if (scancode as usize) < NUMKEYS {
            // Look up in the map, then process.
            let mut char_idx = super::keyboard::state2idx(
                super::kd().state_bits() as c_uint,
                state().kd_extended,
            );
            let mut c = unsafe { KEY_MAP[scancode as usize][char_idx] };
            if c == K_SCAN {
                char_idx += 1;
                c = unsafe { KEY_MAP[scancode as usize][char_idx] };
                let st =
                    super::keyboard::modifier(super::kd().state_bits(), c, up);
                super::kd().set_state_bits(st);
            } else if !up
                && c == K_ESC
                && unsafe { KEY_MAP[scancode as usize][char_idx + 1] } == 0x5b
            {
                // Remap some keys to the readline-like shortcuts the
                // debugger supports.
                c = unsafe { KEY_MAP[scancode as usize][char_idx + 2] };
                return match c {
                    0x48 => 0x01, // home
                    0x41 => 0x10, // up
                    0x44 => 0x02, // left
                    0x43 => 0x06, // right
                    0x42 => 0x0e, // down
                    0x59 => 0x05, // end
                    0x39 => 0x04, // delete
                    _ => K_ESC as c_int,
                };
            } else if !up {
                // A regular key-down.
                if c == K_CR {
                    c = K_LF;
                }
                return c as c_int & 0o177;
            }
        }
    }
}

/// `kdsetbell()` in C: turn the bell on or off.  The caller must hold
/// `SPLKD`.
pub(crate) fn set_bell(val: c_int, _flags: c_int) -> c_int {
    if val == KD_BELLON {
        super::kd_bellon();
        0
    } else if val == KD_BELLOFF {
        // SAFETY: the timeout callback is the driver's.
        unsafe { super::kd_belloff(core::ptr::null_mut()) };
        0
    } else {
        D_INVALID_OPERATION
    }
}
