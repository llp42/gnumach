// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/mach_clock.h:
//   Copyright (C) 2006, 2007 Free Software Foundation, Inc.
// Derived from kern/mach_clock.c:
//   Copyright (c) 1994-1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel timeout element, which `kern/mach_clock.h` declares,
//! the clock readback and host time adjustments, and the `/dev/time`
//! open and close entries.
//!
//! Of the timeout machinery, only `struct timeout` and its
//! `TIMEOUT_*` bits move here.  `kern/mach_clock.c` is still C and
//! keeps touching the same record through the C macro spellings; the
//! scheduler reaches it through [`reset_timeout_check()`].
//!
//! [`read_time_stamp()`] turns a boot-time timestamp back into the
//! real-time frame; `record_time_stamp()` and the mapped-time page stay
//! C.  [`host_set_time()`], [`host_adjust_time()`] and
//! [`host_adjust_time64()`] are the host clock's set and gradual
//! adjustment entries, with `host_set_time64()` still C.
//!
//! [`timeopen()`] and [`timeclose()`] are the device-switch entries
//! `i386/i386at/conf.c` puts in the `timename` row.  `/dev/time` is
//! the mapped-time page and nothing else: there is no per-open state
//! to build or tear down, so opening always succeeds and closing has
//! nothing to do.

use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::i386::percpu::{current_processor, current_thread};
use crate::device::r#return::{DeviceSuccess, IoResultExt};
use crate::glue;
use crate::glue::time_value::{
    MACH_ADJTIME_NSECS_OMIT, TimeValue, TimeValue64,
};
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::thread_bind;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr::{self, NonNull};

/// `struct timeout` of <kern/mach_clock.h>: a kernel timeout element.
#[repr(C)]
pub struct Timeout {
    /// `chain`: links the element into the timeout queue.
    pub chain: QueueEntry,
    /// `fcn`: the routine called at expiry.
    pub fcn: Option<unsafe extern "C" fn(*mut c_void)>,
    /// `param`: the argument passed to `fcn`.
    pub param: *mut c_void,
    /// `t_time`: the expiration time, in ticks since boot.
    pub t_time: usize,
    /// `set`: the `TIMEOUT_*` bits.
    pub set: u8,
}

/// `TIMEOUT_ALLOC` in <kern/mach_clock.h>: allocated from the pool.
pub const TIMEOUT_ALLOC: u8 = 0x1;
pub const TIMEOUT_ACTIVE: u8 = 0x2;
pub const TIMEOUT_PENDING: u8 = 0x4;

/// `reset_timeout_check()` of <kern/mach_clock.h>: cancel the timeout
/// if one is active.
///
/// # Safety
///
/// The caller holds the lock protecting `t`, so it is stable and only
/// its owner can have set it.
pub(crate) unsafe fn reset_timeout_check(t: *mut Timeout) {
    // SAFETY: the caller's contract; `set` is a plain byte field.
    if unsafe { (*t).set } & TIMEOUT_ACTIVE != 0 {
        // SAFETY: the same contract, and `reset_timeout()` takes the
        // element off the timeout queue at splsched.
        unsafe { glue::reset_timeout(t.cast()) };
    }
}

/// Read a timestamp back into the real-time clock frame.
/// `read_time_stamp()` of kern/mach_clock.c.
///
/// `record_time_stamp()` stays C and adds the same offset; the
/// subtraction is [`TimeValue64::sub()`] of <mach/time_value.h>.
///
/// # Safety
///
/// `stamp` must point at a readable [`TimeValue64`] and `result` at
/// writable storage for one.  They may be the same object, which the
/// C's `*result = *stamp` allowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read_time_stamp(
    stamp: *const TimeValue64,
    result: *mut TimeValue64,
) {
    // SAFETY: the caller promises `stamp` is readable.
    let value = unsafe { stamp.read() };
    // SAFETY: `clock_boottime_offset` is the live C global that
    // `clock_boottime_update()` maintains at splhigh and the C
    // `record_time_stamp()` adds back; the reader takes one value of
    // it, exactly as the C `read_time_stamp()` did.
    let offset = unsafe { glue::clock_boottime_offset };
    // SAFETY: the caller promises `result` is writable; `value` is a
    // copy, so the write cannot disturb the read above.
    unsafe { result.write(value.sub(offset)) };
}

/// `MICROSECONDS_IN_ONE_SECOND` in kern/mach_clock.c: the microseconds
/// the gradual adjustment counts in.
const MICROSECONDS_IN_ONE_SECOND: c_int = 1_000_000;

