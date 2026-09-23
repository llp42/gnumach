// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Locks of `kern/lock.h`, the machine-independent half.
//!
//! `SimpleLock` mirrors `struct slock`: one `natural_t`, 0 unlocked
//! and 1 locked.  The C `simple_lock`/`simple_unlock` macros in
//! <i386/lock.h> use a locked `xchg`; the methods here use
//! `AtomicU32::swap` with `AcqRel`, so the critical section a lock
//! acquired release-publishes is acquired by the next locker on every
//! supported architecture.  On x86 the compiler lowers both to the
//! same instruction.
//!
//! `LockData` mirrors `struct lock`, the sleep-capable recursive lock
//! whose operations stay in C (`kern/lock.c`) for now; only its layout
//! is shared, because `struct vm_map` embeds one at offset 0 and the
//! Rust side has to name the field.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

/// A simple spin lock, layout-identical to `struct slock` of
/// <kern/lock.h>.
///
/// The zero value is unlocked, the one value locked; C reads and
/// writes the same word through its `lock_data` member.
#[repr(transparent)]
pub struct SimpleLock {
    lock_data: AtomicU32,
}

// `struct slock` is one `volatile natural_t`, and `natural_t` is
// `unsigned int` on both x86 kernels.
const _: () = assert!(size_of::<SimpleLock>() == size_of::<u32>());
const _: () = assert!(align_of::<SimpleLock>() == align_of::<u32>());

impl SimpleLock {
    /// An unlocked lock, the image a C `static` began with.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lock_data: AtomicU32::new(0),
        }
    }

    /// Reset the lock to unlocked.  `simple_lock_init()` in C.
    ///
    /// Only valid before the lock is first used or after the caller
    /// has proved that no one holds it.  The store is `Relaxed`
    /// because the caller's proof, not an ordering, is what makes it
    /// safe.
    pub fn init(&self) {
        self.lock_data.store(0, Ordering::Relaxed);
    }

    /// Whether the lock is currently held, as a non-synchronizing
    /// query.  The load is `Relaxed`: the answer may be stale by the
    /// time the caller acts on it, so it only informs diagnostics or
    /// a try-again hint.
    pub fn is_locked(&self) -> bool {
        self.lock_data.load(Ordering::Relaxed) != 0
    }

    /// Acquire the lock, spinning while it is held.  The C
    /// `simple_lock()` macro.
    ///
    /// The outer `swap` is the `xchg` of the C macro: it takes the
    /// lock and acquires the releasing unlock's publishes.  The inner
    /// load is its test-and-test-and-set read of the same word and
    /// may be `Relaxed`, because only the `swap` result decides
    /// whether the lock was taken.
    pub fn lock(&self) {
        while self.lock_data.swap(1, Ordering::AcqRel) != 0 {
            while self.lock_data.load(Ordering::Relaxed) != 0 {
                core::hint::spin_loop();
            }
        }
    }

    /// Try to acquire the lock, reporting whether it was taken.
    /// `simple_lock_try()` in C.
    ///
    /// Like `lock()`, the `swap` acquires on success.
    #[must_use]
    pub fn try_lock(&self) -> bool {
        self.lock_data.swap(1, Ordering::AcqRel) == 0
    }

    /// Release the lock.  `simple_unlock()` in C.
    ///
    /// The `AcqRel` store publishes everything this critical section
    /// wrote to the next successful locker.
    pub fn unlock(&self) {
        self.lock_data.swap(0, Ordering::AcqRel);
    }
}

impl Default for SimpleLock {
    fn default() -> Self {
        Self::new()
    }
}

/// The sleep-capable recursive lock of `kern/lock.h`, layout-identical
/// to `struct lock`.
///
/// `thread`, the bitfield word and the interlock are all the C side
/// touches; the Rust side only has to name the member inside
/// `struct vm_map`.  The word packs
/// `read_count:16, want_upgrade:1, want_write:1, waiting:1,
/// can_sleep:1, recursion_depth:12`, in that order from the least
/// significant bit.
#[repr(C)]
pub struct LockData {
    thread: *mut c_void,
    state: u32,
    interlock: SimpleLock,
}

// The bitfield word is one `unsigned int` on the i386 ABI and the
// pointer is target-sized, so the layout is pointer + word + interlock
// on both kernels.  These are `struct lock`'s C sizes.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<LockData>() == 16);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<LockData>() == 12);
const _: () = assert!(align_of::<LockData>() == align_of::<*mut c_void>());
const _: () = assert!(core::mem::offset_of!(LockData, thread) == 0);
const _: () = assert!(
    core::mem::offset_of!(LockData, interlock)
        == core::mem::offset_of!(LockData, state) + size_of::<u32>()
);
