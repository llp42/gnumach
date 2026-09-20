// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kd output engine: `kd_putc()` draws one character, `kd_putc_esc()`
//! collects escape sequences, and `kd_parserest()` interprets the ANSI
//! commands the console writes.
//!
//! The exported functions are the C entry points; the private ones are
//! the same operations for use inside the module, so the interpreter
//! stays safe code and the `unsafe` blocks stay at the boundary.

use super::display::{
    dclear, dmvdown, dmvup, dput, scrolldn, scrollup, setpos,
};
use super::*;
use crate::glue;
use core::ffi::{c_int, c_short};

/// `kd_bellon()`, for use inside the module.
fn bellon() {
    super::kd_bellon();
}

// Safe operations for the interpreter.

pub(crate) fn putc(ch: u8) {
    if ch == 0 && state().sit_for_0 != 0 {
        return;
    }
    match ch {
        K_LF => down(),
        K_CR => cr(),
        K_BS => left(),
        K_HT => tab(),
        K_BEL => {
            if !state().kd_bellstate {
                bellon();
                // SAFETY: the timer is the clock's, at SPLKD.
                unsafe {
                    glue::timeout(
                        Some(super::kd_belloff),
                        core::ptr::null_mut(),
                        glue::hz / 8,
                    )
                };
                state().kd_bellstate = true;
            }
        }
        _ => {
            let s = state();
            dput(s.kd_curpos, ch, s.kd_attr);
            right();
        }
    }
}

fn parseesc() {
    let seq = state().esc_seq;
    match seq[1] {
        b'c' => {
            cls();
            home();
            state().esc_spt = 0;
        }
        b'[' => parserest(&seq, 2),
        0 => {}
        c => {
            putc(c);
            state().esc_spt = 0;
        }
    }
}

fn up() {
    let s = state();
    if s.kd_curpos < ONE_LINE {
        scrolldn();
    } else {
        setpos(s.kd_curpos - ONE_LINE);
    }
}

fn down() {
    let s = state();
    if s.kd_curpos >= ONE_PAGE - ONE_LINE {
        scrollup();
    } else {
        setpos(s.kd_curpos + ONE_LINE);
    }
}

fn right() {
    let s = state();
    if s.kd_curpos < ONE_PAGE - ONE_SPACE {
        setpos(s.kd_curpos + ONE_SPACE);
    } else {
        scrollup();
        setpos(beg_of_line(s.kd_curpos));
    }
}

fn left() {
    let pos = state().kd_curpos;
    if 0 < pos {
        setpos(pos - ONE_SPACE);
    }
}

fn cr() {
    setpos(beg_of_line(state().kd_curpos));
}

fn home() {
    setpos(0);
}

fn cls() {
    let s = state();
    dclear(0, (ONE_PAGE / ONE_SPACE) as c_int, s.kd_attr);
}

fn cltobcur() {
    let s = state();
    let start = s.kd_curpos;
    let count = (ONE_PAGE - s.kd_curpos) / ONE_SPACE;
    dclear(start, count as c_int, s.kd_attr);
}

fn cltopcur() {
    let s = state();
    let count = (s.kd_curpos + ONE_SPACE) / ONE_SPACE;
    dclear(0, count as c_int, s.kd_attr);
}

fn cltoecur() {
    let s = state();
    let hold = beg_of_line(s.kd_curpos) + ONE_LINE;
    let mut i = s.kd_curpos;
    while i < hold {
        dput(i, K_SPACE, s.kd_attr);
        i += ONE_SPACE;
    }
}

fn clfrbcur() {
    let s = state();
    let mut i = beg_of_line(s.kd_curpos);
    while i <= s.kd_curpos {
        dput(i, K_SPACE, s.kd_attr);
        i += ONE_SPACE;
    }
}

fn eraseln() {
    let s = state();
    let stop = beg_of_line(s.kd_curpos) + ONE_LINE;
    let mut i = beg_of_line(s.kd_curpos);
    while i < stop {
        dput(i, K_SPACE, s.kd_attr);
        i += ONE_SPACE;
    }
}

fn erase(number: c_int) {
    let s = state();
    let mut stop = s.kd_curpos + ONE_SPACE * number as c_short;
    let line_end = beg_of_line(s.kd_curpos) + ONE_LINE;
    if stop > line_end {
        stop = line_end;
    }
    let mut i = s.kd_curpos;
    while i < stop {
        dput(i, K_SPACE, s.kd_attr);
        i += ONE_SPACE;
    }
}

