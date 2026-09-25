// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon
//   University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `kern/host.c` symbols C still calls, over the cores in
//! [`crate::kern::host`].

use crate::arch::types::VmOffset;
use crate::kern::host::{self, Host};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::NonNull;
use core::slice;

/// `host_processors()` of kern/host.c, the routine <mach/mach_host.defs>
/// declares.
///
/// # Safety
///
/// `host` must be `HOST_NULL` or the live host pointer the generated server
/// converted the request port into; `processor_list` and `countp` must be
/// valid out-parameters.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_processors(
    host: *mut Host,
    processor_list: *mut *mut VmOffset,
    countp: *mut c_uint,
) -> c_int {
    match host::processors(NonNull::new(host)) {
        Ok((list, count)) => {
            // SAFETY: the caller promises both out-parameters are valid.
            unsafe {
                *processor_list = list.as_ptr();
                *countp = count;
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_info()` of kern/host.c.
///
/// # Safety
///
/// `host` must be `HOST_NULL` or the live host pointer the generated server
/// converted the request port into; `info` must be readable and writable for
/// `*count` integers, and `count` must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_info(
    host: *mut Host,
    flavor: c_int,
    info: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    let (Some(host), Some(count)) = (NonNull::new(host), NonNull::new(count))
    else {
        return c_int::from(KernError::InvalidArgument);
    };
    let Some(info) = NonNull::new(info) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a writable count.
    let capacity = unsafe { count.as_ptr().read() as usize };
    // SAFETY: the MIG stubs clamp the count to `HOST_INFO_MAX` and back it
    // with a `host_info_data_t`, and the caller promises the same: `info` is
    // writable for the count it supplied.
    let buffer = unsafe {
        slice::from_raw_parts_mut(
            info.as_ptr(),
            capacity.min(host::HOST_INFO_MAX),
        )
    };
    // SAFETY: the caller promises a live host.
    let present = unsafe { host.as_ref() };

    match host::info(Some(present), flavor, buffer) {
        Ok(elements) => {
            // SAFETY: the caller promises a writable count.
            unsafe { count.as_ptr().write(elements) };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `host_processor_sets()` of kern/host.c, the routine <mach/mach_host.defs>
/// declares.
///
/// # Safety
///
/// `host` must be `HOST_NULL` or the live host pointer the generated server
/// converted the request port into; `pset_list` and `count` must be valid
/// out-parameters.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_processor_sets(
    host: *mut Host,
    pset_list: *mut *mut c_void,
    count: *mut c_uint,
) -> c_int {
    let Some(host) = NonNull::new(host) else {
        return c_int::from(KernError::InvalidArgument);
    };

    // SAFETY: the caller promises a live host.
    match unsafe { host::processor_sets(Some(host.as_ref())) } {
        Ok((list, actual)) => {
            // SAFETY: the caller promises both out-parameters are valid.
            unsafe {
                *pset_list = list.cast();
                *count = actual;
            }
            0
        }
        Err(error) => c_int::from(error),
    }
}
