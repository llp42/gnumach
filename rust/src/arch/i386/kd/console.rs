// SPDX-License-Identifier: BSD-2-Clause
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

/// `KDGSTATE`-style device returns.
const D_INVALID_OPERATION: c_int = 2505;

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
    // SAFETY: the console layer calls this at boot.
    unsafe { kdinit() };
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
            let c = unsafe { kdcnmaygetc() };
            if c >= 0 {
                return c;
            }
        }
    } else {
        unsafe { kdcnmaygetc() }
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
        unsafe { super::esc::kd_putc(b'\r') };
    }
    unsafe { super::esc::kd_putc_esc(c as u8) };
    0
}

/// Polled keyboard getc, ignoring caps lock.  `kdcnmaygetc()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD` (interrupts are usually off in the
/// debugger).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcnmaygetc() -> c_int {
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
        if unsafe { glue::pio_inb(K_STATUS) } & 0x20 == 0x20 {
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
            // SAFETY: the caller holds SPLKD.
            unsafe { super::keyboard::kd_resend() };
            continue;
        } else if scancode == K_ACKSC {
            // SAFETY: a literal format with no arguments.
            unsafe { glue::printf(c"cngetc: handle_ack".as_ptr()) };
            // SAFETY: the caller holds SPLKD.
            unsafe { super::keyboard::kd_handle_ack() };
            continue;
        }
        if scancode & K_UP != 0 {
            up = true;
            scancode &= !K_UP;
        }
        if state().kd_kbd_mouse != 0 {
            // SAFETY: the caller holds SPLKD.
            unsafe { super::keyboard::kd_kbd_magic(scancode as c_int) };
        }
        if (scancode as usize) < NUMKEYS {
            // Look up in the map, then process.
            // SAFETY: the caller holds SPLKD.
            let mut char_idx = unsafe {
                super::keyboard::kdstate2idx(
                    kd_state as c_uint,
                    state().kd_extended as c_int,
                )
            } as usize;
            let mut c = unsafe { KEY_MAP[scancode as usize][char_idx] };
            if c == K_SCAN {
                char_idx += 1;
                c = unsafe { KEY_MAP[scancode as usize][char_idx] };
                // SAFETY: the caller holds SPLKD.
                let st = unsafe {
                    super::keyboard::do_modifier(kd_state, c, up as c_int)
                };
                unsafe { kd_state = st };
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

/// `kdsetbell()` in C: turn the bell on or off.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdsetbell(val: c_int, _flags: c_int) -> c_int {
    if val == KD_BELLON {
        // SAFETY: the caller holds SPLKD.
        unsafe { super::kd_bellon() };
        0
    } else if val == KD_BELLOFF {
        // SAFETY: the caller holds SPLKD.
        unsafe { super::kd_belloff(core::ptr::null_mut()) };
        0
    } else {
        D_INVALID_OPERATION
    }
}
