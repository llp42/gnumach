// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/kd_queue.c and i386/i386at/kd_queue.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The keyboard/mouse event ring buffer, which `i386/i386at/kd_queue.c`
//! used to define.
//!
//! A fixed `KDQSIZE`-slot queue with a read and a write index.  One
//! slot stays free, so it holds at most `KDQSIZE - 1` events and the
//! indices alone tell a full queue from an empty one.  Its callers
//! (`i386/i386at/kd_event.rs` and `kd_mouse.rs`) serialize access by
//! raising the interrupt level (`SPLKD`).
//!
//! `push_back()` copies an event into the queue; `pop_front()` returns
//! a reference to the slot it left.  `KdEvent` is how those drivers
//! build the records the user side reads, so its layout still mirrors
//! `kd_event` of <device/input.h> in the default kernel configuration.
//! `--enable-user32` redefines `rpc_long_integer_t` to `int32_t`
//! through a configure define Rust cannot see, which would make
//! `kd_event` smaller; only the default configuration is mirrored, as
//! in `src/kern/elf_load.rs`.

use core::ffi::{c_int, c_long};
use core::mem::{offset_of, size_of};

/// `KDQSIZE` in <i386at/kd_queue.h>.
const KDQSIZE: usize = 100;

/// `rpc_time_value` of <mach/time_value.h> as the kernel compiles it.
///
/// `rpc_long_integer_t` is C `long_integer_t`, i.e. `long`, which
/// `c_long` mirrors on both supported targets.  The field is obsolete
/// (kept for user ABI compatibility) and never read here: this struct
/// exists only so that `KdEvent`'s size and alignment come out right.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct RpcTimeValue {
    seconds: c_long,
    microseconds: c_int,
}

/// `kev_type` of <device/input.h>: an event type.
pub type KevType = u16;

/// `Scancode` of <device/input.h>: a keyboard scan code.
pub type Scancode = u8;

/// `struct mouse_motion` of <device/input.h>.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct MouseMotion {
    pub mm_delta_x: i16,
    pub mm_delta_y: i16,
}

/// The `value` union of `kd_event` in <device/input.h>.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
union KdValue {
    up: c_int,
    sc: u8,
    mmotion: MouseMotion,
}

/// `kd_event` of <device/input.h>, field for field.
///
/// The fields are never read here: the queue copies whole events, and
/// the layout is all this module needs.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct KdEvent {
    type_: u16,
    unused_time: RpcTimeValue,
    value: KdValue,
}

/// `kd_event_queue` of <i386at/kd_queue.h>.
#[repr(C)]
pub struct KdEventQueue {
    events: [KdEvent; KDQSIZE],
    firstfree: c_int,
    firstout: c_int,
}

// The C layouts, pinned so that a change to the mirror cannot drift from
// the header silently.  `kd_event` is 16 bytes on i686, and 32 on x86_64
// where `long_integer_t` is 64 bits; the two counts below come from
// `kd_event` times `KDQSIZE` plus the two indices.
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<KdEvent>() == 16);
    assert!(size_of::<KdEventQueue>() == 1608);
};
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<KdEvent>() == 32);
    assert!(size_of::<KdEventQueue>() == 3208);
};
const _: () = assert!(
    offset_of!(KdEventQueue, firstfree) == KDQSIZE * size_of::<KdEvent>()
);
const _: () = assert!(
    offset_of!(KdEventQueue, firstout)
        == offset_of!(KdEventQueue, firstfree) + size_of::<c_int>()
);

impl Default for KdEventQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl KdEvent {
    /// An all-zero event, the image BSS holds before anything is
    /// queued; only `KdEventQueue::new()` needs it.
    const fn zeroed() -> Self {
        Self {
            type_: 0,
            unused_time: RpcTimeValue {
                seconds: 0,
                microseconds: 0,
            },
            value: KdValue { up: 0 },
        }
    }

    /// A `MOUSE_MOTION` event carrying `moved`.
    pub const fn motion(moved: MouseMotion) -> Self {
        Self {
            type_: 4,
            unused_time: RpcTimeValue {
                seconds: 0,
                microseconds: 0,
            },
            value: KdValue { mmotion: moved },
        }
    }

    /// A button event of type `which`, pressed when `up` is false:
    /// what `mouse_button()` builds.  `up` becomes the C `boolean_t`
    /// the callers read.
    pub const fn button(which: KevType, up: bool) -> Self {
        Self {
            type_: which,
            unused_time: RpcTimeValue {
                seconds: 0,
                microseconds: 0,
            },
            value: KdValue { up: up as c_int },
        }
    }

    /// A `KEYBD_EVENT` carrying scancode `sc`: what `kd_enqsc()`
    /// builds.
    pub const fn scancode(sc: Scancode) -> Self {
        Self {
            type_: 5,
            unused_time: RpcTimeValue {
                seconds: 0,
                microseconds: 0,
            },
            value: KdValue { sc },
        }
    }
}

impl KdEventQueue {
    /// An empty queue, the image the drivers' statics start from.
    pub const fn new() -> Self {
        Self {
            events: [KdEvent::zeroed(); KDQSIZE],
            firstfree: 0,
            firstout: 0,
        }
    }

    /// The slot after `index`, wrapping at `KDQSIZE`.
    fn next(index: c_int) -> c_int {
        (index + 1) % KDQSIZE as c_int
    }

    /// Whether the queue holds no events.
    pub fn is_empty(&self) -> bool {
        self.firstfree == self.firstout
    }

    /// Whether the queue holds its most, `KDQSIZE - 1` events: one slot
    /// stays free so that a full queue and an empty one cannot look
    /// alike.
    pub fn is_full(&self) -> bool {
        Self::next(self.firstfree) == self.firstout
    }

    /// Make the queue empty.
    pub fn clear(&mut self) {
        self.firstfree = 0;
        self.firstout = 0;
    }

    /// Copy `ev` into the free slot and advance the write index; the
    /// caller has checked `is_full()`.
    pub fn push_back(&mut self, ev: KdEvent) {
        self.events[self.firstfree as usize] = ev;
        self.firstfree = Self::next(self.firstfree);
    }

    /// Advance the read index and return the slot it left, or `None`
    /// when the queue is empty.
    pub fn pop_front(&mut self) -> Option<&mut KdEvent> {
        if self.is_empty() {
            return None;
        }
        let result = &mut self.events[self.firstout as usize];
        self.firstout = Self::next(self.firstout);
        Some(result)
    }
}
