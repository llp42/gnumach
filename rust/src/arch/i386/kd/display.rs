// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kd display backends: the EGA-style text write and cursor, the
//! screen block moves (through the `kd_slm*` assembly), and the bitmap
//! frame-buffer code kept for the cards that use it.
//!
//! `kdsoft.h`'s function-pointer table is exported as C data, so an
//! external backend can still replace these entries.

use super::*;
use crate::glue;
use core::ffi::{c_char, c_int, c_short};

type Dput = unsafe extern "C" fn(c_short, c_char, c_char);
type Dclear = unsafe extern "C" fn(c_short, c_int, c_char);
type Dmv = unsafe extern "C" fn(c_short, c_short, c_int);
type Dsetcursor = unsafe extern "C" fn(c_short);
type Dreset = unsafe extern "C" fn();

/// `kd_dput` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dput: Dput = charput;
/// `kd_dmvup` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dmvup: Dmv = charmvup;
/// `kd_dmvdown` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dmvdown: Dmv = charmvdown;
/// `kd_dclear` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dclear: Dclear = charclear;
/// `kd_dsetcursor` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dsetcursor: Dsetcursor = charsetcursor;
/// `kd_dreset` of <i386at/kdsoft.h>.
#[unsafe(no_mangle)]
pub static mut kd_dreset: Dreset = kd_noopreset;

// Safe operations for the escape engine.

pub(crate) fn dput(pos: c_short, ch: u8, attr: u8) {
    // SAFETY: the table holds C entry points, and SPLKD is held.
    unsafe { (kd_dput)(pos, ch as c_char, attr as c_char) };
}

pub(crate) fn dclear(to: c_short, count: c_int, attr: u8) {
    // SAFETY: as above.
    unsafe { (kd_dclear)(to, count, attr as c_char) };
}

pub(crate) fn dmvup(from: c_short, to: c_short, count: c_int) {
    // SAFETY: as above.
    unsafe { (kd_dmvup)(from, to, count) };
}

pub(crate) fn dmvdown(from: c_short, to: c_short, count: c_int) {
    // SAFETY: as above.
    unsafe { (kd_dmvdown)(from, to, count) };
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
    // SAFETY: the table holds C entry points, and SPLKD is held.
    unsafe { (kd_dsetcursor)(newpos) };
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

/// Put an attributed character for EGA/CGA.  `charput()` in C.
unsafe extern "C" fn charput(pos: c_short, ch: c_char, chattr: c_char) {
    let s = state();
    // SAFETY: `vid_start` is the mapped screen and `pos` is in range.
    unsafe {
        *s.vid_start.add(pos as usize) = ch as u8;
        *s.vid_start.add(pos as usize + 1) = chattr as u8;
    }
}

/// Set the hardware cursor for EGA/CGA.  `charsetcursor()` in C.
unsafe extern "C" fn charsetcursor(newpos: c_short) {
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

/// Block move up for EGA/CGA.  `charmvup()` in C.
unsafe extern "C" fn charmvup(from: c_short, to: c_short, count: c_int) {
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

/// Block move down for EGA/CGA.  `charmvdown()` in C.
unsafe extern "C" fn charmvdown(from: c_short, to: c_short, count: c_int) {
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

/// Fast clear for EGA/CGA.  `charclear()` in C.
unsafe extern "C" fn charclear(to: c_short, count: c_int, chattr: c_char) {
    let s = state();
    let value = (((chattr as u8) as c_int) << 8) + K_SPACE as c_int;
    // SAFETY: the offset is inside the screen.
    unsafe {
        glue::kd_slmwd(s.vid_start.add(to as usize).cast(), count, value)
    };
}

/// No-op reset.  `kd_noopreset()` in C.
unsafe extern "C" fn kd_noopreset() {}

/// `phystokv()` of <i386/i386/vm_param.h>.
fn phystokv(addr: usize) -> usize {
    #[cfg(target_pointer_width = "64")]
    const BASE: usize = 0xffff_ffff_8000_0000;
    #[cfg(target_pointer_width = "32")]
    const BASE: usize = 0xc000_0000;
    addr.wrapping_add(BASE)
}

/// The current hardware cursor position.  `xga_getpos()` in C.
fn xga_getpos() -> c_short {
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
/// C.
///
/// # Safety
///
/// Called once, from `kdinit()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_xga_init() {
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
        unsafe { core::ptr::write_bytes(addr, 0, 200) };
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
            glue::pio_outb(s.kd_io_reg as u16, 14);
            glue::pio_outb(s.kd_index_reg as u16, C_STOP);
            glue::pio_outb(s.kd_io_reg as u16, 15);
        }
    }

    setpos(xga_getpos());
}

/// Set the cursor, scrolling if needed.  `kd_setpos()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_setpos(newpos: c_short) {
    setpos(newpos);
}

/// Scroll the screen up a line.  `kd_scrollup()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_scrollup() {
    scrollup();
}

