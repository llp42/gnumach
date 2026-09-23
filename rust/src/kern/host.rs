// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon
//   University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The kernel version reports of `kern/host.c`, mirroring
//! <mach/host_info.h>.
//!
//! Both are MIG routines of <mach/mach_host.defs>, and both answer
//! with the string `version.c` builds from the package name and
//! version.  [`host_kernel_version`] is the deprecated spelling and
//! exists on i386 only, as the `.defs` gates it.
//!
//! The rest of `kern/host.c` stays C: it reads `struct host`,
//! `struct machine_info` and the NCPUS-sized load averages, none of
//! which Rust mirrors yet.

use crate::glue;
use crate::kern::types::KernError;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::slice;

/// `KERNEL_VERSION_MAX` of <mach/host_info.h>: the bytes a
/// `kernel_version_t` holds.
pub const KERNEL_VERSION_MAX: usize = 512;

/// Fill `out` with `version`, truncating and zero-filling the rest
/// exactly as the C `strncpy()` over a `kernel_version_t` did.
fn copy_version(out: &mut [u8], version: &CStr) {
    let bytes = version.to_bytes();
    let copied = bytes.len().min(out.len());
    let (head, tail) = out.split_at_mut(copied);
    head.copy_from_slice(&bytes[..copied]);
    tail.fill(0);
}

/// Writes the kernel version string into `out_version`.
/// `host_get_kernel_version()` of kern/host.c.
///
/// Answers `KERN_INVALID_ARGUMENT` for a null host and
/// `KERN_SUCCESS` otherwise, as the C did.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and
/// `out_version` must be valid for writes of [`KERNEL_VERSION_MAX`]
/// bytes.  MIG's `_Xhost_get_kernel_version` passes the reply
/// message's `kernel_version_t` field, which is exactly that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_get_kernel_version(
    host: *mut c_void,
    out_version: *mut c_char,
) -> c_int {
    if host.is_null() {
        return c_int::from(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises `KERNEL_VERSION_MAX` writable bytes
    // at `out_version`, and `version` below is a distinct object.
    let out = unsafe {
        slice::from_raw_parts_mut(out_version.cast::<u8>(), KERNEL_VERSION_MAX)
    };
    // SAFETY: `version` is the NUL-terminated string constant that
    // version.c defines, so the walk to its terminator stays inside
    // it.
    let version = unsafe { CStr::from_ptr(&raw const glue::version) };

    copy_version(out, version);

    0
}

/// Writes the kernel version string into `out_version`.
/// `host_kernel_version()` of kern/host.c.
///
/// The deprecated spelling of [`host_get_kernel_version`], which it
/// forwards to unchanged.  <mach/mach_host.defs> declares the routine
/// only for `__i386__`, so the x86_64 kernel does not define it.
///
/// # Safety
///
/// The same contract as [`host_get_kernel_version`].
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_kernel_version(
    host: *mut c_void,
    out_version: *mut c_char,
) -> c_int {
    // SAFETY: the caller's contract is the one this passes on.
    unsafe { host_get_kernel_version(host, out_version) }
}
