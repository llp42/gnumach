// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

// TODO: this module is shaped by its C callers -- the `#[repr(C)]` event
// mirror, the `c_int` indices and the five `extern "C"` wrappers exist
// for `i386/i386at/kd_event.c` (kbd_queue) and
// `i386/i386at/kd_mouse.c` (mouse_queue), plus the user-mode copy in
// tests/kd_queue.c.  Once the two drivers are ported, review this file
// to turn it into a pure Rust queue: a native event type and the safe
// `KdEventQueue` API, with no FFI surface.  (MIGRATE.md lists both
// drivers as not yet moved.)

//! The keyboard/mouse event ring buffer, which `i386/i386at/kd_queue.c`
//! used to define.
//!
//! A fixed `KDQSIZE`-slot queue with a read and a write index.  One
//! slot stays free, so it holds at most `KDQSIZE - 1` events and the
//! indices alone tell a full queue from an empty one.  The callers
//! serialize access by raising the interrupt level (`SPLKD`).
//!
//! `push_back()` copies an event into the queue; `pop_front()` returns
//! a reference to the slot it left.  `kdq_get()` hands that reference
//! out as a pointer, good only until the caller next touches the queue
//! -- the old contract, preserved because the C callers copy the event
//! out at `SPLKD` before unlocking.  Unlike the C, `kdq_get()` on an
//! empty queue returns null and does not advance the index; every
//! caller checks `kdq_empty()` first.
//!
//! The `KdEvent` mirror matches `kd_event` of <device/input.h> in the
//! default kernel configuration.  `--enable-user32` redefines
//! `rpc_long_integer_t` to `int32_t` through a configure define Rust
//! cannot see, which would make `kd_event` smaller; only the default
//! configuration is mirrored, as in `src/kern/elf_load.rs`.

use core::ffi::{c_int, c_long};
use core::mem::{offset_of, size_of};
use core::ptr;

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
}

impl KdEventQueue {
    /// A queue with every slot zeroed, as the C's BSS image of a
    /// `kd_event_queue` is: for `#[no_mangle]` statics a C file used
    /// to define.
    pub const fn new() -> Self {
        Self {
            events: [KdEvent::zeroed(); KDQSIZE],
            firstfree: 0,
            firstout: 0,
        }
    }

    /// The slot after `index`, wrapping at `KDQSIZE`: `q_next()` in C.
    fn next(index: c_int) -> c_int {
        (index + 1) % KDQSIZE as c_int
    }

    /// Whether the queue holds no events.  `kdq_empty()` in C.
    pub fn is_empty(&self) -> bool {
        self.firstfree == self.firstout
    }

    /// Whether the queue holds its most, `KDQSIZE - 1` events:
    /// `kdq_full()` in C, which leaves one slot free so that a full
    /// queue and an empty one cannot look alike.
    pub fn is_full(&self) -> bool {
        Self::next(self.firstfree) == self.firstout
    }

    /// Make the queue empty.  `kdq_reset()` in C.
    pub fn clear(&mut self) {
        self.firstfree = 0;
        self.firstout = 0;
    }

    /// Copy `ev` into the free slot and advance the write index.
    /// `kdq_put()` in C; the caller has checked `is_full()`.
    pub fn push_back(&mut self, ev: KdEvent) {
        self.events[self.firstfree as usize] = ev;
        self.firstfree = Self::next(self.firstfree);
    }

    /// Advance the read index and return the slot it left, or `None`
    /// when the queue is empty.  `kdq_get()` in C.
    pub fn pop_front(&mut self) -> Option<&mut KdEvent> {
        if self.is_empty() {
            return None;
        }
        let result = &mut self.events[self.firstout as usize];
        self.firstout = Self::next(self.firstout);
        Some(result)
    }

    /// View a caller-owned queue through a shared reference.
    ///
    /// # Safety
    ///
    /// `q` must point at a valid, aligned `kd_event_queue` that
    /// nothing else accesses for the duration of the borrow.
    unsafe fn from_ptr<'a>(q: *const Self) -> &'a Self {
        // SAFETY: the caller promises `q` is valid and unaliased.
        unsafe { &*q }
    }

    /// View a caller-owned queue through a mutable reference.
    ///
    /// # Safety
    ///
    /// Same contract as `from_ptr()`.
    unsafe fn from_ptr_mut<'a>(q: *mut Self) -> &'a mut Self {
        // SAFETY: the caller promises `q` is valid and unaliased.
        unsafe { &mut *q }
    }
}

/// Whether `q` holds no events.  `kdq_empty()` in C.
///
/// # Safety
///
/// `q` must point at a valid, unaliased `kd_event_queue`: the C callers
/// hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdq_empty(q: *const KdEventQueue) -> c_int {
    // SAFETY: the caller promises `q` is valid and unaliased.
    c_int::from(unsafe { KdEventQueue::from_ptr(q) }.is_empty())
}

/// Whether `q` holds `KDQSIZE - 1` events.  `kdq_full()` in C.
///
/// # Safety
///
/// `q` must point at a valid, unaliased `kd_event_queue`: the C callers
/// hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdq_full(q: *const KdEventQueue) -> c_int {
    // SAFETY: the caller promises `q` is valid and unaliased.
    c_int::from(unsafe { KdEventQueue::from_ptr(q) }.is_full())
}

/// Copy `ev` into `q`'s free slot.  `kdq_put()` in C.
///
/// # Safety
///
/// `q` and `ev` must point at valid, aligned objects; `q` must not be
/// full; and nothing else may access `q` during the call: the C
/// callers hold `SPLKD` and have checked `kdq_full()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdq_put(q: *mut KdEventQueue, ev: *mut KdEvent) {
    // SAFETY: the caller promises `ev` is valid for a read.
    let ev = unsafe { *ev };
    // SAFETY: the caller promises `q` is valid, not full, and
    // unaliased.
    unsafe { KdEventQueue::from_ptr_mut(q) }.push_back(ev);
}

/// Advance `q`'s read index and return the slot it left, or null when
/// the queue is empty.  `kdq_get()` in C.
///
/// # Safety
///
/// `q` must point at a valid, unaliased `kd_event_queue`; the callers
/// hold `SPLKD` and have checked `kdq_empty()`.  The returned pointer
/// points into `q`, and the caller must copy the event out before it
/// next touches the queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdq_get(q: *mut KdEventQueue) -> *mut KdEvent {
    // SAFETY: the caller promises `q` is valid, unaliased, and not
    // empty.
    unsafe { KdEventQueue::from_ptr_mut(q) }
        .pop_front()
        .map_or(ptr::null_mut(), ptr::from_mut)
}

/// Make `q` empty.  `kdq_reset()` in C.
///
/// # Safety
///
/// `q` must point at a valid, unaliased `kd_event_queue`, and nothing
/// else may access it during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdq_reset(q: *mut KdEventQueue) {
    // SAFETY: the caller promises `q` is valid and unaliased.
    unsafe { KdEventQueue::from_ptr_mut(q) }.clear();
}
