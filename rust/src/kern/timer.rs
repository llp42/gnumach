// SPDX-License-Identifier: CMU-Mach
// Derived from kern/timer.c and kern/timer.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The statistical timers, which `kern/timer.c` used to define for
//! `kern/timer.h`.

use crate::glue::time_value::TimeValue64;
use crate::kern::thread::Thread;
use core::ffi::c_uint;
use core::mem::offset_of;
use core::sync::atomic::{Ordering, fence};

/// `TIMER_RATE` in <kern/timer.h>: the timer's tick rate, in microseconds per
/// second.
const TIMER_RATE: c_uint = 1_000_000;

/// `struct timer` of <kern/timer.h>: the statistical CPU timer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timer {
    /// `low_bits`: the microsecond count.
    pub low_bits: c_uint,
    /// `high_bits`: the seconds count.
    pub high_bits: c_uint,
    /// `high_bits_check`: a reader's copy of `high_bits`.
    pub high_bits_check: c_uint,
    /// `tstamp`: the last reading's timestamp.
    pub tstamp: c_uint,
}

/// `struct timer_save` of <kern/timer.h>: a saved timer reading.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerSave {
    pub low: c_uint,
    pub high: c_uint,
}

impl Timer {
    /// Zero every field, as `timer_init()` of <kern/timer.c> did.
    pub fn init(&mut self) {
        self.low_bits = 0;
        self.high_bits = 0;
        self.tstamp = 0;
        self.high_bits_check = 0;
    }

    /// Fold whole seconds out of the microsecond count, as `timer_normalize()`
    /// of <kern/timer.c> did.
    pub fn normalize(&mut self) {
        let high_increment = self.low_bits / TIMER_RATE;
        self.high_bits_check =
            self.high_bits_check.wrapping_add(high_increment);
        // The SeqCst fence publishes the new check before the low count is
        // reduced, pairing with the second fence in `grab()`.
        fence(Ordering::SeqCst);
        self.low_bits %= TIMER_RATE;
        // The SeqCst fence publishes the new check before the new high count,
        // pairing with the first fence in `grab()`.
        fence(Ordering::SeqCst);
        self.high_bits = self.high_bits.wrapping_add(high_increment);
    }
}

impl TimerSave {
    /// The ticks elapsed since this reading, which is updated to the current
    /// timer value.
    ///
    /// # Safety
    ///
    /// `timer` and `self` must be the live pair of one thread, and the caller
    /// must serialize updates to them, as the thread lock does.
    #[must_use]
    pub unsafe fn delta(&mut self, timer: &Timer) -> c_uint {
        let low = timer.low_bits;
        if self.high != timer.high_bits_check {
            delta(timer, self)
        } else {
            let elapsed = low.wrapping_sub(self.low);
            self.low = low;
            elapsed
        }
    }
}

/// Read a coherent pair of fields from `timer` into `save`, as `timer_grab()`
/// of <kern/timer.c> did.
fn grab(timer: &Timer, save: &mut TimerSave) {
    loop {
        save.high = timer.high_bits;
        // The SeqCst fence orders the high read before the low read.
        fence(Ordering::SeqCst);
        save.low = timer.low_bits;
        // The SeqCst fence orders the low read before the check read, so the
        // check is as late as the C barrier put it.
        fence(Ordering::SeqCst);
        if save.high == timer.high_bits_check {
            break;
        }
    }
}

/// Take the difference between `save` and the live `timer`, updating `save` to
/// the reading, as `timer_delta()` of <kern/timer.c> did.
fn delta(timer: &Timer, save: &mut TimerSave) -> c_uint {
    let mut new_save = TimerSave::default();
    grab(timer, &mut new_save);
    let result = new_save
        .high
        .wrapping_sub(save.high)
        .wrapping_mul(TIMER_RATE)
        .wrapping_add(new_save.low)
        .wrapping_sub(save.low);
    *save = new_save;
    result
}

/// Read `timer` as seconds and nanoseconds, as `timer_read()` of
/// <kern/timer.c> did.
fn read(timer: &Timer) -> TimeValue64 {
    let mut save = TimerSave::default();
    grab(timer, &mut save);
    TimeValue64 {
        seconds: i64::from(save.high.wrapping_add(save.low / TIMER_RATE)),
        nanoseconds: i64::from(save.low % TIMER_RATE * 1000),
    }
}

/// Read a thread's user and system times, as `thread_read_times()` of
/// <kern/timer.c> did.
fn read_times(thread: &Thread) -> (TimeValue64, TimeValue64) {
    (read(&thread.user_timer), read(&thread.system_timer))
}

/// `timer_init()` in C.
///
/// # Safety
///
/// `timer` must point at writable storage for a [`Timer`] that no other thread
/// can see yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_init(timer: *mut Timer) {
    // SAFETY: the caller promises writable, unshared storage.
    unsafe { (*timer).init() };
}

/// `timer_normalize()` in C.
///
/// # Safety
///
/// `timer` must point at a live [`Timer`], and the caller must serialize the
/// normalize against every other writer, as the CPU owning the timer does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_normalize(timer: *mut Timer) {
    // SAFETY: the caller promises a live timer it owns for writing.
    unsafe { (*timer).normalize() };
}

/// `timer_delta()` in C.
///
/// # Safety
///
/// `timer` and `save` must be the live pair of one thread, must not overlap
/// each other, and the caller must serialize updates to them, as the thread
/// lock does.
#[unsafe(no_mangle)]
#[must_use]
pub unsafe extern "C" fn timer_delta(
    timer: *mut Timer,
    save: *mut TimerSave,
) -> c_uint {
    // SAFETY: the caller promises the live pair and its serialization.
    unsafe { delta(&*timer, &mut *save) }
}

/// `timer_read()` in C.
///
/// # Safety
///
/// `timer` must point at a live [`Timer`] that does not overlap `tv`, and `tv`
/// must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_read(timer: *mut Timer, tv: *mut TimeValue64) {
    // SAFETY: the caller promises a live timer.
    let value = read(unsafe { &*timer });
    // SAFETY: the caller promises `tv` is valid for a write.
    unsafe { tv.write(value) };
}

/// `thread_read_times()` in C.
///
/// # Safety
///
/// `thread` must point at a live [`Thread`], and both output pointers must be
/// valid for writes and must not overlap `thread`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_read_times(
    thread: *mut Thread,
    user_time_p: *mut TimeValue64,
    system_time_p: *mut TimeValue64,
) {
    // SAFETY: the caller promises a live thread.
    let (user, system) = read_times(unsafe { &*thread });
    // SAFETY: the caller promises both pointers are valid for writes, and
    // neither overlaps the thread the reads just finished.
    unsafe {
        user_time_p.write(user);
        system_time_p.write(system);
    }
}

const _: () = assert!(size_of::<Timer>() == 16);
const _: () = assert!(align_of::<Timer>() == 4);
const _: () = assert!(offset_of!(Timer, low_bits) == 0);
const _: () = assert!(offset_of!(Timer, high_bits) == 4);
const _: () = assert!(offset_of!(Timer, high_bits_check) == 8);
const _: () = assert!(offset_of!(Timer, tstamp) == 12);

const _: () = assert!(size_of::<TimerSave>() == 8);
const _: () = assert!(align_of::<TimerSave>() == 4);
const _: () = assert!(offset_of!(TimerSave, low) == 0);
const _: () = assert!(offset_of!(TimerSave, high) == 4);
