// SPDX-License-Identifier: CMU-Mach
// Derived from kern/printf.c:
//   Copyright (c) 1993 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The non-variadic leaves of `kern/printf.c`: `printnum` and `safe_gets`.

use crate::arch::types::VmOffset;
use crate::glue;
use core::ffi::{c_char, c_int};

/// `MAXBUF` of <kern/printf.c>: enough for the binary form of a `long long`.
const MAXBUF: usize = u64::BITS as usize;

/// `digs[]` of <kern/printf.c>: the digits `printnum` indexes by remainder.
const DIGITS: [u8; 16] = *b"0123456789abcdef";

/// A radix in `2..=16`, the range [`DIGITS`] covers.
#[derive(Clone, Copy)]
struct Base(u32);

impl Base {
    /// The base named by the C `int`, or [`None`] when it is outside the digit
    /// table.
    fn new(base: c_int) -> Option<Self> {
        let base = u32::try_from(base).ok()?;
        match base {
            2..=16 => Some(Self(base)),
            _ => None,
        }
    }
}

/// Write `u` in `base` to `putc`, most significant digit first.
fn print_num<F>(mut u: u64, base: Base, putc: &mut F, putc_arg: VmOffset)
where
    F: FnMut(c_char, VmOffset),
{
    let mut buf = [0u8; MAXBUF];
    let mut len = 0;
    loop {
        let rem = u % u64::from(base.0);
        let Ok(index) = usize::try_from(rem) else {
            return;
        };
        let Some(digit) = DIGITS.get(index) else {
            return;
        };
        let Some(cell) = buf.get_mut(len) else {
            return;
        };
        *cell = *digit;
        len += 1;
        u /= u64::from(base.0);
        if u == 0 {
            break;
        }
    }
    let Some(digits) = buf.get(..len) else {
        return;
    };
    for byte in digits.iter().rev() {
        // Every byte in the table is ASCII, so the conversion to `c_char`
        // cannot change the value.
        putc(*byte as c_char, putc_arg);
    }
}

/// The `printnum()` entry of <kern/printf.h>, which `kern/printf.c` used to
/// define.
///
/// # Safety
///
/// When `putc` is non-null, it must be a valid function pointer that can be
/// called with `putc_arg` as its second argument.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn printnum(
    u: u64,
    base: c_int,
    putc: Option<unsafe extern "C" fn(c_char, VmOffset)>,
    putc_arg: VmOffset,
) {
    let Some(base) = Base::new(base) else {
        return;
    };
    let Some(callback) = putc else {
        return;
    };
    let mut putc = |c: c_char, arg: VmOffset| {
        // SAFETY: the caller promises `callback` is a live function pointer
        // and `arg` the argument it takes.
        unsafe { callback(c, arg) };
    };
    print_num(u, base, &mut putc, putc_arg);
}

/// Assemble one console line into `line`, echoing each accepted byte.
fn get_line(
    line: &mut [u8],
    getc: &mut impl FnMut() -> c_int,
    putc: &mut impl FnMut(u8),
) {
    let strmax = line.len().saturating_sub(1);
    let mut len = 0;
    loop {
        match getc() {
            0x0a | 0x0d => {
                putc(b'\n');
                if let Some(cell) = line.get_mut(len) {
                    *cell = 0;
                }
                return;
            }
            0x08 | 0x23 | 0x7f => {
                if len > 0 {
                    putc(b'\x08');
                    putc(b' ');
                    putc(b'\x08');
                    len -= 1;
                }
            }
            0x40 | 0x15 => {
                len = 0;
                putc(b'\n');
                putc(b'\r');
            }
            c if (0x20..0x7f).contains(&c) => {
                if len < strmax {
                    if let Some(cell) = line.get_mut(len) {
                        // The arm's upper bound is below 0x7f, so the byte is
                        // exact.
                        let byte = c as u8;
                        *cell = byte;
                        len += 1;
                        putc(byte);
                    }
                } else {
                    putc(b'\x07');
                }
            }
            _ => (),
        }
    }
}

/// The `safe_gets()` entry of <kern/printf.h>, which `kern/printf.c` used to
/// define.
///
/// # Safety
///
/// When `maxlen` is positive, `str` must point at `maxlen` writable bytes that
/// nothing else writes for the duration.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn safe_gets(str: *mut c_char, maxlen: c_int) {
    let len = usize::try_from(maxlen).unwrap_or(0);
    // SAFETY: for a positive `maxlen` the caller promises `str` is valid for
    // that many writes; for a non-positive one the slice is empty and no byte
    // is touched.
    let line =
        unsafe { core::slice::from_raw_parts_mut(str.cast::<u8>(), len) };
    let mut getc = || {
        // SAFETY: `cngetc()` is the real C symbol <device/cons.h> declares and
        // takes no argument.
        unsafe { glue::cngetc() }
    };
    let mut putc = |byte: u8| {
        // SAFETY: `printf` is the C entry point of <kern/printf.h>, the format
        // is the one-character literal below, and the character is its only
        // argument.
        unsafe { glue::printf(c"%c".as_ptr(), c_int::from(byte)) };
    };
    get_line(line, &mut getc, &mut putc);
}
