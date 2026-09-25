// SPDX-License-Identifier: CMU-Mach
// Derived from device/chario.c:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The tty delayed-reply completion and PDMA table setup, which
//! `device/chario.c` used to define.

use crate::arch::i386::io_req::IoReq;
use crate::device::ds_routines_ffi::iodone;
use crate::glue;
use crate::kern::queue::QueueEntry;
use core::ffi::c_int;
use core::pin::Pin;
use core::ptr::NonNull;

/// The `B*` speed indices of <device/tty_status.h>, the rows of the `pdma_*`
/// tables.
const B0: usize = 0;
const B300: usize = 7;
const B600: usize = 8;
const B1200: usize = 9;
const B1800: usize = 10;
const B2400: usize = 11;
const B4800: usize = 12;
const B9600: usize = 13;
const EXTA: usize = 14;
const EXTB: usize = 15;
const B57600: usize = 16;
const B115200: usize = 17;

/// One row of the PDMA tick table: a speed's `B*` index and the baud rate its
/// timeout divides `hz` by.
const PDMA_TIMEOUTS: [(usize, c_int); 11] = [
    (B300, 30),
    (B600, 60),
    (B1200, 120),
    (B1800, 180),
    (B2400, 240),
    (B4800, 480),
    (B9600, 960),
    (EXTA, 1440),
    (EXTB, 1920),
    (B57600, 5760),
    (B115200, 11520),
];

/// The slow speeds' water marks, the integers `device/chario.c`'s floating
/// point expressions truncate to.
const PDMA_SLOW_MARKS: [(usize, c_int); 6] = [
    (B300, 24),
    (B600, 24),
    (B1200, 24),
    (B1800, 36),
    (B2400, 48),
    (B4800, 96),
];

/// The speeds that buffer half the input queue instead.
const PDMA_FAST_SPEEDS: [usize; 5] = [B9600, EXTA, EXTB, B57600, B115200];

const _: () = assert!(B115200 < glue::NSPEEDS);

/// Complete every request waiting on `head`.
///
/// # Safety
///
/// `head` must be an initialized queue head that stays at its address while
/// entries are linked, every entry must be an `io_req` record, and nothing
/// else may access the queue during the call.
unsafe fn complete(mut head: Pin<&mut QueueEntry>) {
    loop {
        // SAFETY: the caller's contract holds on every iteration, and `head`
        // is the same address throughout.
        let Some(ior) = (unsafe { head.as_mut().pop_front() }) else {
            return;
        };
        // SAFETY: every entry of these queues is an `io_req`, whose chain is
        // its first two fields.
        unsafe { iodone(ior.as_ptr().cast::<IoReq>()) };
    }
}

/// `tty_queue_completion()` in C.
///
/// # Safety
///
/// `queue` must be an initialized queue head that stays at its address while
/// entries are linked, every entry must be an `io_req` record, and nothing
/// else may access the queue during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tty_queue_completion(queue: *mut QueueEntry) {
    let Some(queue) = NonNull::new(queue) else {
        return;
    };
    // SAFETY: the caller promises `queue` is a valid, stable queue head; every
    // entry is an `io_req`.
    let head = unsafe { QueueEntry::pin_in_place(queue) };
    // SAFETY: as above; `complete` keeps the same contract.
    unsafe { complete(head) };
}

/// Half `tty_inq_size`, the water mark `device/chario.c` gives the fast lines.
fn half_input_queue() -> c_int {
    // SAFETY: `tty_inq_size` is the C constant device/chario.c defines; the
    // read is plain data.
    let size = unsafe { glue::tty_inq_size };
    // The C value is 4096, so half is far inside `c_int`; the C narrowed the
    // same unsigned half implicitly when it stored it in the `int` table.
    (size / 2) as c_int
}

/// The PDMA ticks and water marks `chario_init()` of device/chario.c set.
fn init_pdma_tables() {
    let timeouts = (&raw mut glue::pdma_timeouts).cast::<c_int>();
    let water_marks = (&raw mut glue::pdma_water_mark).cast::<c_int>();

    for speed in B0..B300 {
        // SAFETY: `speed` is below `B300`, and both C tables have `NSPEEDS`
        // rows; nothing else touches them during this boot-time call.
        unsafe {
            timeouts.add(speed).write(0);
            water_marks.add(speed).write(0);
        }
    }

    // SAFETY: `hz` is the live C global the clock probe set before
    // `device_service_create()` runs.
    let hz = unsafe { glue::hz };

    for (speed, baud) in PDMA_TIMEOUTS {
        // SAFETY: every speed is below `NSPEEDS`, the length of both tables.
        unsafe { timeouts.add(speed).write(hz / baud + 2) };
    }

    for (speed, mark) in PDMA_SLOW_MARKS {
        // SAFETY: every speed is below `NSPEEDS`, the length of both tables.
        unsafe { water_marks.add(speed).write(mark) };
    }

    let half_queue = half_input_queue();
    for speed in PDMA_FAST_SPEEDS {
        // SAFETY: every speed is below `NSPEEDS`, the length of both tables.
        unsafe { water_marks.add(speed).write(half_queue) };
    }
}

/// `chario_init()` in C.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during boot
/// before any tty is opened.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chario_init() {
    init_pdma_tables();
}
