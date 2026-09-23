// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/kmutex.c and kern/kmutex.h:
//   Copyright (C) 2017 Free Software Foundation, Inc.
//   Contributed by Agustina Arzille <avarzille@riseup.net>, 2017.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel mutex, which `kern/kmutex.c` used to define.
//!
//! A three-state sleepable mutex: [`KMutex::try_lock()`] takes `Avail`
//! to `Locked` with one acquiring compare-exchange, and a failure drops
//! to the interlock, moves the state to `Contended` and sleeps on the
//! mutex's own address.  `kern/gsync.c` embeds one in each hash bucket,
//! so the record and the four `kmutex_*` symbols stay for C; nothing in
//! C reads `state` or `lock` directly.

use crate::arch::i386::percpu::current_thread;
use crate::kern::lock::SimpleLock;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, thread_sleep, thread_wakeup_prim,
};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

/// The three states of a mutex, the `KMUTEX_*` constants of
/// <kern/kmutex.h>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
enum State {
    /// `KMUTEX_AVAIL`: unowned.
    Avail = 0,
    /// `KMUTEX_LOCKED`: owned, with no known sleeper.
    Locked = 1,
    /// `KMUTEX_CONTENDED`: owned, with a sleeper to wake.
    Contended = 2,
}

impl State {
    /// The `unsigned int` the state is stored in.  A fieldless
    /// `repr(u32)` enum casts to its discriminant exactly.
    const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// `struct kmutex` of <kern/kmutex.h>: the three-state sleepable mutex.
///
/// # Invariants
///
/// `state` always holds one of the three `KMUTEX_*` values, and `lock`
/// serializes every slow path.  Both fields are private because C only
/// embeds the record and calls the four entry points.
#[repr(C)]
pub struct KMutex {
    state: AtomicU32,
    lock: SimpleLock,
}

// `struct kmutex` is the state word followed by the interlock; 8 bytes
// with the lock at offset 4 on both x86 kernels.
const _: () = assert!(size_of::<KMutex>() == 8);
const _: () = assert!(align_of::<KMutex>() == 4);
const _: () = assert!(offset_of!(KMutex, state) == 0);
const _: () = assert!(offset_of!(KMutex, lock) == 4);

impl KMutex {
    /// A fresh mutex: available, with an unlocked interlock.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: AtomicU32::new(State::Avail.as_u32()),
            lock: SimpleLock::new(),
        }
    }

    /// Try to acquire the mutex without sleeping.  `kmutex_trylock()`
    /// in C.
    ///
    /// The compare-exchange acquires on success and is `Relaxed` on
    /// failure, since the failure path takes the interlock before it
    /// reads anything the state protects.
    ///
    /// # Errors
    ///
    /// Returns [`KernError::Failure`] when the mutex is already held.
    pub fn try_lock(&self) -> Result<(), KernError> {
        if self
            .state
            .compare_exchange(
                State::Avail.as_u32(),
                State::Locked.as_u32(),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Ok(())
        } else {
            Err(KernError::Failure)
        }
    }

    /// Acquire the mutex, sleeping while it is held.  `kmutex_lock()`
    /// in C.
    ///
    /// The swap is `Acquire`, like the C `atomic_swap_acq()`, because
    /// it must see the previous owner's unlock.  A swap that reads
    /// `Avail` means the mutex was released while this side was taking
    /// the interlock.
    ///
    /// # Errors
    ///
    /// Returns [`KernError::Interrupted`] when `interruptible` is set
    /// and the sleep ends early; the mutex then belongs to its owner,
    /// which sets the state.
    pub fn lock(&self, interruptible: bool) -> Result<(), KernError> {
        if self.try_lock().is_ok() {
            return Ok(());
        }

        self.lock.lock();
        if self
            .state
            .swap(State::Contended.as_u32(), Ordering::Acquire)
            == State::Avail.as_u32()
        {
            // The mutex was released in between.
            self.lock.unlock();
            return Ok(());
        }

        // Sleep and check the result of the wait.  The owner sets the
        // state on every wakeup, so this side does not set it again.
        // SAFETY: this mutex is live and outlives the call, and the
        // interlock `thread_sleep()` is handed is the live second
        // field of the same record; the call releases it before
        // blocking, taking over the hold from above.
        unsafe {
            thread_sleep(
                ptr::from_ref(self).cast_mut().cast::<c_void>(),
                ptr::from_ref(&self.lock).cast_mut(),
                c_int::from(interruptible),
            );
        }

        // SAFETY: this is the thread that just slept, and
        // `current_thread()` reads it from the live per-CPU block.
        let wait_result = unsafe { (*current_thread()).wait_result };
        if wait_result == THREAD_AWAKENED {
            Ok(())
        } else {
            Err(KernError::Interrupted)
        }
    }

    /// Release the mutex, waking one sleeper when one is waiting.
    /// `kmutex_unlock()` in C.
    ///
    /// The compare-exchange releases on success, like the C
    /// `atomic_cas_rel()`; its failure is `Relaxed`, since the
    /// interlock orders the slow path.  The plain `state = AVAIL` the
    /// C makes under the interlock is a `Relaxed` store for the same
    /// reason: the interlock carries the ordering, and a woken thread
    /// takes it before it looks at the state.
    pub fn unlock(&self) {
        if self
            .state
            .compare_exchange(
                State::Locked.as_u32(),
                State::Avail.as_u32(),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            // No waiters.
            return;
        }

        self.lock.lock();

        // The C `thread_wakeup_one()`: wake the first thread waiting
        // on this mutex and report whether one was woken.  The return
        // is a `boolean_t`, so zero is the false case.
        // SAFETY: the event is this live mutex, the key its sleepers
        // registered with, and the caller owns it for the call.
        let woke = unsafe {
            thread_wakeup_prim(
                ptr::from_ref(self).cast_mut().cast::<c_void>(),
                1,
                THREAD_AWAKENED,
            )
        };

        if woke == 0 {
            // Every sleeper was interrupted and left; reset the state.
            self.state.store(State::Avail.as_u32(), Ordering::Relaxed);
        }

        self.lock.unlock();
    }
}

