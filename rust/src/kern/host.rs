// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon
//   University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The cores of `kern/host.c`, mirroring <kern/host.h>.

use crate::arch::i386::percpu::percpu_at;
use crate::arch::types::VmOffset;
use crate::config::NCPUS;
use crate::glue;
use crate::kern::machine;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::{queue_end, queue_first, queue_next};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr;
use core::ptr::NonNull;

/// `struct host` of <kern/host.h>, the host object MIG hands the host
/// routines.
#[repr(C)]
pub struct Host {
    pub host_self: *mut c_void,
    pub host_priv_self: *mut c_void,
}

/// The body of `host_processor_set_priv()` in kern/host.c: a live host and a
/// live name set give back the same set with one more reference.
pub(crate) fn processor_set_priv(
    host: Option<NonNull<Host>>,
    name: Option<&mut ProcessorSet>,
) -> Result<NonNull<ProcessorSet>, KernError> {
    match (host, name) {
        (Some(_), Some(set)) => {
            set.reference();
            Ok(NonNull::from(set))
        }
        _ => Err(KernError::InvalidArgument),
    }
}

/// The body of `processor_set_processors()` in kern/host.c: allocate the array
/// MIG sends back, walk the set's processor queue and convert each processor
/// to its name port.
pub(crate) fn processor_ports(
    pset: &mut ProcessorSet,
) -> Result<(NonNull<VmOffset>, c_uint), KernError> {
    pset.lock.lock();

    // The C read the `int` count into an `unsigned int`; the field is
    // maintained as the number of processors, so it is small and never
    // negative.
    let count = pset.processor_count as c_uint;
    // A `c_uint` count fits a `VmSize` on both supported targets, so the
    // widening cannot lose a bit.
    let count_slots = count as usize;
    let size = count_slots * size_of::<VmOffset>();

    // SAFETY: `kalloc_init()` ran during the boot this MIG entry follows, and
    // the size is the C expression's.
    let base = unsafe { glue::kalloc(size) };
    let Some(ports) =
        NonNull::new(ptr::with_exposed_provenance_mut::<*mut c_void>(base))
    else {
        pset.lock.unlock();
        return Err(KernError::ResourceShortage);
    };
    let list = ptr::addr_of_mut!(pset.processors);
    // SAFETY: the set lock is held, so the queue links are stable and every
    // entry is a live processor.
    let mut entry = unsafe { queue_first(list) };
    let mut i = 0;
    while i < count_slots && unsafe { queue_end(list, entry) } == 0 {
        let processor = entry.cast::<Processor>();
        // SAFETY: `entry` is a live queue member, so `processor` is a live
        // processor whose name port `ipc_processor_init()` built, and `i` is
        // below `count`, inside the allocation.
        unsafe {
            ports
                .add(i)
                .write(glue::convert_processor_name_to_port(processor));
        }
        i += 1;
        // SAFETY: `processor` is a live member of the queue.
        entry =
            unsafe { queue_next(ptr::addr_of_mut!((*processor).processors)) };
    }

    pset.lock.unlock();
    Ok((ports.cast::<VmOffset>(), count))
}

/// The body of `host_processors()` in kern/host.c: the control port of every
/// CPU the machine reports.
fn processors(
    host: Option<NonNull<Host>>,
) -> Result<(NonNull<VmOffset>, c_uint), KernError> {
    if host.is_none() {
        return Err(KernError::InvalidArgument);
    }

    let mut count: c_uint = 0;
    for i in 0..NCPUS {
        // SAFETY: `i` is below the configured `NCPUS`, the C array's length.
        if unsafe { (*machine::slot(i)).is_cpu } != 0 {
            count += 1;
        }
    }

    if count == 0 {
        // SAFETY: `Panic` does not return; the tags reproduce the C
        // `panic()` call's file, function and message.
        unsafe {
            glue::Panic(
                c"kern/host.c".as_ptr(),
                line!() as c_int,
                c"host_processors".as_ptr(),
                c"host_processors".as_ptr(),
            )
        }
    }

    // The C count holds at most `NCPUS` slots, so the widening cannot lose a
    // bit.
    let slots = count as usize;
    let size = slots * size_of::<VmOffset>();

    // SAFETY: `kalloc_init()` ran during the boot this MIG entry follows.
    let base = unsafe { glue::kalloc(size) };
    let Some(ports) =
        NonNull::new(ptr::with_exposed_provenance_mut::<VmOffset>(base))
    else {
        return Err(KernError::ResourceShortage);
    };

    let mut slot = 0;
    for i in 0..NCPUS {
        // SAFETY: `i` is below the configured `NCPUS`, the C array's length.
        if unsafe { (*machine::slot(i)).is_cpu } == 0 {
            continue;
        }

        // The C indexed `percpu_array` with an `int`; `i` counts at most
        // `NCPUS`, so the narrowing cannot wrap.
        let cpu = i as c_int;
        // SAFETY: `i` is a live CPU number, and its per-CPU block and
        // processor are live from `pset_sys_bootstrap()`.
        let processor =
            unsafe { ptr::addr_of_mut!((*percpu_at(cpu)).processor) };
        // SAFETY: `slot` is below `count`, the allocation's length; the C
        // stored the processors and converted them in a second pass, and
        // converting each as it is stored leaves the same array.
        unsafe {
            ports
                .add(slot)
                .write(glue::convert_processor_to_port(processor).addr());
        }
        slot += 1;
    }

    Ok((ports, count))
}

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
    match processors(NonNull::new(host)) {
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
