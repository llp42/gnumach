// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/rtc.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright 1988, 1989 by Intel Corporation, Santa Clara, California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The CMOS clock, which `i386/i386at/rtc.c` used to define and
//! `i386/i386at/rtc.h` declares.
//!
//! `readtodc()` reads the current time into the caller's `uint64_t`,
//! and `writetodc()` programs the registers from the kernel's
//! wall-clock global.  The registers hold binary-coded decimal, so the
//! two nibble conversions and the bissextile-year rules come along.
//! The alarm and status bytes are read back untouched.
//!
//! Three deliberate divergences from the C stay out of contract.  The
//! day arithmetic is `u64`, where the C's `int` product overflows for
//! dates after January 2038; a zero day-of-month saturates instead of
//! subtracting a day; and the month loop stops at the table end where
//! the C would read past `month[12]` for a corrupt month field.
//! `writetodc()` also samples the wall clock once, where the C read it
//! twice and could straddle a tick; the callers run it at `splhigh`
//! with the thread bound to the master CPU.

use crate::arch::i386::pio::Port;
use crate::glue;
use core::ffi::c_int;
use core::mem::{align_of, offset_of, size_of};
use core::sync::atomic::{AtomicBool, Ordering};

/// The first year the two-digit year field can name, `CENTURY_START`
/// in `rtc.c`.
const CENTURY_START: u32 = 1970;

/// The register select port, `RTC_ADDR` of <i386at/rtc.h>.
const RTC_ADDR: Port = Port::new(0x70);
/// The data port, `RTC_DATA` of <i386at/rtc.h>.
const RTC_DATA: Port = Port::new(0x71);

/// Register A: the time base and update rate, `RTC_A`.
const RTC_A: u8 = 0x0a;
/// Register B: the update and mode control, `RTC_B`.
const RTC_B: u8 = 0x0b;
/// Register D: the valid-RAM-and-time byte, `RTC_D`.
const RTC_D: u8 = 0x0d;
/// `RTC_UIP`: an update is in progress, in register A.
const RTC_UIP: u8 = 0x80;
/// `RTC_DIV2`: a 32.768 KHz time base, in register A.
const RTC_DIV2: u8 = 0x20;
/// `RTC_RATE6`: an interrupt rate of 976.562 Hz, in register A.
const RTC_RATE6: u8 = 0x06;
/// `RTC_SET`: updates stopped for a time set, in register B.
const RTC_SET: u8 = 0x80;
/// `RTC_HM`: 24-hour mode, in register B.
const RTC_HM: u8 = 0x02;
/// `RTC_VRT`: RAM and time are valid, in register D.
const RTC_VRT: u8 = 0x80;
/// `RTC_NREG`: how many registers `load_rtc` reads.
const RTC_NREG: u8 = 0x0e;
/// `RTC_NREGP`: how many registers `save_rtc` writes.
const RTC_NREGP: u8 = 0x0a;

/// The month lengths, `month` in `rtc.c`, with February at 28.
const MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Why the clock cannot supply a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RtcError {
    /// Register D's `RTC_VRT` is clear: the battery lost the time.
    NotValid,
}

/// `struct rtc_st` of <i386at/rtc.h>: the fourteen CMOS registers in
/// order.
///
/// The C cast the struct to `unsigned char *` for `load_rtc` and
/// `save_rtc`, so the fields are bytes here.  The alarm registers and
/// the four status bytes are read back unchanged and never inspected.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RtcSt {
    /// `rtc_sec`: seconds, register 0.
    rtc_sec: u8,
    /// `rtc_asec`: alarm seconds, register 1.
    rtc_asec: u8,
    /// `rtc_min`: minutes, register 2.
    rtc_min: u8,
    /// `rtc_amin`: alarm minutes, register 3.
    rtc_amin: u8,
    /// `rtc_hr`: hours, register 4.
    rtc_hr: u8,
    /// `rtc_ahr`: alarm hours, register 5.
    rtc_ahr: u8,
    /// `rtc_dow`: day of the week, register 6.
    rtc_dow: u8,
    /// `rtc_dom`: day of the month, register 7.
    rtc_dom: u8,
    /// `rtc_mon`: month, register 8.
    rtc_mon: u8,
    /// `rtc_yr`: year, register 9.
    rtc_yr: u8,
    /// `rtc_statusa`: register A, register 10.
    rtc_statusa: u8,
    /// `rtc_statusb`: register B, register 11.
    rtc_statusb: u8,
    /// `rtc_statusc`: register C, register 12.
    rtc_statusc: u8,
    /// `rtc_statusd`: register D, register 13.
    rtc_statusd: u8,
}

