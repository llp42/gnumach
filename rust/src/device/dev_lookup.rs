// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_lookup.c and device/dev_hdr.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device lookup table, which `device/dev_lookup.c` used to define and
//! <device/dev_hdr.h> declares.
//!
//! The C table's number lock was a file `static`; the Rust one is a
//! [`Mutex`], still held before any device's `ref_lock`.

use crate::device::dev_name;
use crate::device::ds_routines::{
    DevOps, Device, MACH_DEVICE_EMULATION_OPS, MachDevice,
};
use crate::glue;
use crate::ipc::IpcPort;
use crate::kern::lock::SimpleLock;
use crate::kern::queue::{
    QueueEntry, queue_end, queue_enter_tail, queue_first, queue_init,
    queue_next, queue_remove_generic,
};
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::spin::Mutex;
use core::ffi::{c_char, c_int, c_short, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull};

/// `NDEVHASH` of `device/dev_lookup.c`: the device-number buckets.
const NDEVHASH: usize = 8;

/// `DEV_BSIZE` of <device/param.h>.
const DEV_BSIZE: c_int = 512;

/// `DEV_STATE_INIT` of <device/dev_hdr.h>.
const DEV_STATE_INIT: c_short = 0;

/// `IKOT_DEVICE` of <kern/ipc_kobject.h>.
const IKOT_DEVICE: c_uint = 10;

/// `IKOT_NONE` of <kern/ipc_kobject.h>.
const IKOT_NONE: c_uint = 0;

/// `dev_number_hash_table` of `device/dev_lookup.c`: one bucket per
/// `DEV_NUMBER_HASH()` result.
static mut DEV_NUMBER_HASH_TABLE: [QueueEntry; NDEVHASH] =
    [const { QueueEntry::unlinked() }; NDEVHASH];

/// `dev_number_lock`: serializes the table, and is held before any device's
/// `ref_lock`, as <device/dev_hdr.h> requires.
static DEV_NUMBER_LOCK: Mutex<()> = Mutex::new(());

/// `dev_hdr_cache`: the `struct mach_device` slab cache.
static mut DEV_HDR_CACHE: KmemCache = KmemCache::zeroed();

/// `DEV_NUMBER_HASH()` of `device/dev_lookup.c`.
const fn number_hash(dev_number: c_int) -> usize {
    // The mask leaves a value below `NDEVHASH`, so the cast cannot lose
    // anything that matters.
    (dev_number & (NDEVHASH as c_int - 1)) as usize
}

/// The bucket at `index` of the device-number table.
///
/// # Safety
///
/// `index` must be below `NDEVHASH`.
unsafe fn number_bucket_at(index: usize) -> *mut QueueEntry {
    // SAFETY: the caller promises the index is below the array bound.
    unsafe {
        ptr::addr_of_mut!(DEV_NUMBER_HASH_TABLE)
            .cast::<QueueEntry>()
            .add(index)
    }
}

/// The bucket `dev_number` hashes to.
fn number_bucket(dev_number: c_int) -> *mut QueueEntry {
    // SAFETY: `number_hash()` is below the array bound.
    unsafe { number_bucket_at(number_hash(dev_number)) }
}

/// `kmem_cache_alloc(&dev_hdr_cache)` and the field writes the C ran after
/// it.
///
/// # Safety
///
/// The cache must be initialized, and the caller must not hold
/// [`DEV_NUMBER_LOCK`].
unsafe fn alloc_device(
    dev_ops: *mut DevOps,
    dev_number: c_int,
) -> Option<NonNull<MachDevice>> {
    // SAFETY: the cache is live after `init()`.
    let buf = unsafe { (*ptr::addr_of_mut!(DEV_HDR_CACHE)).alloc() }?;
    let device = buf.as_ptr().cast::<MachDevice>();
    // SAFETY: the cache object is a fresh, unshared `struct mach_device`,
    // and the C wrote every field the mirror carries.
    unsafe {
        ptr::write(
            device,
            MachDevice {
                ref_lock: SimpleLock::new(),
                ref_count: 1,
                lock: SimpleLock::new(),
                state: DEV_STATE_INIT,
                flag: 0,
                open_count: 0,
                io_in_progress: 0,
                io_wait: 0,
                port: ptr::null_mut(),
                number_chain: QueueEntry::unlinked(),
                dev_number,
                bsize: DEV_BSIZE,
                dev_ops,
                dev: Device {
                    emul_ops: ptr::null_mut(),
                    emul_data: ptr::null_mut(),
                },
            },
        );
    }
    NonNull::new(device)
}

