// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The EGA-style kd display backend: the text write and cursor, and
//! the screen block moves (through the `kd_slm*` assembly).  The
//! bitmap backend was never selected by the ported `kdinit()` and had
//! no C callers, so it is gone, along with `kdsoft.h`'s pointer table.

use super::*;
use crate::glue;
use core::ffi::{c_char, c_int, c_short};

/// The CRT cursor shape scanlines the firmware default is replaced
/// with when it left them zero.
const CURSOR_START_SCANLINE: u8 = 14;
const CURSOR_STOP_SCANLINE: u8 = 15;
/// Bytes of the bitmap the C cleared at initialization.
const BITMAP_CLEAR_BYTES: usize = 200;

// Safe operations for the escape engine.

pub(crate) fn dput(pos: c_short, ch: u8, attr: u8) {
    // SAFETY: the screen is mapped and SPLKD is held.
    unsafe { text_put(pos, ch as c_char, attr as c_char) };
}

pub(crate) fn dclear(to: c_short, count: c_int, attr: u8) {
    // SAFETY: as above.
    unsafe { text_clear(to, count, attr as c_char) };
}

pub(crate) fn dmvup(from: c_short, to: c_short, count: c_int) {
    // SAFETY: as above.
    unsafe { move_up(from, to, count) };
}

pub(crate) fn dmvdown(from: c_short, to: c_short, count: c_int) {
    // SAFETY: as above.
    unsafe { move_down(from, to, count) };
}

pub(crate) fn setpos(newpos: c_short) {
    let mut newpos = newpos;
    if newpos > ONE_PAGE {
        scrollup();
        newpos = BOTTOM_LINE;
    }
    if newpos < 0 {
        scrolldn();
        newpos = 0;
    }
    // SAFETY: the CRTC is the driver's and SPLKD is held.
    unsafe { set_cursor(newpos) };
}

pub(crate) fn scrollup() {
    let count = (ONE_PAGE - ONE_LINE) / ONE_SPACE;
    dmvup(ONE_LINE, 0, count as c_int);
    dclear(
        BOTTOM_LINE,
        (ONE_LINE / ONE_SPACE) as c_int,
        state().kd_attr,
    );
}

pub(crate) fn scrolldn() {
    let to = ONE_PAGE - ONE_SPACE;
    let from = ONE_PAGE - ONE_LINE - ONE_SPACE;
    let count = (ONE_PAGE - ONE_LINE) / ONE_SPACE;
    dmvdown(from, to, count as c_int);
    dclear(0, (ONE_LINE / ONE_SPACE) as c_int, state().kd_attr);
}

/// Put an attributed character for EGA/CGA.  `text_put()` in C.
unsafe fn text_put(pos: c_short, ch: c_char, chattr: c_char) {
    let s = state();
    // SAFETY: `vid_start` is the mapped screen and `pos` is in range.
    unsafe {
        *s.vid_start.add(pos as usize) = ch as u8;
        *s.vid_start.add(pos as usize + 1) = chattr as u8;
    }
}

/// Set the hardware cursor for EGA/CGA.  `set_cursor()` in C.
unsafe fn set_cursor(newpos: c_short) {
    let curpos = newpos / ONE_SPACE;
    let s = state();
    // SAFETY: the CRTC index/data pair is the driver's.
    unsafe {
        glue::pio_outb(s.kd_index_reg as u16, C_HIGH);
        glue::pio_outb(s.kd_io_reg as u16, (curpos >> 8) as u8);
        glue::pio_outb(s.kd_index_reg as u16, C_LOW);
        glue::pio_outb(s.kd_io_reg as u16, (curpos & 0xff) as u8);
    }
    s.kd_curpos = newpos;
}

/// Block move up for EGA/CGA.  `move_up()` in C.
unsafe fn move_up(from: c_short, to: c_short, count: c_int) {
    let s = state();
    // SAFETY: both offsets are inside the screen.
    unsafe {
        glue::kd_slmscu(
            s.vid_start.add(from as usize).cast(),
            s.vid_start.add(to as usize).cast(),
            count,
        )
    };
}