// A C `char[14]`: one byte per field, no padding.
const _: () = assert!(size_of::<RtcSt>() == 14);
const _: () = assert!(align_of::<RtcSt>() == 1);
const _: () = assert!(offset_of!(RtcSt, rtc_sec) == 0);
const _: () = assert!(offset_of!(RtcSt, rtc_asec) == 1);
const _: () = assert!(offset_of!(RtcSt, rtc_min) == 2);
const _: () = assert!(offset_of!(RtcSt, rtc_amin) == 3);
const _: () = assert!(offset_of!(RtcSt, rtc_hr) == 4);
const _: () = assert!(offset_of!(RtcSt, rtc_ahr) == 5);
const _: () = assert!(offset_of!(RtcSt, rtc_dow) == 6);
const _: () = assert!(offset_of!(RtcSt, rtc_dom) == 7);
const _: () = assert!(offset_of!(RtcSt, rtc_mon) == 8);
const _: () = assert!(offset_of!(RtcSt, rtc_yr) == 9);
const _: () = assert!(offset_of!(RtcSt, rtc_statusa) == 10);
const _: () = assert!(offset_of!(RtcSt, rtc_statusb) == 11);
const _: () = assert!(offset_of!(RtcSt, rtc_statusc) == 12);
const _: () = assert!(offset_of!(RtcSt, rtc_statusd) == 13);

impl RtcSt {
    /// `load_rtc` of <i386at/rtc.h>: read registers 0 through
    /// `RTC_NREG - 1` into the fields.
    fn load(&mut self) {
        let registers = [
            &mut self.rtc_sec,
            &mut self.rtc_asec,
            &mut self.rtc_min,
            &mut self.rtc_amin,
            &mut self.rtc_hr,
            &mut self.rtc_ahr,
            &mut self.rtc_dow,
            &mut self.rtc_dom,
            &mut self.rtc_mon,
            &mut self.rtc_yr,
            &mut self.rtc_statusa,
            &mut self.rtc_statusb,
            &mut self.rtc_statusc,
            &mut self.rtc_statusd,
        ];
        for (register, byte) in (0_u8..RTC_NREG).zip(registers) {
            RTC_ADDR.write_u8(register);
            *byte = RTC_DATA.read_u8();
        }
    }

    /// `save_rtc` of <i386at/rtc.h>: write the time and alarm fields
    /// back, leaving the status bytes alone.
    fn save(&self) {
        let registers = [
            &self.rtc_sec,
            &self.rtc_asec,
            &self.rtc_min,
            &self.rtc_amin,
            &self.rtc_hr,
            &self.rtc_ahr,
            &self.rtc_dow,
            &self.rtc_dom,
            &self.rtc_mon,
            &self.rtc_yr,
        ];
        for (register, byte) in (0_u8..RTC_NREGP).zip(registers) {
            RTC_ADDR.write_u8(register);
            RTC_DATA.write_u8(*byte);
        }
    }
}

/// Whether [`rtcinit()`] has run, the C's `first_rtcopen_ever`.
///
/// The C programmed the clock on the first open only.  The swap is
/// what picks the one caller that does it if two ever race; the
/// ordering is `Relaxed` because the flag guards the port writes and
/// publishes nothing else, and `splclock` covers callers on the same
/// CPU.
static RTC_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Program registers A and B: `rtcinit()` of `rtc.c`.
fn rtcinit() {
    RTC_ADDR.write_u8(RTC_A);
    RTC_DATA.write_u8(RTC_DIV2 | RTC_RATE6);
    RTC_ADDR.write_u8(RTC_B);
    RTC_DATA.write_u8(RTC_HM);
}

/// Run [`rtcinit()`] on the first call ever, the C's
/// `first_rtcopen_ever` check.
fn rtcinit_once() {
    if !RTC_INITIALIZED.swap(true, Ordering::Relaxed) {
        rtcinit();
    }
}

/// Read the register block: `rtcget()` of `rtc.c`.
///
/// Fails when register D says the time is invalid.  The register A
/// busy-wait re-selects the register each iteration, exactly as the C
/// loop did.
fn rtcget() -> Result<RtcSt, RtcError> {
    rtcinit_once();
    RTC_ADDR.write_u8(RTC_D);
    if RTC_DATA.read_u8() & RTC_VRT == 0 {
        return Err(RtcError::NotValid);
    }
    RTC_ADDR.write_u8(RTC_A);
    while RTC_DATA.read_u8() & RTC_UIP != 0 {
        RTC_ADDR.write_u8(RTC_A);
    }
    let mut st = RtcSt::default();
    st.load();
    Ok(st)
}

