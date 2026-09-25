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
use crate::ipc::{IpcPort, IpcSpace, ipc_port, ipc_space};
use crate::kern::host::{self, Host};
use crate::kern::processor::{self, Processor, ProcessorSet};
use crate::kern::task::current_task;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr;
use core::ptr::NonNull;

/// `IKOT_HOST` of <kern/ipc_kobject.h>: the object type of the host port.
const IKOT_HOST: c_uint = 3;
/// `IKOT_HOST_PRIV`: the object type of the host privilege port.
const IKOT_HOST_PRIV: c_uint = 4;
/// `IKOT_PROCESSOR`: the object type of a processor's control port.
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
/// `MACH_PORT_NULL` of <mach/port.h>.
const MACH_PORT_NULL: c_uint = 0;

/// Allocate a special port in the kernel's IPC space, halting when the
/// allocator fails as the C callers did.
fn alloc_kernel_port(function: &core::ffi::CStr) -> NonNull<c_void> {
    // SAFETY: the kernel's space is live from `ipc_init()` on, and the
    // allocator takes its own locks.
    let port = unsafe { ipc_port::alloc_special(ipc_space::kernel()) };
    let Some(port) = port else {
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
    // SAFETY: `IpcPort` always holds a non-null pointer.
    unsafe { NonNull::new_unchecked(port.as_ptr()) }
}

/// `ipc_host_init()` of kern/ipc_host.c: the two host ports, the default
/// set's two ports, and the master processor's two ports.
pub(crate) unsafe fn init() {
    let port = alloc_kernel_port(c"ipc_host_init");
    // SAFETY: `realhost` is the one host object, and the freshly allocated
    // port can be bound to it before anything else can reach it.
    unsafe {
        glue::ipc_kobject_set(
            port.as_ptr(),
            host::realhost().addr(),
            IKOT_HOST,
        );
        (*host::realhost()).host_self = port.as_ptr();
    }

    let port = alloc_kernel_port(c"ipc_host_init");
    // SAFETY: as above for the privilege port.
    unsafe {
        glue::ipc_kobject_set(
            port.as_ptr(),
            host::realhost().addr(),
            IKOT_HOST_PRIV,
        );
        (*host::realhost()).host_priv_self = port.as_ptr();
    }

    let pset = processor::default_pset();
    // SAFETY: the default set and the master processor are the globals
    // `pset_sys_bootstrap()` initialized during the boot, and their port
    // fields are still null, so no other thread can be looking at them.
    unsafe {
        pset_init(&mut *pset);
        pset_enable(&mut *pset);
        processor_init(&mut *processor::master_processor());
    }
}

/// `mach_host_self()` of kern/ipc_host.c: a send right for the caller's own
/// host port.
pub(crate) unsafe fn mach_host_self() -> c_uint {
    // SAFETY: `realhost` is the live host object.
    let host_self = unsafe { (*host::realhost()).host_self };
    let Some(port) = IpcPort::valid(host_self) else {
        return MACH_PORT_NULL;
    };

    // SAFETY: the host port is live and active from `ipc_host_init()` on.
    let sright = unsafe { ipc_port::make_send(port) };
    // SAFETY: the running task is live, and its space is set before any IPC
    // call the task can make; the `current_space()` macro.
    let space = unsafe { IpcSpace::from_raw((*current_task()).itk_space) };
    // SAFETY: the right is live and belongs to this task's space; the
    // successful copyout consumes it.
    unsafe { ipc_port::copyout_send(sright.as_ptr(), space) }
}

/// `ipc_processor_init()` of kern/ipc_host.c.
pub(crate) fn processor_init(processor: &mut Processor) {
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
pub(crate) fn pset_init(pset: &mut ProcessorSet) {
    let port = alloc_kernel_port(c"ipc_pset_init");
    pset.pset_self = port.as_ptr();

    let port = alloc_kernel_port(c"ipc_pset_init");
    pset.pset_name_self = port.as_ptr();
}

/// `ipc_pset_enable()` of kern/ipc_host.c.
pub(crate) fn pset_enable(pset: &mut ProcessorSet) {
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
pub(crate) fn pset_disable(pset: &mut ProcessorSet) {
    // SAFETY: the caller holds the set lock and a reference, as the C
    // required, so the two port fields are live.
    unsafe {
        glue::ipc_kobject_set(pset.pset_self, IKO_NULL, IKOT_NONE);
        glue::ipc_kobject_set(pset.pset_name_self, IKO_NULL, IKOT_NONE);
    }
    pset.ref_count = pset.ref_count.wrapping_sub(2);
}

/// `ipc_pset_terminate()` of kern/ipc_host.c.
pub(crate) fn pset_terminate(pset: &mut ProcessorSet) {
    // SAFETY: the set is dead, so nothing else may use the two ports, which
    // are live special-space ports.
    unsafe {
        ipc_port::dealloc_special(IpcPort::from_raw(pset.pset_self));
        ipc_port::dealloc_special(IpcPort::from_raw(pset.pset_name_self));
    }
}

/// `convert_port_to_host()` of kern/ipc_host.c.
pub(crate) unsafe fn port_to_host(port: *mut c_void) -> *mut Host {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: the caller promises a port pointer that `IP_VALID()` accepts;
    // the lock makes the kobject stable.
    unsafe {
        port.lock();
        let host = if port.is_active()
            && (port.kotype() == IKOT_HOST || port.kotype() == IKOT_HOST_PRIV)
        {
            port.kobject().cast::<Host>()
        } else {
            ptr::null_mut()
        };
        port.unlock();
        host
    }
}

/// `convert_port_to_host_priv()` of kern/ipc_host.c.
pub(crate) unsafe fn port_to_host_priv(port: *mut c_void) -> *mut Host {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: as `port_to_host()`.
    unsafe {
        port.lock();
        let host = if port.is_active() && port.kotype() == IKOT_HOST_PRIV {
            port.kobject().cast::<Host>()
        } else {
            ptr::null_mut()
        };
        port.unlock();
        host
    }
}

/// `convert_port_to_processor()` of kern/ipc_host.c.
pub(crate) unsafe fn port_to_processor(port: *mut c_void) -> *mut Processor {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: as `port_to_host()`.
    unsafe {
        port.lock();
        let processor = if port.is_active() && port.kotype() == IKOT_PROCESSOR
        {
            port.kobject().cast::<Processor>()
        } else {
            ptr::null_mut()
        };
        port.unlock();
        processor
    }
}

/// `convert_port_to_processor_name()` of kern/ipc_host.c.
pub(crate) unsafe fn port_to_processor_name(
    port: *mut c_void,
) -> *mut Processor {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: as `port_to_host()`.
    unsafe {
        port.lock();
        let processor = if port.is_active()
            && (port.kotype() == IKOT_PROCESSOR
                || port.kotype() == IKOT_PROCESSOR_NAME)
        {
            port.kobject().cast::<Processor>()
        } else {
            ptr::null_mut()
        };
        port.unlock();
        processor
    }
}

/// `convert_port_to_pset()` of kern/ipc_host.c: the set with one more
/// reference, or null.
pub(crate) unsafe fn port_to_pset(port: *mut c_void) -> *mut ProcessorSet {
    // SAFETY: the caller's contract.
    unsafe { port_to_pset_kind(port, IKOT_PSET) }
}

/// `convert_port_to_pset_name()` of kern/ipc_host.c: the set with one more
/// reference, or null.
pub(crate) unsafe fn port_to_pset_name(
    port: *mut c_void,
) -> *mut ProcessorSet {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: as `port_to_host()`; the lock keeps the set alive while its
    // reference is taken.
    unsafe {
        port.lock();
        let pset = if port.is_active()
            && (port.kotype() == IKOT_PSET || port.kotype() == IKOT_PSET_NAME)
        {
            let pset = port.kobject().cast::<ProcessorSet>();
            (*pset).reference();
            pset
        } else {
            ptr::null_mut()
        };
        port.unlock();
        pset
    }
}

/// The shared body of `convert_port_to_pset()`: check the set's object type
/// and take a reference.
unsafe fn port_to_pset_kind(
    port: *mut c_void,
    kotype: c_uint,
) -> *mut ProcessorSet {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: as `port_to_host()`.
    unsafe {
        port.lock();
        let pset = if port.is_active() && port.kotype() == kotype {
            let pset = port.kobject().cast::<ProcessorSet>();
            (*pset).reference();
            pset
        } else {
            ptr::null_mut()
        };
        port.unlock();
        pset
    }
}

/// `convert_host_to_port()` of kern/ipc_host.c: a naked send right, or the
/// invalid pointer unchanged.
pub(crate) unsafe fn host_to_port(host: *mut Host) -> *mut c_void {
    // SAFETY: the caller promises a live host.
    let port = unsafe { (*host).host_self };
    match IpcPort::valid(port) {
        // SAFETY: the host port is live and active from `ipc_host_init()`
        // on.
        Some(port) => unsafe { ipc_port::make_send(port) }.as_ptr(),
        None => port,
    }
}

/// `convert_processor_to_port()` of kern/ipc_host.c: a naked send right for
/// the processor's control port.
pub(crate) unsafe fn processor_to_port(
    processor: *mut Processor,
) -> *mut c_void {
    // SAFETY: the caller promises a live processor.
    let port = unsafe { (*processor).processor_self };
    match IpcPort::valid(port) {
        // SAFETY: the port is live and active from `ipc_processor_init()`
        // on.
        Some(port) => unsafe { ipc_port::make_send(port) }.as_ptr(),
        None => port,
    }
}

/// `convert_processor_name_to_port()` of kern/ipc_host.c: a naked send right
/// for the processor's name port.
pub(crate) unsafe fn processor_name_to_port(
    processor: *mut Processor,
) -> *mut c_void {
    // SAFETY: the caller promises a live processor.
    let port = unsafe { (*processor).processor_name_self };
    match IpcPort::valid(port) {
        // SAFETY: as `processor_to_port()`.
        Some(port) => unsafe { ipc_port::make_send(port) }.as_ptr(),
        None => port,
    }
}

/// `convert_pset_to_port()` of kern/ipc_host.c: consumes the caller's set
/// reference and returns a naked send right, or null when the set is dead.
pub(crate) unsafe fn pset_to_port(pset: *mut ProcessorSet) -> *mut c_void {
    // SAFETY: the caller promises a live, referenced set.
    let pset = unsafe { &mut *pset };
    pset.lock.lock();
    let port = if pset.active != 0 {
        match IpcPort::valid(pset.pset_self) {
            // SAFETY: an active set's port is live from `pset_init()` on.
            Some(port) => unsafe { ipc_port::make_send(port) }.as_ptr(),
            None => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    };
    pset.lock.unlock();

    pset.deallocate();
    port
}

/// `convert_pset_name_to_port()` of kern/ipc_host.c: consumes the caller's
/// set reference and returns its name port as a naked send right, or null
/// when the set is dead.
pub(crate) unsafe fn pset_name_to_port(
    pset: *mut ProcessorSet,
) -> *mut c_void {
    // SAFETY: the caller promises a live, referenced set.
    let pset = unsafe { &mut *pset };
    pset.lock.lock();
    let port = if pset.active != 0 {
        match IpcPort::valid(pset.pset_name_self) {
            // SAFETY: an active set's name port is live from `pset_init()`
            // on.
            Some(port) => unsafe { ipc_port::make_send(port) }.as_ptr(),
            None => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    };
    pset.lock.unlock();

    pset.deallocate();
    port
}

/// `processor_set_default()` of kern/ipc_host.c: the default set with one
/// more reference.
pub(crate) unsafe fn set_default(
    host: *mut c_void,
) -> Result<*mut ProcessorSet, KernError> {
    if host.is_null() {
        return Err(KernError::InvalidArgument);
    }

    let pset = processor::default_pset();
    // SAFETY: the global is live from boot, and the entry is reached only once
    // IPC is up.
    unsafe { (*pset).reference() };
    Ok(pset)
}
