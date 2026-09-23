// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/pit.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
//   Copyright 1988, 1989 by Intel Corporation, Santa Clara, California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The 8254 timer, which `i386/i386/pit.c` used to define and
//! `i386/i386/pit.h` declares.
//!
//! Counter 2 measures the wait: [`prepare_sleep()`] gates it and loads
//! the count, and [`sleep()`] starts it and spins on the output bit.
//! [`udelay()`] splits a wait longer than [`MAX_PIT_USEC`] into
//! successive loads, and [`mdelay()`] scales milliseconds.
//!
//! The caller contract is a non-negative duration.  Two out-of-contract
//! cases are handled deliberately: a negative `usec` or `msec` returns
//! without touching the ports, where the C converted it to `uint64_t`
//! and programmed an accidental count, and the `1000 * msec` product
//! saturates instead of overflowing.  Zero still runs the full
//! prepare-and-wait, as the C does.
//!
//! [`clkstart()`] programs counter 0 for the kernel clock: only the
//! master CPU runs it, and it holds interrupts off across the mode and
//! counter writes.  The whole C file is Rust now.

use crate::arch::i386::percpu::cpu_number;
use crate::arch::i386::pio::Port;
use crate::glue;
use core::ffi::c_int;

/// The PIT control port, `PITCTL_PORT` of <i386/pit.h>.
const PITCTL_PORT: Port = Port::new(0x43);
/// Counter 0's data port, `PITCTR0_PORT` of <i386/pit.h>.
const PITCTR0_PORT: Port = Port::new(0x40);
/// Counter 2's data port, `PITCTR2_PORT` of <i386/pit.h>.
const PITCTR2_PORT: Port = Port::new(0x42);
/// The PIT auxiliary port, `PITAUX_PORT` of <i386/pit.h>.
const PITAUX_PORT: Port = Port::new(0x61);
/// The port read for a tiny I/O delay, `POST_PORT` of <i386/pit.h>.
const POST_PORT: Port = Port::new(0x80);

/// Counter 2's gate input in the auxiliary port, `PITAUX_GATE2`.
const PITAUX_GATE2: u8 = 0x01;
/// Counter 2's clock output enable, `PITAUX_OUT2`.
const PITAUX_OUT2: u8 = 0x02;
/// Counter 2's output bit, `PITAUX_VAL`.
const PITAUX_VAL: u8 = 0x20;

/// Select counter 0, `PIT_C0`.
const PIT_C0: u8 = 0x00;
/// Select counter 2, `PIT_C2`.
const PIT_C2: u8 = 0x80;
/// Load the least significant byte, then the most significant one,
/// `PIT_LOADMODE`.
const PIT_LOADMODE: u8 = 0x30;
/// Read or load the least significant byte, then the most
/// significant one, `PIT_READMODE`.
const PIT_READMODE: u8 = 0x30;
/// Square-wave mode, `PIT_SQUAREMODE`.
const PIT_SQUAREMODE: u8 = 0x06;
/// One-shot mode, `PIT_ONESHOTMODE`.
const PIT_ONESHOTMODE: u8 = 0x02;

/// The control byte `clkstart()` programs for counter 0,
/// `PIT_C0|PIT_SQUAREMODE|PIT_READMODE`.
const PIT0_MODE: u8 = PIT_C0 | PIT_SQUAREMODE | PIT_READMODE;

/// The timer input clock, `CLKNUM` of <i386/pit.h>, in ticks per
/// second.
const CLKNUM: u32 = 1193182;

/// The longest wait one counter load covers, `MAX_PIT_USEC` of
/// <i386/pit.h>.
///
/// A 16-bit counter at [`CLKNUM`] covers 0xffff ticks, which is 54924
/// whole microseconds.
const MAX_PIT_USEC: u32 = 54924;

/// Program counter 2 for a one-shot wait of `usec` microseconds.
///
/// The body of `pit_prepare_sleep()` in `pit.c`.  `usec` is at most
/// [`MAX_PIT_USEC`] in contract; [`udelay()`] splits longer waits, and
/// a zero loads a full 65536-tick count, as the C does.
fn prepare_sleep(usec: u32) {
    let aux = PITAUX_PORT.read_u8();
    let aux = (aux & !PITAUX_OUT2) | PITAUX_GATE2;
    PITAUX_PORT.write_u8(aux);
    PITCTL_PORT.write_u8(PIT_C2 | PIT_LOADMODE | PIT_ONESHOTMODE);
    let count = u64::from(CLKNUM) * u64::from(usec) / 1_000_000;
    // The counter latch is 16 bits wide and `outb` writes one byte, so
    // the C's byte pair is the quotient's low two bytes.
    let lsb = (count & 0xff) as u8;
    let msb = (count >> 8) as u8;
    PITCTR2_PORT.write_u8(lsb);
    // The POST-port read is the C's ~1us I/O delay; the byte it
    // returns is discarded.
    POST_PORT.read_u8();
    PITCTR2_PORT.write_u8(msb);
}

