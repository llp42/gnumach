// SPDX-License-Identifier: CMU-Mach
// Derived from kern/host.c:
//   Copyright (c) 1993,1992,1991,1990,1989,1988 Carnegie Mellon
//   University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The cores of `kern/host.c`, mirroring <kern/host.h>.

use crate::arch::i386::percpu::percpu_at;
use crate::arch::types::VmOffset;
use crate::config::NCPUS;
use crate::kern::debug::kpanic;
use crate::kern::ipc_host::pset_name_to_port;
use crate::kern::mach_factor;
use crate::kern::machine;
use crate::kern::processor::{self, Processor, ProcessorSet};
use crate::kern::queue::{QueueEntry, queue_end, queue_first, queue_next};
use crate::kern::slab::{kalloc, kfree};
use crate::kern::types::KernError;
use core::ffi::{c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull};

/// `struct host` of <kern/host.h>, the host object MIG hands the host
/// routines.
#[repr(C)]
pub struct Host {
    pub host_self: *mut c_void,
    pub host_priv_self: *mut c_void,
}

/// `realhost` of kern/host.c: the one host object, and the C symbol the
/// bootstrap and server paths still name.
#[unsafe(export_name = "realhost")]
pub(crate) static mut REALHOST: Host = Host {
    host_self: ptr::null_mut(),
    host_priv_self: ptr::null_mut(),
};

/// `&realhost`, which the C passes to `ipc_kobject_set()`.
pub(crate) fn realhost() -> *mut Host {
    ptr::addr_of_mut!(REALHOST)
}

/// `HOST_BASIC_INFO` of <mach/host_info.h>.
const HOST_BASIC_INFO: c_int = 1;
/// `HOST_PROCESSOR_SLOTS` of <mach/host_info.h>.
const HOST_PROCESSOR_SLOTS: c_int = 2;
/// `HOST_SCHED_INFO` of <mach/host_info.h>.
const HOST_SCHED_INFO: c_int = 3;
/// `HOST_LOAD_INFO` of <mach/host_info.h>.
const HOST_LOAD_INFO: c_int = 4;
/// `HOST_INFO_MAX` of <mach/host_info.h>: the elements `host_info_data_t`
/// holds.
pub(crate) const HOST_INFO_MAX: usize = 1024;
/// `HOST_SCHED_INFO_COUNT` of <mach/host_info.h>.
const HOST_SCHED_INFO_COUNT: usize = 2;
/// `HOST_LOAD_INFO_COUNT` of <mach/host_info.h>.
const HOST_LOAD_INFO_COUNT: usize = 6;

/// `struct host_basic_info` of <mach/host_info.h>.
#[repr(C)]
struct HostBasicInfo {
    max_cpus: c_int,
    avail_cpus: c_int,
    /// `memory_size`: an `rpc_vm_size_t`, pointer-sized on both targets.
    memory_size: usize,
    cpu_type: c_int,
    cpu_subtype: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<HostBasicInfo>() == 24);
    assert!(align_of::<HostBasicInfo>() == 8);
    assert!(offset_of!(HostBasicInfo, max_cpus) == 0);
    assert!(offset_of!(HostBasicInfo, avail_cpus) == 4);
    assert!(offset_of!(HostBasicInfo, memory_size) == 8);
    assert!(offset_of!(HostBasicInfo, cpu_type) == 16);
    assert!(offset_of!(HostBasicInfo, cpu_subtype) == 20);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<HostBasicInfo>() == 20);
    assert!(align_of::<HostBasicInfo>() == 4);
    assert!(offset_of!(HostBasicInfo, max_cpus) == 0);
    assert!(offset_of!(HostBasicInfo, avail_cpus) == 4);
    assert!(offset_of!(HostBasicInfo, memory_size) == 8);
    assert!(offset_of!(HostBasicInfo, cpu_type) == 12);
    assert!(offset_of!(HostBasicInfo, cpu_subtype) == 16);
};

