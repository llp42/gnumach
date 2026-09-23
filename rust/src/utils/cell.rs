// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `SyncCell`: a singleton cell for the kernel's C-shaped mutable state.

use core::cell::UnsafeCell;

/// A singleton cell whose `Sync` promise the caller's interrupt level makes
/// true.
pub(crate) struct SyncCell<T>(pub(crate) UnsafeCell<T>);

// SAFETY: the user serializes every access at its documented level.
unsafe impl<T> Sync for SyncCell<T> {}