/// Start the one-shot and spin until counter 2 reaches zero.
///
/// The body of `pit_sleep()` in `pit.c`.
fn sleep() {
    let aux = PITAUX_PORT.read_u8();
    let low = aux & !PITAUX_GATE2;
    PITAUX_PORT.write_u8(low);
    PITAUX_PORT.write_u8(low | PITAUX_GATE2);
    while PITAUX_PORT.read_u8() & PITAUX_VAL == 0 {}
}

/// Busy-wait for `usec` microseconds.
///
/// The body of `pit_udelay()` in `pit.c`: a wait longer than
/// [`MAX_PIT_USEC`] is the same prepare-and-spin run repeatedly.
fn udelay(mut usec: u32) {
    while usec > MAX_PIT_USEC {
        prepare_sleep(MAX_PIT_USEC);
        sleep();
        usec -= MAX_PIT_USEC;
    }
    prepare_sleep(usec);
    sleep();
}

/// Busy-wait for `msec` milliseconds.
///
/// The body of `pit_mdelay()` in `pit.c`.  The C's `1000 * msec`
/// overflows a signed `int`, which is undefined; the port saturates at
/// `u32::MAX` so a huge wait stays long instead of wrapping into a
/// short one.
fn mdelay(msec: u32) {
    udelay(msec.saturating_mul(1000));
}

/// Prepare counter 2 for a sleep of `usec` microseconds.
///
/// The `pit_prepare_sleep()` entry of <i386/pit.h>, which
/// `i386/i386/pit.c` used to define.  A negative duration is out of
/// contract and returns without touching the ports.
#[unsafe(no_mangle)]
pub extern "C" fn pit_prepare_sleep(usec: c_int) {
    let Ok(usec) = u32::try_from(usec) else {
        return;
    };
    prepare_sleep(usec);
}

/// Start the one-shot and wait for counter 2.
///
/// The `pit_sleep()` entry of <i386/pit.h>, which `i386/i386/pit.c`
/// used to define.  It takes no argument.
#[unsafe(no_mangle)]
pub extern "C" fn pit_sleep() {
    sleep();
}

/// Busy-wait for `usec` microseconds.
///
/// The `pit_udelay()` entry of <i386/pit.h>, which `i386/i386/pit.c`
/// used to define.  A negative duration is out of contract and returns
/// without touching the ports.
#[unsafe(no_mangle)]
pub extern "C" fn pit_udelay(usec: c_int) {
    let Ok(usec) = u32::try_from(usec) else {
        return;
    };
    udelay(usec);
}

/// Busy-wait for `msec` milliseconds.
///
/// The `pit_mdelay()` entry of <i386/pit.h>, which `i386/i386/pit.c`
/// used to define.  A negative duration is out of contract and returns
/// without touching the ports.
#[unsafe(no_mangle)]
pub extern "C" fn pit_mdelay(msec: c_int) {
    let Ok(msec) = u32::try_from(msec) else {
        return;
    };
    mdelay(msec);
}

/// Program counter 0 for the kernel clock.
///
/// The `clkstart()` entry of <i386/pit.h>, which `i386/i386/pit.c`
/// used to define.  Only the master CPU programs the PIT; every other
/// CPU returns at once.  The mode and the two interval bytes reach
/// their ports with interrupts disabled.
///
/// # Panics
///
/// Panics if the kernel's `hz` global is zero: the interval
/// conversion divides by it.  The C divides by zero in the same case
/// and faults; `hz` is initialized to `HZ` and never written.
#[unsafe(no_mangle)]
pub extern "C" fn clkstart() {
    if cpu_number() != 0 {
        // Only one PIT initialization is needed.
        return;
    }

    // SAFETY: `sploff()` is the real asm function <i386/spl.h>
    // declares and the arch's `spl.S` (i386/i386/spl.S or x86_64/spl.S)
    // defines.  It disables interrupts
    // and returns the interrupt-enable flags word, which only
    // `splon()` consumes.
    let s = unsafe { glue::sploff() };

    PITCTL_PORT.write_u8(PIT0_MODE);

    // SAFETY: `hz` is the `int hz` of <kern/mach_clock.h>, initialized
    // to `HZ` before `clkstart()` can run and never written after, so
    // this is a race-free load of an initialized `int`.
    let hz = unsafe { glue::hz };

    // The C computed the interval in `int` and stored it in an
    // `unsigned int`: `(CLKNUM + hz / 2) / hz`.  For the positive `hz`
    // the kernel sets, `hz / 2` is at most `c_int::MAX / 2`, so the
    // sum cannot overflow, and the quotient is non-negative and at
    // most `CLKNUM`; converting it to `u32` is exact.  A zero `hz` is
    // the division by zero the `# Panics` section names.
    // 1193182 fits `c_int`, so the cast is exact.
    let clknumb = ((CLKNUM as c_int) + hz / 2) / hz;
    // The C's `unsigned int` store.
    let clknumb = clknumb as u32;

    // The counter latch is 16 bits wide, so the C's byte pair is the
    // quotient's low two bytes.
    PITCTR0_PORT.write_u8(clknumb as u8);
    PITCTR0_PORT.write_u8((clknumb >> 8) as u8);

    // SAFETY: `s` is the flags word just returned by `sploff()`.
    unsafe { glue::splon(s) };
}