/// Scroll the screen down a line.  `kd_scrolldn()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_scrolldn() {
    scrolldn();
}

// The bitmap backend.  Private safe helpers, exported C entry points.

/// Character position to bit addresses.  `bmpch2bit()` in C.
fn ch2bit(pos: c_short) -> (c_short, c_short) {
    let s = state();
    let xch = (pos / ONE_SPACE) % s.kd_cols;
    let ych = pos / (ONE_SPACE * s.kd_cols);
    (
        s.xstart + xch * s.char_width,
        s.ystart + ych * (s.char_height + s.cursor_height),
    )
}

/// The frame-buffer pointer for a bit address.  `bit2fbptr()` in C.
fn fbptr(xb: c_short, yb: c_short) -> *mut u8 {
    let s = state();
    s.vid_start.wrapping_add(
        yb as usize * s.fb_byte_width as usize + (xb / 8) as usize,
    )
}

/// Copy a character from the font to the frame buffer.
fn put(pos: c_short, ch: c_char, chattr: c_char) {
    let s = state();
    let mut ch = ch as u8;
    if ch as c_short >= s.chars_in_font {
        ch = K_QUES;
    }
    let mask = if chattr as u8 == KA_REVERSE { 0xff } else { 0 };
    let (xbit, ybit) = ch2bit(pos);
    // SAFETY: the bitmap backend set the font and frame buffer up.
    unsafe {
        let mut to = fbptr(xbit, ybit);
        let mut from =
            s.font_start.add(ch as usize * s.char_byte_width as usize);
        for _ in 0..s.char_height {
            for j in 0..s.char_byte_width {
                *to.add(j as usize) = *from.add(j as usize) ^ mask;
            }
            to = to.add(s.fb_byte_width as usize);
            from = from.add(s.font_byte_width as usize);
        }
    }
}

/// Copy one character within the frame buffer.  `bmpcp1char()` in C.
fn cp1char(from: c_short, to: c_short) {
    let s = state();
    let (from_xbit, from_ybit) = ch2bit(from);
    let (to_xbit, to_ybit) = ch2bit(to);
    // SAFETY: the bitmap backend set the frame buffer up.
    unsafe {
        let mut tp = fbptr(to_xbit, to_ybit);
        let mut fp = fbptr(from_xbit, from_ybit);
        for _ in 0..s.char_height {
            for j in 0..s.char_byte_width {
                *tp.add(j as usize) = *fp.add(j as usize);
            }
            tp = tp.add(s.fb_byte_width as usize);
            fp = fp.add(s.fb_byte_width as usize);
        }
    }
}

/// Paint the cursor bits.  `bmppaintcsr()` in C.
fn paintcsr(pos: c_short, val: u8) {
    let s = state();
    let (xbit, mut ybit) = ch2bit(pos);
    ybit += s.char_height;
    // SAFETY: the cursor block is inside the frame buffer.
    unsafe {
        let mut cp = fbptr(xbit, ybit);
        for _ in 0..s.cursor_height {
            for byte in 0..s.char_byte_width {
                *cp.add(byte as usize) = val;
            }
            cp = cp.add(s.fb_byte_width as usize);
        }
    }
}

/// Copy a character from the font to the frame buffer.  `bmpput()` in
/// C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpput(pos: c_short, ch: c_char, chattr: c_char) {
    put(pos, ch, chattr);
}

