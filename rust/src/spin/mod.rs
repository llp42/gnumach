#![deny(missing_docs)]

//! Spin-based synchronization primitives for the kernel.
//!
//! Vendored from the `spin` crate (<https://codeberg.org/zesterer/spin>)
//! and adapted to build as a module of `mach_rs`: the Cargo features and
//! the `lock_api`, `std` and `portable-atomic` integrations are gone, so
//! every primitive is compiled unconditionally.  The upstream MIT license
//! is in the `LICENSE` file beside this one.
//!
//! This module provides [spin-based](https://en.wikipedia.org/wiki/Spinlock)
//! versions of the primitives in `std::sync`. Because synchronization is
//! done through spinning, the primitives are suitable for use in a kernel.
//!
//! # Relationship with `std::sync`
//!
//! While `spin` is not a drop-in replacement for `std::sync` (and
//! [should not be considered as such](https://matklad.github.io/2020/01/02/spinlocks-considered-harmful.html))
//! an effort is made to keep this module reasonably consistent with `std::sync`.
//!
//! Many of the types defined here have 'additional capabilities' when compared to `std::sync`:
//!
//! - Because spinning does not depend on the thread-driven model of `std::sync`, guards ([`MutexGuard`],
//!   [`RwLockReadGuard`], [`RwLockWriteGuard`], etc.) may be sent and shared between threads.
//!
//! - [`RwLockUpgradableGuard`] supports being upgraded into a [`RwLockWriteGuard`].
//!
//! - Guards support [leaking](https://doc.rust-lang.org/nomicon/leaking.html).
//!
//! - [`Once`] owns the value returned by its `call_once` initializer.
//!
//! - [`RwLock`] supports counting readers and writers.
//!
//! Conversely, the types in this module do not have some of the features `std::sync` has:
//!
//! - Locks do not track [panic poisoning](https://doc.rust-lang.org/nomicon/poisoning.html).

use core::sync::atomic;

pub mod barrier;
pub mod lazylock;
pub mod mutex;
pub mod once;
pub mod relax;
pub mod rwlock;

pub use mutex::MutexGuard;
pub use relax::{RelaxStrategy, Spin};
pub use rwlock::RwLockReadGuard;

// Avoid confusing inference errors by aliasing away the relax strategy parameter. Users that need to use a different
// relax strategy can do so by accessing the types through their fully-qualified path. This is a little bit horrible
// but sadly adding a default type parameter is *still* a breaking change in Rust (for understandable reasons).

/// A primitive that synchronizes the execution of multiple threads. See [`barrier::Barrier`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type Barrier = self::barrier::Barrier;

/// A value which is initialized on the first access. See [`lazylock::LazyLock`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type LazyLock<T, F = fn() -> T> = self::lazylock::LazyLock<T, F>;

/// A type alias to [`LazyLock`] for compatibility reasons.
///
#[deprecated(note = "use `spin::LazyLock` instead")]
pub type Lazy<T, F = fn() -> T> = self::lazylock::LazyLock<T, F>;

/// A primitive that synchronizes the execution of multiple threads. See [`mutex::Mutex`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type Mutex<T> = self::mutex::Mutex<T>;

/// A primitive that provides lazy one-time initialization. See [`once::Once`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type Once<T = ()> = self::once::Once<T>;

/// A lock that provides data access to either one writer or many readers. See [`rwlock::RwLock`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type RwLock<T> = self::rwlock::RwLock<T>;

/// A guard that provides immutable data access but can be upgraded to [`RwLockWriteGuard`]. See
/// [`rwlock::RwLockUpgradableGuard`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type RwLockUpgradableGuard<'a, T> =
    self::rwlock::RwLockUpgradableGuard<'a, T>;

/// A guard that provides mutable data access. See [`rwlock::RwLockWriteGuard`] for documentation.
///
/// A note for advanced users: this alias exists to avoid subtle type inference errors due to the default relax
/// strategy type parameter. If you need a non-default relax strategy, use the fully-qualified path.
pub type RwLockWriteGuard<'a, T> = self::rwlock::RwLockWriteGuard<'a, T>;

/// In the event of an invalid operation, it's best to abort the current process.
fn abort() -> ! {
    // Panicking while panicking is defined by Rust to result in an abort.
    struct Panic;

    impl Drop for Panic {
        fn drop(&mut self) {
            panic!("aborting due to invalid operation");
        }
    }

    let _panic = Panic;
    panic!("aborting due to invalid operation");
}
