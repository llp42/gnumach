// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_host.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The host, processor and processor-set ports, which `kern/ipc_host.c` used
//! to define and <kern/ipc_host.h> declares.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::types::KernError;
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::ptr;
use core::ptr::NonNull;

/// `IKOT_PROCESSOR` of <kern/ipc_kobject.h>: the object type of a processor's
/// control port.
const IKOT_PROCESSOR: c_uint = 5;
/// `IKOT_PSET`: the object type of a processor set's control port.
const IKOT_PSET: c_uint = 6;
/// `IKOT_PSET_NAME`: the object type of a set's name port.
const IKOT_PSET_NAME: c_uint = 7;
/// `IKOT_PROCESSOR_NAME`: the object type of a processor's name port.
const IKOT_PROCESSOR_NAME: c_uint = 29;
/// `IKOT_NONE`: the type of a port bound to no kernel object.
const IKOT_NONE: c_uint = 0;
/// `IKO_NULL`: the value that clears a port's `ip_kobject`.
const IKO_NULL: VmOffset = 0;

/// Allocate a special port in the kernel's IPC space, halting when the
/// allocator fails as the C callers did.
fn alloc_kernel_port(function: &CStr) -> NonNull<c_void> {
    // SAFETY: `ipc_space_kernel` is the live space `ipc_init()` built, and the
    // allocator takes its own locks.
    let port = unsafe { glue::ipc_port_alloc_special(glue::ipc_space_kernel) };
    let Some(port) = NonNull::new(port) else {
        // SAFETY: `Panic` does not return; the tags reproduce the C `panic()`
        // call's file, function and message.
        unsafe {
            glue::Panic(
                c"kern/ipc_host.c".as_ptr(),
                line!() as c_int,
                function.as_ptr(),
                function.as_ptr(),
            )
        }
    };
    port
}

/// `ipc_processor_init()` of kern/ipc_host.c.
fn processor_init(processor: &mut Processor) {
    let port = alloc_kernel_port(c"ipc_processor_init");
    processor.processor_self = port.as_ptr();
    // SAFETY: `port` is the special port just allocated and nothing else can
    // reach it yet; `processor` is the live object the C bound.
    unsafe {
        glue::ipc_kobject_set(
            port.as_ptr(),
            ptr::from_mut(processor).addr(),
            IKOT_PROCESSOR,
        );
    }

    let port = alloc_kernel_port(c"ipc_processor_init");
    processor.processor_name_self = port.as_ptr();
    // SAFETY: as above for the second, name port.
    unsafe {
        glue::ipc_kobject_set(
            port.as_ptr(),
            ptr::from_mut(processor).addr(),
            IKOT_PROCESSOR_NAME,
        );
    }
}

/// `ipc_pset_init()` of kern/ipc_host.c.
fn pset_init(pset: &mut ProcessorSet) {
    let port = alloc_kernel_port(c"ipc_pset_init");
    pset.pset_self = port.as_ptr();

    let port = alloc_kernel_port(c"ipc_pset_init");
    pset.pset_name_self = port.as_ptr();
}

/// `ipc_pset_enable()` of kern/ipc_host.c.
fn pset_enable(pset: &mut ProcessorSet) {
    pset.lock.lock();
    if pset.active != 0 {
        let pset_addr = ptr::from_mut(pset).addr();
        // SAFETY: the set lock is held, so the two port fields are the ports
        // `pset_init()` allocated and nothing is deallocating them.
        unsafe {
            glue::ipc_kobject_set(pset.pset_self, pset_addr, IKOT_PSET);
            glue::ipc_kobject_set(
                pset.pset_name_self,
                pset_addr,
                IKOT_PSET_NAME,
            );
        }
        pset.ref_lock.lock();
        pset.ref_count = pset.ref_count.wrapping_add(2);
        pset.ref_lock.unlock();
    }
    pset.lock.unlock();
}

/// `ipc_pset_disable()` of kern/ipc_host.c.
fn pset_disable(pset: &mut ProcessorSet) {
    // SAFETY: the caller holds the set lock and a reference, as the C
    // required, so the two port fields are live.
    unsafe {
        glue::ipc_kobject_set(pset.pset_self, IKO_NULL, IKOT_NONE);
        glue::ipc_kobject_set(pset.pset_name_self, IKO_NULL, IKOT_NONE);
    }
    pset.ref_count = pset.ref_count.wrapping_sub(2);
}

/// `ipc_pset_terminate()` of kern/ipc_host.c.
fn pset_terminate(pset: &mut ProcessorSet) {
    // SAFETY: the set is dead, so nothing else may use the two ports, and
    // `ipc_space_kernel` is the live space `ipc_init()` built.
    unsafe {
        glue::ipc_port_dealloc_special(pset.pset_self, glue::ipc_space_kernel);
        glue::ipc_port_dealloc_special(
            pset.pset_name_self,
            glue::ipc_space_kernel,
        );
    }
}

/// `processor_set_default()` of kern/ipc_host.c.
fn set_default(host: *mut c_void) -> Result<*mut ProcessorSet, KernError> {
    if host.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: `default_pset` is the C global `pset_sys_bootstrap()`
    // initialized during the boot; only its address is formed, because the
    // Rust mirror stops before the NCPUS-sized tail.
    let pset = ptr::addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
    // SAFETY: the global is live from boot, and the entry is reached only once
    // IPC is up.
    unsafe { (*pset).reference() };
    Ok(pset)
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
    unsafe { processor_init(&mut *processor) };
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
    unsafe { pset_init(&mut *pset) };
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
    unsafe { pset_enable(&mut *pset) };
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
    unsafe { pset_disable(&mut *pset) };
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
    unsafe { pset_terminate(&mut *pset) };
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
    match set_default(host) {
        Ok(default) => {
            // SAFETY: the caller promises a valid out-parameter.
            unsafe { *pset = default };
            0
        }
        Err(error) => c_int::from(error),
    }
}