impl Default for KMutex {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize a mutex in caller storage.  `kmutex_init()` in C.
///
/// # Safety
///
/// A non-null `mtxp` must point at writable [`KMutex`] storage that no
/// other thread can see yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmutex_init(mtxp: *mut KMutex) {
    let Some(mtxp) = ptr::NonNull::new(mtxp) else {
        // A null mutex has no storage to write; the C would fault.
        return;
    };

    // SAFETY: the caller promises writable, unshared storage, and the
    // write covers the whole record.
    unsafe { mtxp.as_ptr().write(KMutex::new()) };
}

/// Acquire a mutex, sleeping while it is held.  `kmutex_lock()` in C.
///
/// # Safety
///
/// A non-null `mtxp` must point at a live initialized [`KMutex`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmutex_lock(
    mtxp: *mut KMutex,
    interruptible: c_int,
) -> c_int {
    let Some(mtxp) = ptr::NonNull::new(mtxp) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live initialized mutex.
    match unsafe { (*mtxp.as_ptr()).lock(interruptible != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Try to acquire a mutex without sleeping.  `kmutex_trylock()` in C.
///
/// # Safety
///
/// A non-null `mtxp` must point at a live initialized [`KMutex`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmutex_trylock(mtxp: *mut KMutex) -> c_int {
    let Some(mtxp) = ptr::NonNull::new(mtxp) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live initialized mutex.
    match unsafe { (*mtxp.as_ptr()).try_lock() } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// Release a mutex.  `kmutex_unlock()` in C.
///
/// # Safety
///
/// A non-null `mtxp` must point at a live initialized [`KMutex`] the
/// caller holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmutex_unlock(mtxp: *mut KMutex) {
    let Some(mtxp) = ptr::NonNull::new(mtxp) else {
        // A null mutex has no state to release; the C would fault.
        return;
    };

    // SAFETY: the caller promises a live initialized mutex it holds.
    unsafe { (*mtxp.as_ptr()).unlock() };
}
