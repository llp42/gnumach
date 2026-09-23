// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/mach_clock.h:
//   Copyright (C) 2006, 2007 Free Software Foundation, Inc.
// Derived from kern/mach_clock.c:
//   Copyright (c) 1994-1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel timeout element, which `kern/mach_clock.h` declares, the clock
//! readback and host time adjustments, and the `/dev/time` open and close
//! entries.

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

/// `reset_timeout_check()` of <kern/mach_clock.h>: cancel the timeout if one
/// is active.
///
/// # Safety
///
/// The caller holds the lock protecting `t`, so it is stable and only its
/// owner can have set it.
pub(crate) unsafe fn reset_timeout_check(t: *mut Timeout) {
    // SAFETY: the caller's contract; `set` is a plain byte field.
    if unsafe { (*t).set } & TIMEOUT_ACTIVE != 0 {
        // SAFETY: the same contract, and `reset_timeout()` takes the element
        // off the timeout queue at splsched.
        unsafe { glue::reset_timeout(t.cast()) };
    }
}

/// `read_time_stamp()` of kern/mach_clock.c.
///
/// # Safety
///
/// `stamp` must point at a readable [`TimeValue64`] and `result` at writable
/// storage for one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read_time_stamp(
    stamp: *const TimeValue64,
    result: *mut TimeValue64,
) {
    // SAFETY: the caller promises `stamp` is readable.
    let value = unsafe { stamp.read() };
    // SAFETY: `clock_boottime_offset` is the live C global that
    // `clock_boottime_update()` maintains at splhigh and the C
    // `record_time_stamp()` adds back; the reader takes one value of it,
    // exactly as the C `read_time_stamp()` did.
    let offset = unsafe { glue::clock_boottime_offset };
    // SAFETY: the caller promises `result` is writable; `value` is a copy, so
    // the write cannot disturb the read above.
    unsafe { result.write(value.sub(offset)) };
}

/// `MICROSECONDS_IN_ONE_SECOND` in kern/mach_clock.c: the microseconds the
/// gradual adjustment counts in.
const MICROSECONDS_IN_ONE_SECOND: c_int = 1_000_000;

/// The body of `host_adjust_time64()` in kern/mach_clock.c: bind to the master
/// CPU, then read and rewrite the gradual-adjustment globals at `splclock()`,
/// answering the outstanding adjustment.
fn adjust_time(
    host: Option<NonNull<c_void>>,
    new_adjustment: TimeValue64,
) -> Result<TimeValue64, KernError> {
    if host.is_none() {
        return Err(KernError::InvalidHost);
    }

    let thread = current_thread();
    // SAFETY: `master_processor` is the live C global the boot pointed at the
    // master slot; the C compares and passes the same value.
    let master = unsafe { glue::master_processor };
    // SAFETY: `thread` is the live current thread and `master` the live master
    // processor; `thread_bind()` only stores the pairing under the thread
    // lock.
    unsafe { thread_bind(thread, master) };

    if current_processor() != master {
        // SAFETY: the thread is bound to `master`, so the block resumes there;
        // the C passed `thread_no_continuation`, a null continuation.
        unsafe { glue::thread_block(None) };
    }

    // SAFETY: `splclock()` is the real asm routine of <machine/spl.h>, and its
    // return value is only handed back to `splx()`.
    let s = unsafe { glue::splclock() };

    // SAFETY: the clock interrupt is held off, so `timedelta` cannot change
    // under this read, and it is the live C global.
    let timedelta = unsafe { glue::timedelta };
    let old = TimeValue64 {
        seconds: i64::from(timedelta / MICROSECONDS_IN_ONE_SECOND),
        nanoseconds: i64::from(timedelta % MICROSECONDS_IN_ONE_SECOND) * 1000,
    };

    if new_adjustment.nanoseconds != MACH_ADJTIME_NSECS_OMIT {
        // SAFETY: as above; the tuning globals are the C ones and the clock
        // lock still holds.
        unsafe {
            let mut ndelta = new_adjustment
                .seconds
                .wrapping_mul(i64::from(MICROSECONDS_IN_ONE_SECOND))
                .wrapping_add(new_adjustment.nanoseconds / 1000);

            if glue::timedelta == 0 {
                if ndelta > i64::from(glue::bigadj)
                    || ndelta < i64::from(glue::bigadj.wrapping_neg())
                {
                    // The C product is `unsigned`; the assignment narrows it
                    // to the `int` tickdelta, which the cast spells.
                    glue::tickdelta = glue::tickadj.wrapping_mul(10) as c_int;
                } else {
                    glue::tickdelta = glue::tickadj as c_int;
                }
            }

            let tickdelta = i64::from(glue::tickdelta);
            if ndelta % tickdelta != 0 {
                ndelta = ndelta / tickdelta * tickdelta;
            }
            // The C assignment narrows `int64_t` to the `int` timedelta, which
            // the cast spells.
            glue::timedelta = ndelta as c_int;
        }
    }

    // SAFETY: `s` is the level `splclock()` returned.
    unsafe { glue::splx(s) };

    // SAFETY: `thread` is the live current thread and the null is the C
    // `PROCESSOR_NULL`, the unbind the C performed.
    unsafe { thread_bind(thread, ptr::null_mut()) };

    Ok(old)
}

/// `host_set_time()` of kern/mach_clock.c.
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
    // `host_set_time64()` takes the converted record exactly as the C entry
    // handed it over.
    unsafe { glue::host_set_time64(host, TimeValue64::from(new_time)) }
}

/// `host_adjust_time()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and `old_adjustment`
/// must be valid for a write on success; MIG's `_Xhost_adjust_time` passes its
/// reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time(
    host: *mut c_void,
    new_adjustment: TimeValue,
    old_adjustment: *mut TimeValue,
) -> c_int {
    match adjust_time(NonNull::new(host), TimeValue64::from(new_adjustment)) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for a
            // write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(TimeValue::from(old)) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_adjust_time64()` of kern/mach_clock.c.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and `old_adjustment`
/// must be valid for a write on success; MIG's `_Xhost_adjust_time64` passes
/// its reply field, which is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_adjust_time64(
    host: *mut c_void,
    new_adjustment: TimeValue64,
    old_adjustment: *mut TimeValue64,
) -> c_int {
    match adjust_time(NonNull::new(host), new_adjustment) {
        Ok(old) => {
            // SAFETY: the caller promises `old_adjustment` is valid for a
            // write, and the C wrote it on the success path only.
            unsafe { old_adjustment.write(old) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `timeopen()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn timeopen(
    _dev: DevT,
    _flag: c_int,
    _ior: *mut IoReq,
) -> c_int {
    Ok(DeviceSuccess::Success).as_io_return()
}

/// `timeclose()` of kern/mach_clock.c.
#[unsafe(no_mangle)]
pub extern "C" fn timeclose(_dev: DevT, _flag: c_int) {}

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