/// `kmem_cache_free(&dev_hdr_cache, device)` of the C.
///
/// # Safety
///
/// `device` must be a dead device from [`alloc_device()`] with no holder
/// left.
unsafe fn free_device(device: *mut MachDevice) {
    // SAFETY: the caller promises the dead, owned device.
    unsafe {
        (*ptr::addr_of_mut!(DEV_HDR_CACHE))
            .free(NonNull::new_unchecked(device.cast::<u8>()));
    }
}

/// `dev_number_enter()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// [`DEV_NUMBER_LOCK`] must be held, and `device` must be live and not
/// linked into the table.
unsafe fn number_enter(device: *mut MachDevice) {
    // SAFETY: the caller promises the live, unlinked device.
    unsafe {
        queue_enter_tail(
            number_bucket((*device).dev_number),
            device.cast::<c_void>(),
            offset_of!(MachDevice, number_chain),
        );
    }
}

/// `dev_number_remove()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// [`DEV_NUMBER_LOCK`] must be held, and `device` must be linked into the
/// table.
unsafe fn number_remove(device: *mut MachDevice) {
    // SAFETY: the caller promises the linked device.
    unsafe {
        queue_remove_generic(
            number_bucket((*device).dev_number),
            device.cast::<c_void>(),
            offset_of!(MachDevice, number_chain),
        );
    }
}

/// `dev_number_lookup()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// [`DEV_NUMBER_LOCK`] must be held, and every linked device live.
unsafe fn number_lookup(
    dev_ops: *mut DevOps,
    dev_number: c_int,
) -> *mut MachDevice {
    let head = number_bucket(dev_number);
    // SAFETY: the caller promises the initialized table, and the queue
    // invariant keeps `device` on a live device.
    unsafe {
        let mut device = queue_first(head).cast::<MachDevice>();
        while queue_end(head, device.cast()) == 0 {
            if (*device).dev_ops == dev_ops
                && (*device).dev_number == dev_number
            {
                return device;
            }
            device = queue_next(ptr::addr_of_mut!((*device).number_chain))
                .cast::<MachDevice>();
        }
        ptr::null_mut()
    }
}

/// `device_lookup()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `name` must be a NUL-terminated string readable by the caller, and the
/// device package must be initialized. The returned device carries one
/// reference.
pub(crate) unsafe fn lookup(
    name: *const c_char,
) -> Option<NonNull<MachDevice>> {
    // SAFETY: the caller promises the NUL-terminated name, and the device
    // name tables are initialized.
    let (dev_ops, dev_number) = unsafe { dev_name::lookup(name) }?;
    let dev_ops = dev_ops.as_ptr();

    let mut new_device: *mut MachDevice = ptr::null_mut();
    loop {
        let guard = DEV_NUMBER_LOCK.lock();
        // SAFETY: the lock is held and the table initialized.
        let found = unsafe { number_lookup(dev_ops, dev_number) };
        if !found.is_null() {
            // SAFETY: the found device is live under the lock.
            unsafe {
                reference(found);
                drop(guard);
                if !new_device.is_null() {
                    free_device(new_device);
                }
            }
            return NonNull::new(found);
        }
        if !new_device.is_null() {
            // SAFETY: the fresh device is unshared and the lock is held.
            unsafe {
                number_enter(new_device);
                drop(guard);
            }
            return NonNull::new(new_device);
        }
        drop(guard);

        // SAFETY: the cache is initialized, and the C allocated without the
        // table lock too.
        let device = (unsafe { alloc_device(dev_ops, dev_number) })?;
        new_device = device.as_ptr();
    }
}

/// `mach_device_reference()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live mach device.
pub(crate) unsafe fn reference(device: *mut MachDevice) {
    // SAFETY: the caller promises the live device.
    unsafe {
        (*device).ref_lock.lock();
        (*device).ref_count += 1;
        (*device).ref_lock.unlock();
    }
}

/// `mach_device_deallocate()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live mach device the caller holds a reference on, and
/// nothing may touch it once its last reference goes.
pub(crate) unsafe fn deallocate(device: *mut MachDevice) {
    // SAFETY: the caller promises the live device, and the lock order is
    // the C's: the number lock before the device's `ref_lock`.
    unsafe {
        (*device).ref_lock.lock();
        (*device).ref_count -= 1;
        if (*device).ref_count > 0 {
            (*device).ref_lock.unlock();
            return;
        }
        (*device).ref_count = 1;
        (*device).ref_lock.unlock();

        let guard = DEV_NUMBER_LOCK.lock();
        (*device).ref_lock.lock();
        (*device).ref_count -= 1;
        if (*device).ref_count > 0 {
            (*device).ref_lock.unlock();
            drop(guard);
            return;
        }
        number_remove(device);
        (*device).ref_lock.unlock();
        drop(guard);
    }

    // SAFETY: the last reference is gone, and the caller must not touch the
    // device again.
    unsafe { free_device(device) };
}