/// The body of `host_adjust_time64()` in kern/mach_clock.c: bind to the
/// master CPU, then read and rewrite the gradual-adjustment globals at
/// `splclock()`, answering the outstanding adjustment.
///
/// A null host is [`KernError::InvalidHost`]; the value returned on
/// success is the adjustment the call replaced.
fn adjust_time(
    host: Option<NonNull<c_void>>,
    new_adjustment: TimeValue64,
) -> Result<TimeValue64, KernError> {
    if host.is_none() {
        return Err(KernError::InvalidHost);
    }

    let thread = current_thread();
    // SAFETY: `master_processor` is the live C global the boot pointed
    // at the master slot; the C compares and passes the same value.
    let master = unsafe { glue::master_processor };
    // SAFETY: `thread` is the live current thread and `master` the
    // live master processor; `thread_bind()` only stores the pairing
    // under the thread lock.
    unsafe { thread_bind(thread, master) };

    if current_processor() != master {
        // SAFETY: the thread is bound to `master`, so the block
        // resumes there; the C passed `thread_no_continuation`, a null
        // continuation.
        unsafe { glue::thread_block(None) };
    }

    // SAFETY: `splclock()` is the real asm routine of <machine/spl.h>,
    // and its return value is only handed back to `splx()`.
    let s = unsafe { glue::splclock() };

    // The C reads the outstanding adjustment before deciding whether to
    // replace it, all under the clock lock.
    // SAFETY: the clock interrupt is held off, so `timedelta` cannot
    // change under this read, and it is the live C global.
    let timedelta = unsafe { glue::timedelta };
    let old = TimeValue64 {
        seconds: i64::from(timedelta / MICROSECONDS_IN_ONE_SECOND),
        nanoseconds: i64::from(timedelta % MICROSECONDS_IN_ONE_SECOND) * 1000,
    };

    if new_adjustment.nanoseconds != MACH_ADJTIME_NSECS_OMIT {
        // SAFETY: as above; the tuning globals are the C ones and the
        // clock lock still holds.
        unsafe {
            // The C product is `int64_t`; the wrapping spellings keep
            // an out-of-range argument defined where the C overflow
            // was not.
            let mut ndelta = new_adjustment
                .seconds
                .wrapping_mul(i64::from(MICROSECONDS_IN_ONE_SECOND))
                .wrapping_add(new_adjustment.nanoseconds / 1000);

            if glue::timedelta == 0 {
                // The C tests against the `unsigned bigadj`, and its
                // `-bigadj` negates in `unsigned` too, so the value
                // widens to `int64_t` as 2^32 - bigadj.  Keep that
                // spelling: it is the comparison the C compiled.
                if ndelta > i64::from(glue::bigadj)
                    || ndelta < i64::from(glue::bigadj.wrapping_neg())
                {
                    // The C product is `unsigned`; the assignment
                    // narrows it to the `int` tickdelta, which the
                    // cast spells.
                    glue::tickdelta = glue::tickadj.wrapping_mul(10) as c_int;
                } else {
                    glue::tickdelta = glue::tickadj as c_int;
                }
            }

            // Make `ndelta` a multiple of `tickdelta`, as the C does.
            let tickdelta = i64::from(glue::tickdelta);
            if ndelta % tickdelta != 0 {
                ndelta = ndelta / tickdelta * tickdelta;
            }
            // The C assignment narrows `int64_t` to the `int`
            // timedelta, which the cast spells.
            glue::timedelta = ndelta as c_int;
        }
    }

    // SAFETY: `s` is the level `splclock()` returned.
    unsafe { glue::splx(s) };

    // SAFETY: `thread` is the live current thread and the null is the
    // C `PROCESSOR_NULL`, the unbind the C performed.
    unsafe { thread_bind(thread, ptr::null_mut()) };

    Ok(old)
}

/// Set the wall clock with the legacy microseconds record.
/// `host_set_time()` of kern/mach_clock.c.
///
/// The C converted the record with `TIME_VALUE_TO_TIME_VALUE64()` and
/// forwarded it to `host_set_time64()`, which stays C.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`; MIG's
/// `_Xhost_set_time` passes the host private port's host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_set_time(
    host: *mut c_void,
    new_time: TimeValue,
) -> c_int {
    // SAFETY: the caller promises the host pointer, and the C
    // `host_set_time64()` takes the converted record exactly as the C
    // entry handed it over.
    unsafe { glue::host_set_time64(host, TimeValue64::from(new_time)) }
}

/// Adjust the wall clock gradually with the legacy microseconds
/// record.  `host_adjust_time()` of kern/mach_clock.c.
///
/// The C converted both records and called `host_adjust_time64()`; the
/// body is the shared [`adjust_time()`].
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and
/// `old_adjustment` must be valid for a write on success; MIG's
/// `_Xhost_adjust_time` passes its reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time(
    host: *mut c_void,
    new_adjustment: TimeValue,
    old_adjustment: *mut TimeValue,
) -> c_int {
    match adjust_time(NonNull::new(host), TimeValue64::from(new_adjustment)) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for
            // a write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(TimeValue::from(old)) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// Adjust the wall clock gradually.  `host_adjust_time64()` of
/// kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and
/// `old_adjustment` must be valid for a write on success; MIG's
/// `_Xhost_adjust_time64` passes its reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time64(
    host: *mut c_void,
    new_adjustment: TimeValue64,
    old_adjustment: *mut TimeValue64,
) -> c_int {
    match adjust_time(NonNull::new(host), new_adjustment) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for
            // a write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(old) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// Open `/dev/time`.  `timeopen()` of kern/mach_clock.c.
///
/// The C ignores its arguments and always succeeds; the request is
/// never read.  `/dev/time` carries no per-open state, so there is
/// nothing to build.
#[unsafe(no_mangle)]
pub extern "C" fn timeopen(
    _dev: DevT,
    _flag: c_int,
    _ior: *mut IoReq,
) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// Close `/dev/time`.  `timeclose()` of kern/mach_clock.c.
///
/// The C ignores its arguments and returns nothing: [`timeopen()`]
/// built no state, so there is none to release.
#[unsafe(no_mangle)]
pub extern "C" fn timeclose(_dev: DevT, _flag: c_int) {}

// `struct timeout`: the queue chain, the callback pair, the expiry and
// the state byte.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Timeout>() == 48);
    assert!(offset_of!(Timeout, chain) == 0);
    assert!(offset_of!(Timeout, fcn) == 16);
    assert!(offset_of!(Timeout, param) == 24);
    assert!(offset_of!(Timeout, t_time) == 32);
    assert!(offset_of!(Timeout, set) == 40);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Timeout>() == 24);
    assert!(offset_of!(Timeout, chain) == 0);
    assert!(offset_of!(Timeout, fcn) == 8);
    assert!(offset_of!(Timeout, param) == 12);
    assert!(offset_of!(Timeout, t_time) == 16);
    assert!(offset_of!(Timeout, set) == 20);
};
