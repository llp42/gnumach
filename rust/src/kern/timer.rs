// SPDX-License-Identifier: CMU-Mach
// Derived from kern/timer.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The statistical timers, which `kern/timer.h` declares.
//!
//! [`Timer`] splits microseconds from seconds, and [`TimerSave`] holds
//! a saved reading for the delta macros to subtract.  The functions
//! that read and normalize them stay C in `kern/timer.c`, and
//! `struct thread` embeds two of each, so their layouts are pinned
//! here.

use crate::glue;
use core::ffi::c_uint;
use core::mem::offset_of;

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
    /// `low`: the saved low half.
    pub low: c_uint,
    /// `high`: the saved high half.
    pub high: c_uint,
}

impl TimerSave {
    /// The ticks elapsed since this reading, which is updated to the
    /// current timer value.  The `TIMER_DELTA` macro of <kern/timer.h>.
    ///
    /// # Safety
    ///
    /// `timer` and `self` must be the live pair of one thread, and the
    /// caller must serialize updates to them, as the thread lock does.
    #[must_use]
    pub unsafe fn delta(&mut self, timer: &Timer) -> c_uint {
        let low = timer.low_bits;
        if self.high != timer.high_bits_check {
            // SAFETY: the caller's contract; `timer_delta()` reads the
            // timer under its own coherency loop and updates the save.
            unsafe { glue::timer_delta(timer, self) }
        } else {
            let elapsed = low.wrapping_sub(self.low);
            self.low = low;
            elapsed
        }
    }
}

// `struct timer` is four `unsigned int`s and `struct timer_save` two,
// with the C compiler's offsets, on both x86 kernels.
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
