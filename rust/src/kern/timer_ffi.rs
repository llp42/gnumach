// SPDX-License-Identifier: CMU-Mach
// Derived from kern/timer.c and kern/timer.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the timer module, one adapter per symbol
//! `kern/timer.c` used to define and `kern/timer.h` declares.

use crate::glue::time_value::TimeValue64;
use crate::kern::thread::Thread;
use crate::kern::timer::{self, Timer, TimerSave};
use core::ffi::c_uint;

/// `init_timers()` of kern/timer.c.
///
/// # Safety
///
/// `kern/startup.c` calls this once during the boot, before any other CPU
/// starts its timer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_timers() {
    // SAFETY: the caller's contract.
    unsafe { timer::init_timers() };
}

/// `timer_init()` of kern/timer.c.
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

/// `timer_normalize()` of kern/timer.c.
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

/// `timer_delta()` of kern/timer.c.
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
    unsafe { timer::delta(&*timer, &mut *save) }
}

/// `timer_read()` of kern/timer.c.
///
/// # Safety
///
/// `timer` must point at a live [`Timer`] that does not overlap `tv`, and `tv`
/// must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_read(timer: *mut Timer, tv: *mut TimeValue64) {
    // SAFETY: the caller promises a live timer.
    let value = timer::read(unsafe { &*timer });
    // SAFETY: the caller promises `tv` is valid for a write.
    unsafe { tv.write(value) };
}

/// `thread_read_times()` of kern/timer.c.
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
    let (user, system) = timer::read_times(unsafe { &*thread });
    // SAFETY: the caller promises both pointers are valid for writes, and
    // neither overlaps the thread the reads just finished.
    unsafe {
        user_time_p.write(user);
        system_time_p.write(system);
    }
}

/// `db_thread_read_times()` of kern/timer.c.
///
/// # Safety
///
/// `thread` must point at a live [`Thread`] that is not being reaped, and both
/// output pointers must be valid for writes and must not overlap `thread`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn db_thread_read_times(
    thread: *mut Thread,
    user_time_p: *mut TimeValue64,
    system_time_p: *mut TimeValue64,
) {
    // SAFETY: the caller promises a live thread.
    let (user, system) = timer::db_read_times(unsafe { &*thread });
    // SAFETY: the caller promises both pointers are valid for writes, and
    // neither overlaps the thread the reads just finished.
    unsafe {
        user_time_p.write(user);
        system_time_p.write(system);
    }
}
