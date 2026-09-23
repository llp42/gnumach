// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The <mach/mach_host.defs> server entries that have moved to Rust,
//! 4 of its 44 routines.
//!
//! MIG's `_X` stubs call these by the names the `.defs` gives them;
//! the cores are in [`crate::kern::host`].

use crate::arch::types::VmOffset;
use crate::config::{KERNEL_VERSION, KERNEL_VERSION_MAX};
use crate::kern::host::{Host, processor_ports, processor_set_priv};
use crate::kern::processor::ProcessorSet;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint};
use core::ptr::NonNull;

const _: () = assert!(KERNEL_VERSION.len() < KERNEL_VERSION_MAX);

/// Both the name and the 512 come from <mach/mach_host.defs>, whose
/// reply is a `c_string[512]`.
#[unsafe(no_mangle)]
pub extern "C" fn host_get_kernel_version(
    host: Option<NonNull<Host>>,
    out_version: Option<&mut [u8; KERNEL_VERSION_MAX]>,
) -> c_int {
    let (Some(_), Some(out)) = (host, out_version) else {
        return c_int::from(KernError::InvalidArgument);
    };

    let (version, pad) = out.split_at_mut(KERNEL_VERSION.len());
    version.copy_from_slice(KERNEL_VERSION.as_bytes());
    pad.fill(0);

    0
}

/// The deprecated spelling of [`host_get_kernel_version`].
#[unsafe(no_mangle)]
pub extern "C" fn host_kernel_version(
    host: Option<NonNull<Host>>,
    out_version: Option<&mut [u8; KERNEL_VERSION_MAX]>,
) -> c_int {
    host_get_kernel_version(host, out_version)
}

/// `host_processor_set_priv()` of kern/host.c.
#[unsafe(no_mangle)]
pub extern "C" fn host_processor_set_priv(
    host: Option<NonNull<Host>>,
    pset_name: Option<&mut ProcessorSet>,
    pset: Option<&mut Option<NonNull<ProcessorSet>>>,
) -> c_int {
    let Some(out) = pset else {
        return c_int::from(KernError::InvalidArgument);
    };

    match processor_set_priv(host, pset_name) {
        Ok(set) => {
            *out = Some(set);
            0
        }
        Err(error) => {
            // The C stores the null set on this path too.
            *out = None;
            c_int::from(error)
        }
    }
}

/// `processor_set_processors()` of kern/host.c.
#[unsafe(no_mangle)]
pub extern "C" fn processor_set_processors(
    pset: Option<&mut ProcessorSet>,
    processor_list: Option<&mut Option<NonNull<VmOffset>>>,
    countp: Option<&mut c_uint>,
) -> c_int {
    let (Some(pset), Some(out_list), Some(out_count)) =
        (pset, processor_list, countp)
    else {
        return c_int::from(KernError::InvalidArgument);
    };

    match processor_ports(pset) {
        Ok((list, count)) => {
            *out_list = Some(list);
            *out_count = count;
            0
        }
        Err(error) => c_int::from(error),
    }
}