/// `HOST_BASIC_INFO_COUNT` of <mach/host_info.h>.
const HOST_BASIC_INFO_COUNT: usize =
    size_of::<HostBasicInfo>() / size_of::<c_int>();

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

    // `kalloc_init()` ran during the boot this MIG entry follows, and the
    // size is the C expression's.
    let Some(ports) = kalloc(size).map(|buf| buf.cast::<*mut c_void>()) else {
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
                .write(crate::kern::ipc_host::processor_name_to_port(
                    processor,
                ));
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
pub(crate) fn processors(
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
        kpanic!("host_processors", "host_processors")
    }

    // The C count holds at most `NCPUS` slots, so the widening cannot lose a
    // bit.
    let slots = count as usize;
    let size = slots * size_of::<VmOffset>();

    // `kalloc_init()` ran during the boot this MIG entry follows.
    let Some(ports) = kalloc(size).map(|buf| buf.cast::<VmOffset>()) else {
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
            ports.add(slot).write(
                crate::kern::ipc_host::processor_to_port(processor).addr(),
            );
        }
        slot += 1;
    }

    Ok((ports, count))
}

/// The body of `host_info()` in kern/host.c: one flavor of machine
/// information into the C caller's scratch array.
pub(crate) fn info(
    host: Option<&Host>,
    flavor: c_int,
    info: &mut [c_int],
) -> Result<c_uint, KernError> {
    if host.is_none() {
        return Err(KernError::InvalidArgument);
    }

    match flavor {
        HOST_BASIC_INFO => {
            if info.len() < HOST_BASIC_INFO_COUNT {
                return Err(KernError::Failure);
            }

            // SAFETY: `primary_processor` is the live processor
            // `pset_sys_bootstrap()` initialized, whose `slot_num` is a live
            // CPU number, below `NCPUS`.
            let (cpu_type, cpu_subtype) = unsafe {
                let processor = processor::master_processor();
                let slot = machine::slot((*processor).slot_num as usize);
                ((*slot).cpu_type, (*slot).cpu_subtype)
            };
            let basic = info.as_mut_ptr().cast::<HostBasicInfo>();
            // SAFETY: the count check above guarantees the caller's array
            // holds a whole record; the writes are unaligned because the MIG
            // buffer is only `integer_t`-aligned.
            unsafe {
                let machine_info = &*machine::info();
                ptr::addr_of_mut!((*basic).max_cpus)
                    .write_unaligned(machine_info.max_cpus);
                ptr::addr_of_mut!((*basic).avail_cpus)
                    .write_unaligned(machine_info.avail_cpus);
                ptr::addr_of_mut!((*basic).memory_size)
                    .write_unaligned(machine_info.memory_size);
                ptr::addr_of_mut!((*basic).cpu_type).write_unaligned(cpu_type);
                ptr::addr_of_mut!((*basic).cpu_subtype)
                    .write_unaligned(cpu_subtype);
            }

            Ok(HOST_BASIC_INFO_COUNT as c_uint)
        }

        HOST_PROCESSOR_SLOTS => {
            if info.len() < NCPUS {
                return Err(KernError::InvalidArgument);
            }

            let mut count = 0;
            for i in 0..NCPUS {
                // SAFETY: `i` is below the configured `NCPUS`.
                let slot = unsafe { machine::slot(i) };
                // SAFETY: the slot is live.
                if unsafe { (*slot).is_cpu } != 0
                    && unsafe { (*slot).running } != 0
                {
                    info[count] = i as c_int;
                    count += 1;
                }
            }

            Ok(count as c_uint)
        }

        HOST_SCHED_INFO => {
            if info.len() < HOST_SCHED_INFO_COUNT {
                return Err(KernError::Failure);
            }

            let tick = crate::kern::mach_clock::tick;
            let min_quantum = crate::kern::sched_prim::min_quantum();
            info[0] = tick / 1000;
            // The C overflowed an `int` the same way; the clock and the
            // quantum are both small at run time.
            info[1] = min_quantum.wrapping_mul(tick) / 1000;

            Ok(HOST_SCHED_INFO_COUNT as c_uint)
        }

        HOST_LOAD_INFO => {
            if info.len() < HOST_LOAD_INFO_COUNT {
                return Err(KernError::Failure);
            }

            let avenrun = mach_factor::avenrun();
            let factor = mach_factor::mach_factor();
            for (i, (average, factor)) in
                avenrun.iter().zip(factor.iter()).enumerate()
            {
                let Some(average_slot) = info.get_mut(i) else {
                    return Err(KernError::Failure);
                };
                // The C assigned a `long` to an `integer_t`, a deliberate
                // truncation.
                *average_slot = *average as c_int;
                let Some(factor_slot) = info.get_mut(3 + i) else {
                    return Err(KernError::Failure);
                };
                *factor_slot = *factor as c_int;
            }

            Ok(HOST_LOAD_INFO_COUNT as c_uint)
        }

        _ => Err(KernError::InvalidArgument),
    }
}