fn insch(number: c_int) {
    if number <= 0 {
        return;
    }
    let s = state();
    let nextline = beg_of_line(s.kd_curpos) + ONE_LINE;
    let mut insbytes = number * ONE_SPACE as c_int;
    if s.kd_curpos as c_int + insbytes > nextline as c_int {
        insbytes = nextline as c_int - s.kd_curpos as c_int;
    }
    let to = nextline - ONE_SPACE;
    let from = to - insbytes as c_short;
    if from >= s.kd_curpos {
        let count = ((from - s.kd_curpos + ONE_SPACE) / ONE_SPACE) as c_int;
        dmvdown(from, to, count);
    }
    let count = insbytes / ONE_SPACE as c_int;
    dclear(s.kd_curpos, count, s.kd_attr);
}

fn delln(number: c_int) {
    if number <= 0 {
        return;
    }
    let s = state();
    let mut delbytes = number * ONE_LINE as c_int;
    let to = beg_of_line(s.kd_curpos);
    if to as c_int + delbytes >= ONE_PAGE as c_int {
        delbytes = ONE_PAGE as c_int - to as c_int;
    }
    if (to as c_int + delbytes) < ONE_PAGE as c_int {
        let from = to + delbytes as c_short;
        let count = ((ONE_PAGE - from) / ONE_SPACE) as c_int;
        dmvup(from, to, count);
    }
    let to = ONE_PAGE - delbytes as c_short;
    let count = delbytes / ONE_SPACE as c_int;
    dclear(to, count, s.kd_attr);
}

fn insln(number: c_int) {
    if number <= 0 {
        return;
    }
    let s = state();
    let top = beg_of_line(s.kd_curpos);
    let mut insbytes = number * ONE_LINE as c_int;
    if top as c_int + insbytes > ONE_PAGE as c_int {
        insbytes = ONE_PAGE as c_int - top as c_int;
    }
    let to = ONE_PAGE - ONE_SPACE;
    let from = to - insbytes as c_short;
    if from > top {
        let count = ((from - top + ONE_SPACE) / ONE_SPACE) as c_int;
        dmvdown(from, to, count);
    }
    let count = insbytes / ONE_SPACE as c_int;
    dclear(top, count, s.kd_attr);
}

fn delch(number: c_int) {
    if number <= 0 {
        return;
    }
    let s = state();
    let nextline = beg_of_line(s.kd_curpos) + ONE_LINE;
    let mut delbytes = number * ONE_SPACE as c_int;
    if s.kd_curpos as c_int + delbytes > nextline as c_int {
        delbytes = nextline as c_int - s.kd_curpos as c_int;
    }
    if (s.kd_curpos as c_int + delbytes) < nextline as c_int {
        let from = s.kd_curpos + delbytes as c_short;
        let to = s.kd_curpos;
        let count = ((nextline - from) / ONE_SPACE) as c_int;
        dmvup(from, to, count);
    }
    let to = nextline - delbytes as c_short;
    let count = delbytes / ONE_SPACE as c_int;
    dclear(to, count, s.kd_attr);
}

fn tab() {
    let n = 8 - current_column(state().kd_curpos) % 8;
    let mut i = 0;
    while i < n {
        putc(b' ');
        i += 1;
    }
}

/// The `reverse_video_char()` macro of <i386at/kd.c>.
fn reverse_video_char(a: u8) -> u8 {
    (a & 0x88) | ((a.wrapping_shr(4) | a.wrapping_shl(4)) & 0x77)
}

/// `kd_update_kd_attr()` in C: blend `kd_attrflags` and `kd_color`.
fn update_kd_attr() {
    let s = state();
    let mut attr = s.kd_color;
    if s.kd_attrflags & KAX_UNDERLINE != 0 {
        attr = (attr & 0xf0) | KAX_COL_UNDERLINE;
    } else if s.kd_attrflags & KAX_DIM != 0 {
        attr = (attr & 0xf0) | KAX_COL_DIM;
    }
    if s.kd_attrflags & KAX_REVERSE != 0 {
        attr = reverse_video_char(attr);
    }
    if s.kd_attrflags & KAX_BLINK != 0 {
        attr ^= 0x80;
    }
    if s.kd_attrflags & KAX_BOLD != 0 {
        attr ^= 0x08;
    }
    s.kd_attr = attr;
}

