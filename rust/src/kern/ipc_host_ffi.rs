// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_host.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `kern/ipc_host.c` symbols C still calls, over the cores in
//! [`crate::kern::ipc_host`].

use crate::kern::host::Host;
use crate::kern::ipc_host;
use crate::kern::processor::{Processor, ProcessorSet};
use core::ffi::{c_int, c_uint, c_void};

/// `ipc_host_init()` of kern/ipc_host.c.
///
/// # Safety
///
/// Runs once from the boot sequence, before any port or thread can be
/// looked up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_host_init() {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::init() };
}

/// `mach_host_self()` of kern/ipc_host.c.
///
/// # Safety
///
/// Runs on the caller's own thread once `ipc_host_init()` has built the host
/// port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_host_self() -> c_uint {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::mach_host_self() }
}

/// `ipc_processor_init()` of kern/ipc_host.c.
///
/// # Safety
///
/// `processor` must point at a live `struct processor` whose two port fields
/// no other thread can reach yet, as `pset_sys_init()` leaves each slot during
/// the boot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_processor_init(processor: *mut Processor) {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::processor_init(&mut *processor) };
}

/// `ipc_pset_init()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` whose two port fields no
/// other thread can reach yet, as `processor_set_create()` and the default-set
/// bootstrap leave it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_init(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_init(&mut *pset) };
}

/// `ipc_pset_enable()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` whose ports
/// [`ipc_pset_init()`] built, and the caller must hold a reference to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_enable(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_enable(&mut *pset) };
}

/// `ipc_pset_disable()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` that has not been
/// terminated, and the caller must hold its lock and a reference to it, as the
/// C required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_disable(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_disable(&mut *pset) };
}

/// `ipc_pset_terminate()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live `struct processor_set` that no other thread may
/// use any more, and its two port fields must be the ports [`ipc_pset_init()`]
/// built and [`ipc_pset_disable()`] unbound.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_pset_terminate(pset: *mut ProcessorSet) {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_terminate(&mut *pset) };
}

/// `processor_set_default()` of kern/ipc_host.c.
///
/// # Safety
///
/// `host` must be null or point at a live `struct host`, and `pset` must be a
/// valid out-parameter; MIG's `_Xprocessor_set_default` passes both.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_set_default(
    host: *mut c_void,
    pset: *mut *mut ProcessorSet,
) -> c_int {
    match unsafe { ipc_host::set_default(host) } {
        Ok(default) => {
            // SAFETY: the caller promises a valid out-parameter.
            unsafe { *pset = default };
            0
        }
        Err(error) => c_int::from(error),
    }
}

/// `convert_port_to_host()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_host(port: *mut c_void) -> *mut Host {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_host(port) }
}

/// `convert_port_to_host_priv()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_host_priv(
    port: *mut c_void,
) -> *mut Host {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_host_priv(port) }
}

/// `convert_host_to_port()` of kern/ipc_host.c.
///
/// # Safety
///
/// `host` must point at a live `struct host`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_host_to_port(host: *mut Host) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::host_to_port(host) }
}

/// `convert_port_to_processor()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_processor(
    port: *mut c_void,
) -> *mut Processor {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_processor(port) }
}

/// `convert_port_to_processor_name()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_processor_name(
    port: *mut c_void,
) -> *mut Processor {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_processor_name(port) }
}

/// `convert_processor_to_port()` of kern/ipc_host.c.
///
/// # Safety
///
/// `processor` must point at a live `struct processor` whose control port
/// [`ipc_processor_init()`] built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_processor_to_port(
    processor: *mut Processor,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::processor_to_port(processor) }
}

/// `convert_processor_name_to_port()` of kern/ipc_host.c.
///
/// # Safety
///
/// `processor` must point at a live `struct processor` whose name port
/// [`ipc_processor_init()`] built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_processor_name_to_port(
    processor: *mut Processor,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::processor_name_to_port(processor) }
}

/// `convert_port_to_pset()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_pset(
    port: *mut c_void,
) -> *mut ProcessorSet {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_pset(port) }
}

/// `convert_port_to_pset_name()` of kern/ipc_host.c.
///
/// # Safety
///
/// `port` must be null or a live port pointer `IP_VALID()` accepts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_port_to_pset_name(
    port: *mut c_void,
) -> *mut ProcessorSet {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::port_to_pset_name(port) }
}

/// `convert_pset_to_port()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live, referenced `struct processor_set`; the call
/// consumes the reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_pset_to_port(
    pset: *mut ProcessorSet,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_to_port(pset) }
}

/// `convert_pset_name_to_port()` of kern/ipc_host.c.
///
/// # Safety
///
/// `pset` must point at a live, referenced `struct processor_set`; the call
/// consumes the reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert_pset_name_to_port(
    pset: *mut ProcessorSet,
) -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { ipc_host::pset_name_to_port(pset) }
}