/// The body of `host_processor_sets()` in kern/host.c: the name port of
/// every set on the host, in the array MIG sends back.
pub(crate) unsafe fn processor_sets(
    host: Option<&Host>,
) -> Result<(*mut *mut c_void, c_uint), KernError> {
    if host.is_none() {
        return Err(KernError::InvalidArgument);
    }

    let lock = processor::all_psets_lock();
    let mut size: usize = 0;
    let mut addr: *mut u8 = ptr::null_mut();
    let actual;
    let size_needed;

    loop {
        // SAFETY: the lock guards `all_psets`.
        unsafe { (*lock).lock() };
        // SAFETY: the lock is held.
        let count = unsafe { *processor::all_psets_count() };
        // The count is the number of live sets, never negative.
        let count = count as usize;
        let needed = count.wrapping_mul(size_of::<usize>());
        if needed <= size {
            actual = count;
            size_needed = needed;
            break;
        }

        // SAFETY: the lock taken above.
        unsafe { (*lock).unlock() };
        if let Some(old) = NonNull::new(addr) {
            // SAFETY: `addr` came from `kalloc(size)`.
            unsafe { kfree(old, size) };
        }
        size = needed;
        let Some(buffer) = kalloc(size) else {
            return Err(KernError::ResourceShortage);
        };
        addr = buffer.as_ptr();
    }

    let psets = addr.cast::<*mut c_void>();
    let list = processor::all_psets();
    // SAFETY: the lock is held, the queue was initialized by
    // `processor_set_create()`, and every link is a live set.
    let mut entry = unsafe { queue_first(list) };
    for i in 0..actual {
        let pset = entry.cast::<ProcessorSet>();
        // SAFETY: `entry` is a live set in the locked queue.
        unsafe { (*pset).reference() };
        // SAFETY: `i` is below `actual`, the allocation's slot count.
        unsafe { psets.add(i).write(pset.cast()) };
        // SAFETY: `pset` is a live member of the queue.
        entry = unsafe { queue_next(ptr::addr_of_mut!((*pset).all_psets)) };
    }
    // SAFETY: the lock taken above.
    unsafe { (*lock).unlock() };

    let mut psets = psets;
    if size_needed < size {
        let Some(buffer) = kalloc(size_needed) else {
            for i in 0..actual {
                // SAFETY: every slot below `actual` holds a referenced set.
                let pset = unsafe { psets.add(i).read() };
                // SAFETY: the reference came from the loop above.
                unsafe { (*pset.cast::<ProcessorSet>()).deallocate() };
            }
            // SAFETY: `addr` came from `kalloc(size)`.
            unsafe { kfree(NonNull::new_unchecked(addr), size) };
            return Err(KernError::ResourceShortage);
        };
        let newaddr = buffer.as_ptr();

        // SAFETY: `size_needed <= size`, and both regions are live
        // allocations.
        unsafe {
            ptr::copy_nonoverlapping(addr, newaddr, size_needed);
            kfree(NonNull::new_unchecked(addr), size);
        }
        psets = newaddr.cast::<*mut c_void>();
    }

    for i in 0..actual {
        // SAFETY: every slot below `actual` holds a referenced set, which
        // `pset_name_to_port()` consumes.
        let pset = unsafe { psets.add(i).read() };
        // SAFETY: the set is referenced and live.
        let port = unsafe { pset_name_to_port(pset.cast::<ProcessorSet>()) };
        // SAFETY: the slot is writable.
        unsafe { psets.add(i).write(port) };
    }

    Ok((psets, actual as c_uint))
}

/// The `HOST_PROCESSOR_SLOTS` array holds `NCPUS` entries at most, and the
/// `HostBasicInfo` record is the widest one `host_info()` writes.
const _: () = assert!(NCPUS <= HOST_INFO_MAX);

/// The C `struct host` is two pointers, in the order `Host` mirrors.
const _: () = {
    const PTR: usize = size_of::<*mut c_void>();
    assert!(size_of::<Host>() == 2 * PTR);
    assert!(align_of::<Host>() == align_of::<*mut c_void>());
    assert!(offset_of!(Host, host_self) == 0);
    assert!(offset_of!(Host, host_priv_self) == PTR);
};

/// `QueueEntry` is the C `struct queue_entry` the `all_psets` head is.
const _: () =
    assert!(size_of::<QueueEntry>() == 2 * size_of::<*mut QueueEntry>());