/// `dev_port_enter()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live device whose port is a live port, and the caller
/// must own a device reference for the mapping to take.
pub(crate) unsafe fn port_enter(device: *mut MachDevice) {
    // SAFETY: the caller promises the live device and port; the kobject is
    // bound before the emulation the lookup dispatches through.
    unsafe {
        reference(device);
        glue::ipc_kobject_set(
            (*device).port,
            ptr::addr_of_mut!((*device).dev).addr(),
            IKOT_DEVICE,
        );
        (*device).dev.emul_data = device.cast::<c_void>();
        (*device).dev.emul_ops = ptr::addr_of_mut!(MACH_DEVICE_EMULATION_OPS);
    }
}

/// `dev_port_remove()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be a live device whose port carries the mapping, and the
/// caller's reference moves into the call.
pub(crate) unsafe fn port_remove(device: *mut MachDevice) {
    // SAFETY: the caller promises the live device and its mapping.
    unsafe {
        glue::ipc_kobject_set((*device).port, 0, IKOT_NONE);
        deallocate(device);
    }
}

/// `dev_port_lookup()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `port` must be null, dead, or a live port.
pub(crate) unsafe fn port_lookup(port: *mut c_void) -> *mut Device {
    let Some(port) = IpcPort::valid(port) else {
        return ptr::null_mut();
    };

    // SAFETY: the live port's lock serializes the kobject read, and a
    // `IKOT_DEVICE` kobject is the embedded `struct device` of a live
    // `mach_device`.
    unsafe {
        port.lock();
        let device = if port.is_active() && port.kotype() == IKOT_DEVICE {
            let device = port.kobject().cast::<Device>();
            let ops = (*device).emul_ops;
            if !ops.is_null()
                && let Some(reference) = (*ops).reference
            {
                reference((*device).emul_data);
            }
            device
        } else {
            ptr::null_mut()
        };
        port.unlock();
        device
    }
}

/// `convert_device_to_port()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `device` must be null or a live `struct device`, and its emulation must
/// be one whose `dev_to_port` takes the reference the caller consumed.
pub(crate) unsafe fn convert_to_port(device: *mut Device) -> *mut c_void {
    if device.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: the caller promises the live device and its emulation.
    unsafe {
        let ops = (*device).emul_ops;
        if ops.is_null() {
            return ptr::null_mut();
        }
        match (*ops).dev_to_port {
            Some(dev_to_port) => dev_to_port((*device).emul_data),
            None => ptr::null_mut(),
        }
    }
}

/// `dev_map()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// `routine` must be a C callback that takes the `mach_device_t` and
/// `mach_port_t` it is passed, and the device package must be initialized.
pub(crate) unsafe fn map(
    routine: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    port: *mut c_void,
) -> c_int {
    let Some(routine) = routine else {
        return 0;
    };

    for index in 0..NDEVHASH {
        // SAFETY: `index` is below the array bound.
        let head = unsafe { number_bucket_at(index) };
        let mut prev: *mut MachDevice = ptr::null_mut();
        let mut guard = DEV_NUMBER_LOCK.lock();
        // SAFETY: the lock is held and the bucket initialized.
        let mut device = unsafe { queue_first(head) }.cast::<MachDevice>();
        // SAFETY: the C released the table lock around each callback and
        // re-took it on the same reference, which the walk keeps.
        unsafe {
            while queue_end(head, device.cast()) == 0 {
                reference(device);
                drop(guard);
                if !prev.is_null() {
                    deallocate(prev);
                }

                if routine(device.cast::<c_void>(), port) != 0 {
                    deallocate(device);
                    return c_int::from(true);
                }

                guard = DEV_NUMBER_LOCK.lock();
                prev = device;
                device = queue_next(ptr::addr_of_mut!((*device).number_chain))
                    .cast::<MachDevice>();
            }
            drop(guard);
            if !prev.is_null() {
                deallocate(prev);
            }
        }
    }
    c_int::from(false)
}

/// `dev_lookup_init()` of `device/dev_lookup.c`.
///
/// # Safety
///
/// Runs once from the boot sequence, before any device exists.
pub(crate) unsafe fn init() {
    // SAFETY: this call builds the cache and the queue heads before any
    // device is looked up.
    unsafe {
        (*ptr::addr_of_mut!(DEV_HDR_CACHE)).init(
            b"mach_device",
            size_of::<MachDevice>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
        for index in 0..NDEVHASH {
            queue_init(number_bucket_at(index));
        }
    }
}