/// Program the time registers back: `rtcput()` of `rtc.c`.
///
/// Register B is read and written out with `RTC_SET` while the bytes
/// go, then written back without it.
fn rtcput(st: &RtcSt) {
    rtcinit_once();
    RTC_ADDR.write_u8(RTC_B);
    let saved = RTC_DATA.read_u8();
    RTC_ADDR.write_u8(RTC_B);
    RTC_DATA.write_u8(saved | RTC_SET);
    st.save();
    RTC_ADDR.write_u8(RTC_B);
    RTC_DATA.write_u8(saved & !RTC_SET);
}

/// The decimal value of the binary-coded-decimal byte `byte`:
/// `hexdectodec()` of `rtc.c`.
fn hexdectodec(byte: u8) -> u32 {
    // C `char` is signed on this target, but both operands are masked,
    // so the C's signed shift produces the same two digits.
    u32::from((byte >> 4) & 0x0F) * 10 + u32::from(byte & 0x0F)
}

/// The binary-coded-decimal byte for the two decimal digits of
/// `value`: `dectohexdec()` of `rtc.c`.
fn dectohexdec(value: u64) -> u8 {
    // In contract `value` is below 100, so the two nibbles are its two
    // digits and the C's `char` conversion loses nothing.
    ((((value / 10) << 4) & 0xF0) | ((value % 10) & 0x0F)) as u8
}

/// The number of days in `year`: `yeartoday()` of `rtc.c`.
fn yeartoday(year: u32) -> u32 {
    if !year.is_multiple_of(4) {
        // Not divisible by 4, not bissextile.
        return 365;
    }
    if !year.is_multiple_of(100) {
        // Not divisible by 100, bissextile.
        return 366;
    }
    if !year.is_multiple_of(400) {
        // Not divisible by 400, not bissextile.
        return 365;
    }
    // Divisible by 400: 2000 was made bissextile, and the rules after
    // it are not officially decided.
    366
}

/// The month lengths with February at 29 in a bissextile year.
///
/// The C mutated its `month` table for the length of each call; here
/// the table is [`MONTH`] and every caller takes its own copy.
fn month_lengths(bissextile: bool) -> [u8; 12] {
    let mut months = MONTH;
    if bissextile {
        months[1] = 29;
    }
    months
}

/// Read the wall clock.  The body of `readtodc()` in `rtc.c`.
///
/// Returns the seconds since [`CENTURY_START`] and fails only when the
/// clock reports itself invalid.  The clock is read at `splclock`; the
/// conversion then runs at the level `splx` restored, as the C did.
fn read_todc() -> Result<u64, RtcError> {
    // SAFETY: `splclock()` is the real asm function <i386/spl.h>
    // declares, and the value it returns is only handed back to
    // `splx()`.
    let ospl = unsafe { glue::splclock() };
    let st = match rtcget() {
        Ok(st) => st,
        Err(error) => {
            // SAFETY: `ospl` is the level `splclock()` returned.
            unsafe { glue::splx(ospl) };
            return Err(error);
        }
    };
    // SAFETY: `ospl` is the level `splclock()` returned.
    unsafe { glue::splx(ospl) };

    let sec = hexdectodec(st.rtc_sec);
    let min = hexdectodec(st.rtc_min);
    let hr = hexdectodec(st.rtc_hr);
    let dom = hexdectodec(st.rtc_dom);
    let mon = hexdectodec(st.rtc_mon);
    let mut yr = hexdectodec(st.rtc_yr);
    yr = if yr < CENTURY_START % 100 {
        yr + CENTURY_START - CENTURY_START % 100 + 100
    } else {
        yr + CENTURY_START - CENTURY_START % 100
    };

    if yr >= CENTURY_START + 90 {
        // SAFETY: `printf` is variadic; the format's one conversion is
        // `%u`, and the argument is the unsigned constant the C
        // passed.
        unsafe {
            glue::printf(
                c"FIXME: we are approaching %u, update CENTURY_START\n"
                    .as_ptr(),
                CENTURY_START,
            );
        }
    }

    // SAFETY: `printf` is variadic and every `%u` takes one of the
    // unsigned fields in the order the C passed them.
    unsafe {
        glue::printf(
            c"RTC time is %04u-%02u-%02u %02u:%02u:%02u\n".as_ptr(),
            yr,
            mon,
            dom,
            hr,
            min,
            sec,
        );
    }

    let mut days = 0_u64;
    let months = month_lengths(yeartoday(yr) == 366);
    for (length, month) in months.iter().zip(1_u32..) {
        if month >= mon {
            break;
        }
        days += u64::from(*length);
    }
    for year in CENTURY_START..yr {
        days += u64::from(yeartoday(year));
    }

    let mut n = u64::from(sec) + 60 * u64::from(min) + 3600 * u64::from(hr);
    n += u64::from(dom.saturating_sub(1)) * 3600 * 24;
    n += days * 3600 * 24;
    Ok(n)
}