/// Copy a block of characters up.  `bmpmvup()` in C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpmvup(from: c_short, to: c_short, count: c_int) {
    let mut from = from;
    let mut to = to;
    let (mut from_xbit, from_ybit) = ch2bit(from);
    let (mut to_xbit, to_ybit) = ch2bit(to);
    let s = state();
    if from_xbit == s.xstart
        && to_xbit == s.xstart
        && count % s.kd_cols as c_int == 0
    {
        // Fast case: entire lines.
        from_xbit = 0;
        to_xbit = 0;
        paintcsr(s.kd_curpos, s.char_black);
        let lines = count / s.kd_cols as c_int;
        let bytes = lines
            * s.fb_byte_width as c_int
            * (s.char_height + s.cursor_height) as c_int;
        // SAFETY: the whole-line block is inside the frame buffer.
        unsafe {
            glue::kd_slmscu(
                fbptr(from_xbit, from_ybit).cast(),
                fbptr(to_xbit, to_ybit).cast(),
                bytes / SLAMBPW,
            )
        };
        paintcsr(s.kd_curpos, s.char_white);
    } else {
        // Slow case: one character at a time.
        for _ in 0..count {
            cp1char(from, to);
            from += ONE_SPACE;
            to += ONE_SPACE;
        }
    }
}

/// Copy a block of characters down.  `bmpmvdown()` in C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpmvdown(from: c_short, to: c_short, count: c_int) {
    let mut from = from;
    let mut to = to;
    let (mut from_xbit, from_ybit) = ch2bit(from);
    let (mut to_xbit, to_ybit) = ch2bit(to);
    let s = state();
    let last = s.xstart + (s.kd_cols - 1) * s.char_width;
    if from_xbit == last && to_xbit == last && count % s.kd_cols as c_int == 0
    {
        // Fast case: entire lines, from the last byte on the line.
        from_xbit = 8 * (s.fb_byte_width - 1);
        to_xbit = from_xbit;
        paintcsr(s.kd_curpos, s.char_black);
        let lines = count / s.kd_cols as c_int;
        let bytes = lines
            * s.fb_byte_width as c_int
            * (s.char_height + s.cursor_height) as c_int;
        // SAFETY: the whole-line block is inside the frame buffer.
        unsafe {
            glue::kd_slmscd(
                fbptr(from_xbit, from_ybit).cast(),
                fbptr(to_xbit, to_ybit).cast(),
                bytes / SLAMBPW,
            )
        };
        paintcsr(s.kd_curpos, s.char_white);
    } else {
        for _ in 0..count {
            cp1char(from, to);
            from -= ONE_SPACE;
            to -= ONE_SPACE;
        }
    }
}

/// Clear one or more character positions.  `bmpclear()` in C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpclear(to: c_short, count: c_int, chattr: c_char) {
    let s = state();
    let clearbyte = if chattr as u8 == KA_REVERSE {
        s.char_white
    } else {
        s.char_black
    };
    let clearval = ((clearbyte as u16) << 8) + clearbyte as u16;
    if to == 0 && count >= s.kd_lines as c_int * s.kd_cols as c_int {
        // Fast case: the entire page.
        // SAFETY: the page is inside the frame buffer.
        unsafe {
            glue::kd_slmwd(
                s.vid_start.cast(),
                (s.fb_byte_width as c_int * s.fb_height as c_int) / SLAMBPW,
                clearval as c_int,
            )
        };
    } else {
        let mut to = to;
        for _ in 0..count {
            put(to, K_SPACE as c_char, chattr);
            to += ONE_SPACE;
        }
    }
}

/// Update the display cursor.  `bmpsetcursor()` in C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpsetcursor(pos: c_short) {
    let s = state();
    paintcsr(s.kd_curpos, s.char_black);
    paintcsr(pos, s.char_white);
    s.kd_curpos = pos;
}

/// Paint the cursor bits.  `bmppaintcsr()` in C.
///
/// # Safety
///
/// The bitmap backend must be selected and initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmppaintcsr(pos: c_short, val: u8) {
    paintcsr(pos, val);
}

/// Character position to bit addresses.  `bmpch2bit()` in C.
///
/// # Safety
///
/// `xb` and `yb` must point at valid `short`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bmpch2bit(
    pos: c_short,
    xb: *mut c_short,
    yb: *mut c_short,
) {
    let (x, y) = ch2bit(pos);
    // SAFETY: the caller promises both pointers are valid.
    unsafe {
        *xb = x;
        *yb = y;
    }
}

/// The frame-buffer pointer for a bit address.  `bit2fbptr()` in C.
///
/// # Safety
///
/// `xb` and `yb` must name a bit inside the frame buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bit2fbptr(xb: c_short, yb: c_short) -> *mut u8 {
    fbptr(xb, yb)
}
