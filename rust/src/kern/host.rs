// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon
//   University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The processor-set cores of `kern/host.c`, mirroring <kern/host.h>.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::{queue_end, queue_first, queue_next};
use crate::kern::types::KernError;
use core::ffi::{c_uint, c_void};
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