/// Program the wall clock.  The body of `writetodc()` in `rtc.c`.
///
/// Reads the register block first, so the alarm and status bytes
/// survive, fills the time fields from the kernel's wall clock, and
/// programs them back at `splclock`.  Fails only when the clock reports
/// itself invalid.
fn write_todc() -> Result<(), RtcError> {
    // SAFETY: as in `read_todc()`.
    let ospl = unsafe { glue::splclock() };
    let mut st = match rtcget() {
        Ok(st) => st,
        Err(error) => {
            // SAFETY: `ospl` is the level `splclock()` returned.
            unsafe { glue::splx(ospl) };
            return Err(error);
        }
    };
    // SAFETY: `ospl` is the level `splclock()` returned.
    unsafe { glue::splx(ospl) };

    // SAFETY: `time` is the wall-clock global `kern/mach_clock.c`
    // defines, read at the level `splx()` just restored, as the C did.
    let seconds = unsafe { glue::time.seconds };
    // `time_t` is `unsigned long long`, and the C assigned the int64
    // wall clock to it; the clock is a post-epoch count, so the
    // sign-extending cast is that conversion.
    let seconds = seconds as u64;

    let mut n = seconds % (3600 * 24);
    st.rtc_sec = dectohexdec(n % 60);
    n /= 60;
    st.rtc_min = dectohexdec(n % 60);
    st.rtc_hr = dectohexdec(n / 60);

    n = seconds / (3600 * 24);
    // 1/1/70 is a Thursday and the field counts from Sunday, so the
    // value is below seven and the cast is exact.
    st.rtc_dow = ((n + 4) % 7) as u8;

    let mut year = u64::from(CENTURY_START);
    let mut year_days = u64::from(yeartoday(CENTURY_START));
    while n >= year_days {
        n -= year_days;
        year += 1;
        // In contract the clock is between 1970 and 2070, so the year
        // stays far inside `u32`.
        year_days = u64::from(yeartoday(year as u32));
    }
    st.rtc_yr = dectohexdec(year % 100);

    let months = month_lengths(year_days == 366);
    let mut month = 0_u64;
    for length in months.iter() {
        let length = u64::from(*length);
        if n < length {
            break;
        }
        n -= length;
        month += 1;
    }
    st.rtc_mon = dectohexdec(month + 1);
    st.rtc_dom = dectohexdec(n + 1);

    // SAFETY: `splclock()` returns the level `splx()` restores; the C
    // re-took it right before `rtcput()`.
    let ospl = unsafe { glue::splclock() };
    rtcput(&st);
    // SAFETY: `ospl` is the level just returned by `splclock()`.
    unsafe { glue::splx(ospl) };

    Ok(())
}

/// Read the time of day.  `readtodc()` of <i386at/rtc.h>, which
/// `i386/i386at/rtc.c` used to define.
///
/// Returns zero on success and `-1` when the clock reports itself
/// invalid; `*tp` is left alone on failure, as in the C.
///
/// # Safety
///
/// `tp` must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readtodc(tp: *mut u64) -> c_int {
    match read_todc() {
        Ok(seconds) => {
            // SAFETY: the caller promises `tp` is valid for a write.
            unsafe { *tp = seconds };
            0
        }
        Err(_) => -1,
    }
}

/// Program the time of day.  `writetodc()` of <i386at/rtc.h>, which
/// `i386/i386at/rtc.c` used to define.
///
/// Returns zero on success and `-1` when the clock reports itself
/// invalid.
///
/// # Safety
///
/// There is no argument contract, the C prototype takes none.  The
/// function programs the RTC through port I/O at `splclock`, as the C
/// did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn writetodc() -> c_int {
    match write_todc() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}