/// Run `f` `n` times, for the `while (number[0]--)` loops.
fn repeat(n: c_int, f: fn()) {
    let mut i = 0;
    while i < n {
        f();
        i += 1;
    }
}

/// The ANSI interpreter.  `parserest()` in C.
fn parserest(seq: &[u8; K_MAXESC], start: usize) {
    let mut cp = start;
    let mut number = [MACH_ATOI_DEFAULT; 16];
    let mut npar: usize = 0;
    let mut question = false;
    let mut angle = false;

    if seq[cp] == b'?' {
        question = true;
        cp += 1;
    } else if seq[cp] == b'<' {
        angle = true;
        cp += 1;
    }

    loop {
        let mut n: c_int = 0;
        // SAFETY: `seq` is NUL-terminated, so the pointer is valid for
        // `mach_atoi()`.
        let used = unsafe { glue::mach_atoi(seq.as_ptr().add(cp), &mut n) };
        cp += used as usize;
        number[npar] = n;
        if seq[cp] != b';' {
            break;
        }
        npar += 1;
        if npar > 15 {
            break;
        }
        cp += 1;
        if seq[cp] == 0 {
            break;
        }
    }
    let np = npar.min(15);

    if question || angle {
        // Unsupported `\e[?...` and `\e[<...` sequences.
        match seq[cp] {
            0 => {}
            c if (b'@'..=b'~').contains(&c) => state().esc_spt = 0,
            c => {
                putc(c);
                state().esc_spt = 0;
            }
        }
        return;
    }

    match seq[cp] {
        b'm' => {
            for value in &number[..=np] {
                match *value {
                    MACH_ATOI_DEFAULT | 0 => {
                        state().kd_attrflags = 0;
                        state().kd_color = KA_NORMAL;
                    }
                    1 => {
                        state().kd_attrflags |= KAX_BOLD;
                        state().kd_attrflags &= !KAX_DIM;
                    }
                    2 => {
                        state().kd_attrflags |= KAX_DIM;
                        state().kd_attrflags &= !KAX_BOLD;
                    }
                    4 => state().kd_attrflags |= KAX_UNDERLINE,
                    5 => state().kd_attrflags |= KAX_BLINK,
                    7 => state().kd_attrflags |= KAX_REVERSE,
                    8 => state().kd_attrflags |= KAX_INVISIBLE,
                    21 | 22 => state().kd_attrflags &= !(KAX_BOLD | KAX_DIM),
                    24 => state().kd_attrflags &= !KAX_UNDERLINE,
                    25 => state().kd_attrflags &= !KAX_BLINK,
                    27 => state().kd_attrflags &= !KAX_REVERSE,
                    38 => {
                        state().kd_attrflags |= KAX_UNDERLINE;
                        state().kd_color =
                            (state().kd_color & 0xf0) | (KA_NORMAL & 0x0f);
                    }
                    39 => {
                        state().kd_attrflags &= !KAX_UNDERLINE;
                        state().kd_color =
                            (state().kd_color & 0xf0) | (KA_NORMAL & 0x0f);
                    }
                    v if (30..=37).contains(&v) => {
                        let c = COLOR_TABLE[(v - 30) as usize];
                        state().kd_color = (state().kd_color & 0xf0) | c;
                    }
                    v if (40..=47).contains(&v) => {
                        let c = COLOR_TABLE[(v - 40) as usize];
                        state().kd_color =
                            (state().kd_color & 0x0f) | (c << 4);
                    }
                    _ => {}
                }
            }
            update_kd_attr();
            state().esc_spt = 0;
        }
        b'@' => {
            if number[0] == MACH_ATOI_DEFAULT {
                insch(1);
            } else {
                insch(number[0]);
            }
            state().esc_spt = 0;
        }
        b'A' => {
            if number[0] == MACH_ATOI_DEFAULT {
                up();
            } else {
                repeat(number[0], up);
            }
            state().esc_spt = 0;
        }
        b'B' => {
            if number[0] == MACH_ATOI_DEFAULT {
                down();
            } else {
                repeat(number[0], down);
            }
            state().esc_spt = 0;
        }
        b'C' => {
            if number[0] == MACH_ATOI_DEFAULT {
                right();
            } else {
                repeat(number[0], right);
            }
            state().esc_spt = 0;
        }
        b'D' => {
            if number[0] == MACH_ATOI_DEFAULT {
                left();
            } else {
                repeat(number[0], left);
            }
            state().esc_spt = 0;
        }
        b'E' => {
            cr();
            if number[0] == MACH_ATOI_DEFAULT {
                down();
            } else {
                repeat(number[0], down);
            }
            state().esc_spt = 0;
        }
        b'F' => {
            cr();
            if number[0] == MACH_ATOI_DEFAULT {
                up();
            } else {
                repeat(number[0], up);
            }
            state().esc_spt = 0;
        }
        b'G' => {
            if number[0] == MACH_ATOI_DEFAULT {
                number[0] = 0;
            } else if number[0] > 0 {
                number[0] -= 1; // numbered from 1
            }
            setpos(
                beg_of_line(state().kd_curpos)
                    + number[0] as c_short * ONE_SPACE,
            );
            state().esc_spt = 0;
        }
        b'f' | b'H' => {
            if number[0] == MACH_ATOI_DEFAULT && number[1] == MACH_ATOI_DEFAULT
            {
                home();
                state().esc_spt = 0;
                return;
            }
            if number[0] == MACH_ATOI_DEFAULT {
                number[0] = 0;
            } else if number[0] > 0 {
                number[0] -= 1; // numbered from 1
            }
            let mut newpos = number[0] as c_short * ONE_LINE;
            if number[1] == MACH_ATOI_DEFAULT {
                number[1] = 0;
            } else if number[1] > 0 {
                number[1] -= 1;
            }
            newpos += number[1] as c_short * ONE_SPACE;
            if newpos < 0 {
                newpos = 0; // upper left
            }
            if newpos > ONE_PAGE {
                newpos = ONE_PAGE - ONE_SPACE; // lower right
            }
            setpos(newpos);
            state().esc_spt = 0;
        }
        b'J' => {
            match number[0] {
                MACH_ATOI_DEFAULT | 0 => cltobcur(),
                1 => cltopcur(),
                2 => cls(),
                _ => {}
            }
            state().esc_spt = 0;
        }
        b'K' => {
            match number[0] {
                MACH_ATOI_DEFAULT | 0 => cltoecur(),
                1 => clfrbcur(),
                2 => eraseln(),
                _ => {}
            }
            state().esc_spt = 0;
        }
        b'L' => {
            if number[0] == MACH_ATOI_DEFAULT {
                insln(1);
            } else {
                insln(number[0]);
            }
            state().esc_spt = 0;
        }
        b'M' => {
            if number[0] == MACH_ATOI_DEFAULT {
                delln(1);
            } else {
                delln(number[0]);
            }
            state().esc_spt = 0;
        }
        b'P' => {
            if number[0] == MACH_ATOI_DEFAULT {
                delch(1);
            } else {
                delch(number[0]);
            }
            state().esc_spt = 0;
        }
        b'S' => {
            if number[0] == MACH_ATOI_DEFAULT {
                scrollup();
            } else {
                repeat(number[0], scrollup);
            }
            state().esc_spt = 0;
        }
        b'T' => {
            if number[0] == MACH_ATOI_DEFAULT {
                scrolldn();
            } else {
                repeat(number[0], scrolldn);
            }
            state().esc_spt = 0;
        }
        b'X' => {
            if number[0] == MACH_ATOI_DEFAULT {
                erase(1);
            } else {
                erase(number[0]);
            }
            state().esc_spt = 0;
        }
        0 => {}
        c => {
            if !(b'@'..=b'~').contains(&c) {
                putc(c);
            }
            state().esc_spt = 0;
        }
    }
}

/// Collect one character, or replay an escape sequence.
/// `kd_putc_esc()` in C.
pub(crate) fn putc_esc(c: u8) {
    let s = state();
    if c == K_ESC {
        if s.esc_spt == 0 {
            let sp = s.esc_spt;
            s.esc_seq[sp] = K_ESC;
            s.esc_spt = sp + 1;
            s.esc_seq[s.esc_spt] = 0;
        } else {
            putc(K_ESC);
            s.esc_spt = 0;
        }
    } else if s.esc_spt != 0 {
        if s.esc_spt > K_MAXESC - 1 {
            s.esc_spt = 0;
        } else {
            let sp = s.esc_spt;
            s.esc_seq[sp] = c;
            s.esc_spt = sp + 1;
            s.esc_seq[s.esc_spt] = 0;
            parseesc();
        }
    } else {
        putc(c);
    }
}
