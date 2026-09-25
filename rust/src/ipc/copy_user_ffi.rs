// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from ipc/copy_user.c:
//   Copyright (C) 2023 Free Software Foundation
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` edge of the user-message copy: the one symbol
//! `ipc/copy_user.c` still exported on the LP64 kernel.

use crate::ipc::copy_user;
use core::ffi::{c_int, c_void};

/// `copyinmsg()` of `ipc/copy_user.c`.
///
/// # Safety
///
/// `userbuf` must be readable for `usize` bytes, and `kernelbuf` must point
/// at a writable buffer of `usize` bytes holding at least one message header.
/// `_ksize` is that buffer's size, which the C ignores on the 64-bit kernel.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn copyinmsg(
    userbuf: *const c_void,
    kernelbuf: *mut c_void,
    usize: usize,
    _ksize: usize,
) -> c_int {
    // SAFETY: the caller's contract.
    if unsafe { copy_user::copy_in(userbuf, kernelbuf.cast(), usize) }.is_err()
    {
        1
    } else {
        0
    }
}