/// Block move down for EGA/CGA.  `move_down()` in C.
unsafe fn move_down(from: c_short, to: c_short, count: c_int) {
    let s = state();
    // SAFETY: both offsets are inside the screen.
    unsafe {
        glue::kd_slmscd(
            s.vid_start.add(from as usize).cast(),
            s.vid_start.add(to as usize).cast(),
            count,
        )
    };
}

/// Fast clear for EGA/CGA.  `text_clear()` in C.
unsafe fn text_clear(to: c_short, count: c_int, chattr: c_char) {
    let s = state();
    let value = (((chattr as u8) as c_int) << 8) + K_SPACE as c_int;
    // SAFETY: the offset is inside the screen.
    unsafe {
        glue::kd_slmwd(s.vid_start.add(to as usize).cast(), count, value)
    };
}

/// No-op reset.  `noop_reset()` in C.
unsafe fn noop_reset() {}

/// Prepare the display for reboot.  The EGA backend needs nothing.
pub(crate) fn reset() {
    // SAFETY: resetting has no preconditions.
    unsafe { noop_reset() };
}

/// `phystokv()` of <i386/i386/vm_param.h>.
fn phystokv(addr: usize) -> usize {
    #[cfg(target_pointer_width = "64")]
    const BASE: usize = 0xffff_ffff_8000_0000;
    #[cfg(target_pointer_width = "32")]
    const BASE: usize = 0xc000_0000;
    addr.wrapping_add(BASE)
}

/// The current hardware cursor position.  `get_cursor()` in C.
fn get_cursor() -> c_short {
    let s = state();
    // SAFETY: the CRTC index/data pair is the driver's.
    unsafe {
        glue::pio_outb(s.kd_index_reg as u16, C_HIGH);
        let high = glue::pio_inb(s.kd_io_reg as u16);
        glue::pio_outb(s.kd_index_reg as u16, C_LOW);
        let low = glue::pio_inb(s.kd_io_reg as u16);
        let pos = (low as u16) | ((high as u16) << 8);
        ONE_SPACE * pos as c_short
    }
}

/// Initialize the character-based graphics adapter.  `kd_xga_init()` in
/// C; called once, from `kdinit()`.
pub(crate) fn xga_init() {
    {
        let s = state();
        s.vid_start = phystokv(EGA_START) as *mut u8;
        s.kd_index_reg = EGA_IDX_REG as c_short;
        s.kd_io_reg = EGA_IO_REG as c_short;
        s.kd_lines = 25;
        s.kd_cols = 80;
        // Clear the first 200 bytes of the bitmap.
        let addr = phystokv(C_BITMAP_START) as *mut u8;
        // SAFETY: the bitmap base is mapped by the boot.
        unsafe { core::ptr::write_bytes(addr, 0, BITMAP_CLEAR_BYTES) };
    }

    let mut start: u8;
    let stop: u8;
    // SAFETY: the CRTC index/data pair is the driver's.
    unsafe {
        let s = state();
        glue::pio_outb(s.kd_index_reg as u16, C_START);
        start = glue::pio_inb(s.kd_io_reg as u16);
        // Make sure the cursor is enabled.
        start &= !0x20;
        glue::pio_outb(s.kd_io_reg as u16, start);
        glue::pio_outb(s.kd_index_reg as u16, C_STOP);
        stop = glue::pio_inb(s.kd_io_reg as u16);
    }

    if start == 0 && stop == 0 {
        // Some firmware leaves the cursor size unset; use standards.
        // SAFETY: as above.
        unsafe {
            let s = state();
            glue::pio_outb(s.kd_index_reg as u16, C_START);
            glue::pio_outb(s.kd_io_reg as u16, CURSOR_START_SCANLINE);
            glue::pio_outb(s.kd_index_reg as u16, C_STOP);
            glue::pio_outb(s.kd_io_reg as u16, CURSOR_STOP_SCANLINE);
        }
    }

    setpos(get_cursor());
}
