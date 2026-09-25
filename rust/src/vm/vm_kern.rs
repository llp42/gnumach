// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_kern.c and vm/vm_kern.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Kernel memory management, which `vm/vm_kern.c` used to define and
//! `vm/vm_kern.h` declares.

use crate::arch::i386::pmap::pmap_pageable;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue::{
    copyin, copyout, kernel_object, kernel_pmap, pmap_enter, pmap_extract,
    pmap_map_bd, pmap_reference, pmap_remove, vm_page_queue_lock,
};
use crate::kern::console::{CStrArg, kprint};
use crate::kern::debug::kpanic;
use crate::kern::slab::slab_collect;
use crate::kern::task::current_task;
use crate::vm::error::{Error, KERN_SUCCESS, error_from_kern_return};
use crate::vm::types::{Pmap, VmInherit, VmObject, VmProt};
use crate::vm::vm_map::{
    EnterRequest, Projection, VmMap, VmMapCopy, VmMapCopyinArgs, round_page,
    trunc_page,
};
use crate::vm::vm_object::vm_submap_object;
use crate::vm::vm_object::{self, allocate, deallocate, reference};
use crate::vm::{vm_page, vm_resident};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull, addr_of_mut, with_exposed_provenance_mut};
use core::sync::atomic::{AtomicBool, Ordering};

/// `VM_PAGE_HIGHMEM` of <vm/vm_page.h>: the page may come from high physical
/// memory.
const VM_PAGE_HIGHMEM: c_uint = 0x08;

/// `VM_MIN_KERNEL_ADDRESS` of <machine/vm_param.h>: `KERNEL_MAP_BASE` on
/// x86_64 and `0xC0000000` on i686.  A `kernel_object` offset is linear in
/// the kernel virtual address, so a kernel mapping is stored at
/// `addr - VM_MIN_KERNEL_ADDRESS`, and `phystokv()` adds it back.
#[cfg(target_arch = "x86_64")]
pub(crate) const VM_MIN_KERNEL_ADDRESS: VmOffset = 0xffff_ffff_8000_0000;
#[cfg(target_arch = "x86")]
pub(crate) const VM_MIN_KERNEL_ADDRESS: VmOffset = 0xC000_0000;

/// `kernel_map_store`, file-private in the C: the boot kernel map's storage.
static mut KERNEL_MAP_STORE: VmMap = VmMap::zeroed();

/// `kernel_map` of <vm/vm_kern.h>: the kernel map `kmem_init()` builds.
#[unsafe(no_mangle)]
pub static mut kernel_map: *mut VmMap = &raw mut KERNEL_MAP_STORE;

/// `kernel_pageable_map` of <vm/vm_kern.h>: never set by either build.
#[unsafe(no_mangle)]
pub static mut kernel_pageable_map: *mut VmMap = ptr::null_mut();

/// `projected_buffer_collect()` in C: unmap every projected buffer of `map`.
pub(crate) fn projected_buffer_collect(
    map: NonNull<VmMap>,
) -> Result<(), Error> {
    // SAFETY: `kernel_map` is the boot kernel map storage.
    if map.as_ptr() == unsafe { kernel_map } {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a live map, and the sentinel is the
    // header's links as `vm_map_to_entry()` computes it.
    let sentinel = unsafe { (*map.as_ptr()).to_entry() };
    // SAFETY: the header's `next` is the sentinel or a live entry.
    let mut entry =
        unsafe { (*map.as_ptr()).hdr.links.next.unwrap_or(sentinel) };

    while entry != sentinel {
        // SAFETY: `entry` is the sentinel or a live entry of the map; the
        // caller promises the chain stays stable except for the deallocation
        // below.
        let (next, start, end, projected) = unsafe {
            let entry = &*entry.as_ptr();
            (
                entry.links.next.unwrap_or(sentinel),
                entry.links.start,
                entry.links.end,
                !entry.projected_on.is_null(),
            )
        };
        if projected {
            // `start` and `end` were read from the live entry before the
            // call, which may delete it.
            let _ = projected_buffer_deallocate(map, start, end);
        }
        entry = next;
    }

    Ok(())
}

/// `projected_buffer_in_range()` in C: whether a projected buffer overlaps
/// `start..end`.
pub(crate) fn projected_buffer_in_range(
    map: &VmMap,
    start: VmOffset,
    end: VmOffset,
) -> bool {
    // SAFETY: `kernel_map` is the boot kernel map storage.
    if ptr::from_ref(map).cast_mut() == unsafe { kernel_map } {
        return false;
    }

    let sentinel = map.to_entry();
    let (found, entry) = map.lookup_entry(start);
    let mut entry = if found {
        entry
    } else {
        // SAFETY: `lookup_entry` returned the sentinel or a live entry, so
        // its `next` is the sentinel or a live entry.
        unsafe { (*entry.as_ptr()).links.next.unwrap_or(sentinel) }
    };

    while entry != sentinel
        && unsafe { (*entry.as_ptr()).projected_on.is_null() }
        && unsafe { (*entry.as_ptr()).links.start } <= end
    {
        // SAFETY: as above.
        entry = unsafe { (*entry.as_ptr()).links.next.unwrap_or(sentinel) };
    }

    entry != sentinel && unsafe { (*entry.as_ptr()).links.start } <= end
}

/// `kmem_alloc_wired_flags()` in C: reserve kernel virtual space and wire
/// memory for it.
pub(crate) fn kmem_alloc_wired_flags(
    map: NonNull<VmMap>,
    size: VmSize,
    flags: c_uint,
) -> Result<VmOffset, Error> {
    let addr = kmem_valloc(map, size)?;

    let offset = addr.wrapping_sub(VM_MIN_KERNEL_ADDRESS);
    // SAFETY: `kernel_object` is the boot object every kernel mapping maps,
    // and `addr..addr + size` is the region `kmem_valloc` just reserved.
    unsafe {
        alloc_pages(
            kernel_object,
            offset,
            addr,
            addr.wrapping_add(size),
            VmProt::READ | VmProt::WRITE,
            flags,
        )
    };

    Ok(addr)
}

/// `kmem_alloc_wired()` in C: `kmem_alloc_wired_flags()` with
/// `VM_PAGE_HIGHMEM`.
pub(crate) fn kmem_alloc_wired(
    map: NonNull<VmMap>,
    size: VmSize,
) -> Result<VmOffset, Error> {
    kmem_alloc_wired_flags(map, size, VM_PAGE_HIGHMEM)
}

/// `kmem_map_aligned_table()` in C: map a physical table at a kernel address
/// with the physical address's in-page offset.
pub(crate) fn kmem_map_aligned_table(
    map: NonNull<VmMap>,
    phys_address: VmOffset,
    size: VmSize,
    mode: c_int,
) -> Option<NonNull<c_void>> {
    let into_page = phys_address % PAGE_SIZE;
    let nearest_page = phys_address.wrapping_sub(into_page);
    let size = round_page(size.wrapping_add(into_page));

    let virt_addr = kmem_alloc_wired(map, size).ok()?;

    // SAFETY: `virt_addr` is the wired region just allocated, and
    // `nearest_page..nearest_page + size` is the physical range it stands for.
    unsafe {
        pmap_map_bd(
            virt_addr,
            nearest_page,
            nearest_page.wrapping_add(size),
            VmProt::from_bits(mode),
        )
    };

    // SAFETY: the C casts the integer address back to a pointer, and
    // `virt_addr + into_page` is inside the mapping just made.
    Some(unsafe {
        NonNull::new_unchecked(with_exposed_provenance_mut::<c_void>(
            virt_addr.wrapping_add(into_page),
        ))
    })
}

/// `kmem_alloc_pageable()` in C: reserve pageable space in the kernel map.
pub(crate) fn kmem_alloc_pageable(
    map: &mut VmMap,
    size: VmSize,
) -> Result<VmOffset, Error> {
    let mut addr = map.hdr.links.start;

    let entered = map.enter(EnterRequest {
        address: &mut addr,
        size: round_page(size),
        mask: 0,
        anywhere: true,
        object: ptr::null_mut(),
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    });

    match entered {
        Ok(()) => Ok(addr),
        Err(error) => {
            // The C `printf_once` guards a diagnostic; a relaxed flag is
            // enough because the worst case is printing it twice.
            static PRINTED: AtomicBool = AtomicBool::new(false);
            if !PRINTED.swap(true, Ordering::Relaxed) {
                // SAFETY: `map.name` is the map's NUL-terminated name.
                let name = unsafe { CStrArg::from_ptr(map.name) };
                kprint!(
                    "no more room for kmem_alloc_pageable in {:x} ({})\n",
                    ptr::from_ref(map).expose_provenance(),
                    name,
                );
            }
            Err(error)
        }
    }
}

/// `kmem_free()` in C: release a region a `kmem_alloc*` call made.
pub(crate) fn kmem_free(
    map: &mut VmMap,
    addr: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    map.remove(trunc_page(addr), round_page(addr.wrapping_add(size)))
}

/// `kmem_submap()` in C: build `map` as a submap of `parent`.
pub(crate) fn kmem_submap(
    map: &mut VmMap,
    parent: NonNull<VmMap>,
    size: VmSize,
) -> Result<(VmOffset, VmOffset), Error> {
    let size = round_page(size);

    // SAFETY: `vm_submap_object` is the boot placeholder and is live for the
    // life of the kernel.
    let object = unsafe { vm_submap_object };
    // SAFETY: the parent's new entry holds the reference taken here, as the C
    // does before `vm_map_enter`.
    unsafe { reference(object) };

    // SAFETY: the caller promises a valid, unlocked parent map.
    let parent = unsafe { &mut *parent.as_ptr() };
    let mut addr = parent.hdr.links.start;
    parent.enter(EnterRequest {
        address: &mut addr,
        size,
        mask: 0,
        anywhere: true,
        object,
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    })?;

    let pmap = parent.pmap;
    // SAFETY: the parent owns a reference to `pmap`, and the submap must hold
    // its own.
    unsafe { pmap_reference(pmap) };
    VmMap::setup(map, pmap, addr, addr.wrapping_add(size));

    // The caller promises the parent is a live map and `map` the storage just
    // set up, which is what `vm_map_submap()` requires.
    parent.submap(addr, addr.wrapping_add(size), ptr::from_mut(map))?;

    Ok((addr, addr.wrapping_add(size)))
}

/// `kmem_init()` in C: initialize the kernel map's address range.
pub(crate) fn kmem_init(
    map: NonNull<VmMap>,
    pmap: *mut Pmap,
    start: VmOffset,
    end: VmOffset,
) -> Result<(), Error> {
    // SAFETY: the caller passes the kernel's own map storage before anything
    // else uses it, and `pmap` is the boot pmap.
    unsafe {
        VmMap::setup(&mut *map.as_ptr(), pmap, VM_MIN_KERNEL_ADDRESS, end)
    };

    if start == VM_MIN_KERNEL_ADDRESS {
        return Ok(());
    }

    let mut addr = VM_MIN_KERNEL_ADDRESS;
    // SAFETY: the map was just set up and is unlocked.
    unsafe {
        (*map.as_ptr()).enter(EnterRequest {
            address: &mut addr,
            size: start.wrapping_sub(VM_MIN_KERNEL_ADDRESS),
            mask: 0,
            anywhere: true,
            object: ptr::null_mut(),
            offset: 0,
            needs_copy: false,
            cur_protection: VmProt::READ | VmProt::WRITE,
            max_protection: VmProt::ALL,
            inheritance: VmInherit::COPY,
        })
    }
}

/// `kmem_io_map_deallocate()` in C: drop the mapping `kmem_io_map_copyout()`
/// established.
pub(crate) fn kmem_io_map_deallocate(
    map: &mut VmMap,
    addr: VmOffset,
    size: VmSize,
) {
    let end = addr.wrapping_add(size);
    // SAFETY: the caller promises the range belongs to this map, whose pmap
    // is live.
    unsafe { pmap_remove(map.pmap, addr, end) };
    let _ = map.remove(addr, end);
}

/// The C `panic()` for an allocation that cannot fail.
fn die(func: &'static str) -> ! {
    kpanic!(func, "{}", func)
}

/// `kernel_map` as the non-null map the boot path built.
fn kernel_map_non_null() -> NonNull<VmMap> {
    // SAFETY: the boot map storage is a live `VmMap` from the first call
    // onwards; `kmem_init()` is what fills its range.
    unsafe { NonNull::new_unchecked(ptr::addr_of_mut!(KERNEL_MAP_STORE)) }
}

/// `projected_buffer_allocate()` in C: allocate a fresh, zero-filled buffer
/// shared between the kernel map and `map`.
pub(crate) fn projected_buffer_allocate(
    map: NonNull<VmMap>,
    size: VmSize,
    persistence: bool,
    protection: VmProt,
    inheritance: VmInherit,
) -> Result<(VmOffset, VmOffset), Error> {
    let kernel = kernel_map_non_null();
    if map == kernel {
        return Err(Error::InvalidArgument);
    }

    let size = round_page(size);
    // SAFETY: the object allocator halts the kernel rather than fail, so
    // this reference always exists.
    let object = unsafe { allocate(size) }
        .unwrap_or_else(|| die("vm_object_allocate"))
        .as_ptr();

    // SAFETY: `kernel` is the live boot map, and the write lock serializes
    // the entry search.
    VmMap::lock(kernel);
    let found = unsafe {
        VmMap::find_entry(
            &mut *kernel.as_ptr(),
            size,
            0,
            ptr::null_mut(),
            VmProt::READ | VmProt::WRITE,
            VmProt::ALL,
        )
    };
    let (kaddr, k_entry) = match found {
        Ok(found) => found,
        Err(error) => {
            VmMap::unlock(kernel);
            // SAFETY: the allocator's reference is the only one.
            unsafe { deallocate(object) };
            return Err(error);
        }
    };
    // SAFETY: `k_entry` is the new entry of the locked boot map; the object
    // reference it takes is the allocator's.
    unsafe {
        (*k_entry.as_ptr()).object.vm_object = object;
        if !persistence {
            (*k_entry.as_ptr()).set_projection_non_persistent();
        }
    }
    VmMap::unlock(kernel);

    // SAFETY: the caller promises a valid map; the write lock serializes the
    // entry search.
    VmMap::lock(map);
    let found = unsafe {
        VmMap::find_entry(
            &mut *map.as_ptr(),
            size,
            0,
            ptr::null_mut(),
            protection,
            protection,
        )
    };
    let (uaddr, u_entry) = match found {
        Ok(found) => found,
        Err(error) => {
            VmMap::unlock(map);
            VmMap::lock(kernel);
            // SAFETY: `k_entry` is the kernel entry of this buffer, and the
            // boot map is locked; deleting it drops the object reference.
            unsafe { (*kernel.as_ptr()).entry_delete(k_entry) };
            VmMap::unlock(kernel);
            // SAFETY: as in the C, which deallocated the object a second
            // time after the entry did.
            unsafe { deallocate(object) };
            return Err(error);
        }
    };
    // SAFETY: `u_entry` is the new entry of the locked user map, and it
    // takes the second reference to `object`.
    unsafe {
        (*u_entry.as_ptr()).object.vm_object = object;
        reference(object);
        (*u_entry.as_ptr()).projected_on = k_entry.as_ptr();
        (*u_entry.as_ptr()).inheritance = inheritance;
    }
    VmMap::unlock(map);

    // SAFETY: `object` owns the wired range `kaddr..kaddr + size`.
    unsafe {
        alloc_pages(
            object,
            0,
            kaddr,
            kaddr.wrapping_add(size),
            VmProt::READ | VmProt::WRITE,
            VM_PAGE_HIGHMEM,
        )
    };
    // SAFETY: the wired range is `size` bytes of kernel memory.
    unsafe {
        ptr::write_bytes(with_exposed_provenance_mut::<u8>(kaddr), 0, size)
    };

    let pmap = unsafe { (*map.as_ptr()).pmap };
    pmap_pageable(pmap, uaddr, uaddr.wrapping_add(size), c_int::from(false));
    let mut r_size = 0;
    while r_size < size {
        // SAFETY: `kernel_pmap` is the boot pmap and the kernel address is
        // mapped by the range just allocated.
        let physical_addr =
            unsafe { pmap_extract(kernel_pmap, kaddr.wrapping_add(r_size)) };
        // SAFETY: `pmap` is the live pmap of `map`, and the user address was
        // just entered in it.
        unsafe {
            pmap_enter(
                pmap,
                uaddr.wrapping_add(r_size),
                physical_addr,
                protection,
                c_int::from(true),
            )
        };
        r_size = r_size.wrapping_add(PAGE_SIZE);
    }

    Ok((kaddr, uaddr))
}

/// `projected_buffer_map()` in C: map an existing kernel buffer into `map`.
pub(crate) fn projected_buffer_map(
    map: NonNull<VmMap>,
    kernel_addr: VmOffset,
    size: VmSize,
    protection: VmProt,
    inheritance: VmInherit,
) -> Result<VmOffset, Error> {
    let kernel = kernel_map_non_null();
    if map == kernel {
        return Err(Error::InvalidArgument);
    }

    let size = round_page(size);
    // SAFETY: `kernel` is the live boot map, and the caller's read of its
    // entry chain is serialized by its own lock only in the C; the lookup
    // uses the hint lock as the ported map does.
    let (found, k_entry) =
        unsafe { (*kernel.as_ptr()).lookup_entry(kernel_addr) };
    // SAFETY: `k_entry` is the sentinel or a live entry.
    let k_end = unsafe { (*k_entry.as_ptr()).links.end };
    if !found || kernel_addr.wrapping_add(size) > k_end {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a valid map; the write lock serializes the
    // entry search.
    VmMap::lock(map);
    let found = unsafe {
        VmMap::find_entry(
            &mut *map.as_ptr(),
            size,
            0,
            ptr::null_mut(),
            protection,
            protection,
        )
    };
    let (uaddr, u_entry) = match found {
        Ok(found) => found,
        Err(error) => {
            VmMap::unlock(map);
            return Err(error);
        }
    };
    // SAFETY: `k_entry` and `u_entry` are live entries of their locked maps,
    // and the user entry takes a reference on the shared object.
    unsafe {
        let object = (*k_entry.as_ptr()).object.vm_object;
        (*u_entry.as_ptr()).object.vm_object = object;
        reference(object);
        (*u_entry.as_ptr()).offset = kernel_addr
            .wrapping_sub((*k_entry.as_ptr()).links.start)
            .wrapping_add((*k_entry.as_ptr()).offset);
        (*u_entry.as_ptr()).projected_on = k_entry.as_ptr();
        (*u_entry.as_ptr()).inheritance = inheritance;
        (*u_entry.as_ptr()).wired_count = (*k_entry.as_ptr()).wired_count;
    }
    VmMap::unlock(map);

    let pmap = unsafe { (*map.as_ptr()).pmap };
    // SAFETY: `k_entry` is a live entry of the boot map and it is wired.
    let wired = unsafe { (*k_entry.as_ptr()).wired_count };
    pmap_pageable(
        pmap,
        uaddr,
        uaddr.wrapping_add(size),
        c_int::from(wired == 0),
    );
    let mut r_size = 0;
    while r_size < size {
        // SAFETY: `kernel_pmap` is the boot pmap and the kernel address is
        // mapped by the buffer's range.
        let physical_addr = unsafe {
            pmap_extract(kernel_pmap, kernel_addr.wrapping_add(r_size))
        };
        // SAFETY: `pmap` is the live pmap of `map`, and the user address was
        // just entered in it.
        unsafe {
            pmap_enter(
                pmap,
                uaddr.wrapping_add(r_size),
                physical_addr,
                protection,
                c_int::from(wired != 0),
            )
        };
        r_size = r_size.wrapping_add(PAGE_SIZE);
    }

    Ok(uaddr)
}

/// `projected_buffer_deallocate()` in C: unmap a projected buffer from `map`,
/// and from the kernel map when it was the last non-persistent use.
pub(crate) fn projected_buffer_deallocate(
    map: NonNull<VmMap>,
    start: VmOffset,
    end: VmOffset,
) -> Result<(), Error> {
    let kernel = kernel_map_non_null();
    if map == kernel {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a valid map; the write lock serializes the
    // entry surgery below.
    VmMap::lock(map);
    let (found, entry) = unsafe { (*map.as_ptr()).lookup_entry(start) };
    let projected = unsafe { (*entry.as_ptr()).projected_on };
    if !found
        || end > unsafe { (*entry.as_ptr()).links.end }
        || projected.is_null()
    {
        VmMap::unlock(map);
        return Err(Error::InvalidArgument);
    }
    // SAFETY: the check above read the non-null kernel entry.
    let k_entry = unsafe { NonNull::new_unchecked(projected) };

    // SAFETY: `entry` is a live entry of the locked map, and the clips split
    // it only when the range lies strictly inside.
    unsafe {
        if (*entry.as_ptr()).links.start < start {
            (*map.as_ptr()).hdr.clip_start(entry, start, true);
        }
        if (*entry.as_ptr()).links.end > end {
            (*map.as_ptr()).hdr.clip_end(entry, end, true);
        }
        if (*map.as_ptr()).first_free == entry.as_ptr() {
            (*map.as_ptr()).first_free = (*entry.as_ptr())
                .links
                .prev
                .map_or(ptr::null_mut(), NonNull::as_ptr);
        }
        (*entry.as_ptr()).projected_on = ptr::null_mut();
        (*entry.as_ptr()).wired_count = 0;
        (*map.as_ptr()).entry_delete(entry);
    }
    VmMap::unlock(map);

    // SAFETY: the boot map is live; the write lock serializes the kernel
    // entry surgery.
    VmMap::lock(kernel);
    // SAFETY: `k_entry` is the live kernel entry the user entry pointed at.
    let non_persistent = unsafe { (*k_entry.as_ptr()).projection() }
        == Projection::NonPersistent;
    let last_reference = unsafe {
        !(*k_entry.as_ptr()).object.vm_object.is_null()
            && (*(*k_entry.as_ptr()).object.vm_object).ref_count == 1
    };
    if non_persistent && last_reference {
        // SAFETY: `k_entry` is linked in the locked boot map.
        unsafe {
            if (*kernel.as_ptr()).first_free == k_entry.as_ptr() {
                (*kernel.as_ptr()).first_free = (*k_entry.as_ptr())
                    .links
                    .prev
                    .map_or(ptr::null_mut(), NonNull::as_ptr);
            }
            (*k_entry.as_ptr()).projected_on = ptr::null_mut();
            (*kernel.as_ptr()).entry_delete(k_entry);
        }
    }
    VmMap::unlock(kernel);
    Ok(())
}

/// `kmem_alloc()` in C: allocate wired-down memory in a kernel map or
/// submap, not zeroed.
pub(crate) fn kmem_alloc(
    map: NonNull<VmMap>,
    size: VmSize,
) -> Result<VmOffset, Error> {
    let size = round_page(size);
    // SAFETY: the object allocator halts the kernel rather than fail.
    let object = unsafe { allocate(size) }
        .unwrap_or_else(|| die("vm_object_allocate"))
        .as_ptr();

    let mut attempts = 0;
    loop {
        // SAFETY: the caller promises a valid map; the write lock serializes
        // the entry search.
        VmMap::lock(map);
        let found = unsafe {
            VmMap::find_entry(
                &mut *map.as_ptr(),
                size,
                0,
                ptr::null_mut(),
                VmProt::READ | VmProt::WRITE,
                VmProt::ALL,
            )
        };
        match found {
            Ok((addr, entry)) => {
                // SAFETY: `entry` is the new entry of the locked map, and the
                // reference it takes is the allocator's.
                unsafe {
                    (*entry.as_ptr()).object.vm_object = object;
                    (*entry.as_ptr()).offset = 0;
                }
                VmMap::unlock(map);
                // SAFETY: `object` owns the range `addr..addr + size` the
                // entry just took.
                unsafe {
                    alloc_pages(
                        object,
                        0,
                        addr,
                        addr.wrapping_add(size),
                        VmProt::READ | VmProt::WRITE,
                        VM_PAGE_HIGHMEM,
                    )
                };
                return Ok(addr);
            }
            Err(error) => {
                VmMap::unlock(map);
                if attempts == 0 {
                    attempts += 1;
                    slab_collect();
                    continue;
                }
                // The C `printf_once` guards a diagnostic; a relaxed flag is
                // enough because the worst case is printing it twice.
                static PRINTED: AtomicBool = AtomicBool::new(false);
                if !PRINTED.swap(true, Ordering::Relaxed) {
                    // SAFETY: `map.name` is the map's NUL-terminated name.
                    let name =
                        unsafe { CStrArg::from_ptr((*map.as_ptr()).name) };
                    kprint!(
                        "no more room for kmem_alloc in {:x} ({})\n",
                        map.as_ptr().expose_provenance(),
                        name,
                    );
                }
                // SAFETY: the entry search failed, so the allocator's
                // reference is the only one.
                unsafe { deallocate(object) };
                return Err(error);
            }
        }
    }
}

/// `kmem_valloc()` in C: reserve addressing space in a kernel map or submap
/// without mapping anything.
pub(crate) fn kmem_valloc(
    map: NonNull<VmMap>,
    size: VmSize,
) -> Result<VmOffset, Error> {
    let size = round_page(size);
    // SAFETY: `kernel_object` is the boot object of every kernel mapping.
    let object = unsafe { kernel_object };

    let mut attempts = 0;
    loop {
        // SAFETY: the caller promises a valid map; the write lock serializes
        // the entry search.
        VmMap::lock(map);
        let found = unsafe {
            VmMap::find_entry(
                &mut *map.as_ptr(),
                size,
                0,
                object,
                VmProt::READ | VmProt::WRITE,
                VmProt::ALL,
            )
        };
        match found {
            Ok((addr, entry)) => {
                let offset = addr.wrapping_sub(VM_MIN_KERNEL_ADDRESS);
                // SAFETY: `entry` is the new or extended entry of the locked
                // map; only a fresh entry needs the object reference.
                unsafe {
                    if (*entry.as_ptr()).object.vm_object.is_null() {
                        reference(object);
                        (*entry.as_ptr()).object.vm_object = object;
                        (*entry.as_ptr()).offset = offset;
                    }
                }
                VmMap::unlock(map);
                return Ok(addr);
            }
            Err(error) => {
                VmMap::unlock(map);
                if attempts == 0 {
                    attempts += 1;
                    slab_collect();
                    continue;
                }
                // The C `printf_once` guards a diagnostic; a relaxed flag is
                // enough because the worst case is printing it twice.
                static PRINTED: AtomicBool = AtomicBool::new(false);
                if !PRINTED.swap(true, Ordering::Relaxed) {
                    // SAFETY: `map.name` is the map's NUL-terminated name.
                    let name =
                        unsafe { CStrArg::from_ptr((*map.as_ptr()).name) };
                    kprint!(
                        "no more room for kmem_valloc in {:x} ({})\n",
                        map.as_ptr().expose_provenance(),
                        name,
                    );
                }
                return Err(error);
            }
        }
    }
}

/// `kmem_alloc_aligned()` in C: `kmem_valloc()` with an aligned address and
/// the pages wired in.
///
/// # Panics
///
/// The caller must pass a power-of-two `size`; anything else halts the
/// kernel, as the C `panic("kmem_alloc_aligned")` did.
pub(crate) fn kmem_alloc_aligned(
    map: NonNull<VmMap>,
    size: VmSize,
) -> Result<VmOffset, Error> {
    let size = round_page(size);
    // SAFETY: `kernel_object` is the boot object of every kernel mapping.
    let object = unsafe { kernel_object };

    let mut attempts = 0;
    loop {
        // SAFETY: the caller promises a valid map; the write lock serializes
        // the entry search.
        VmMap::lock(map);
        let found = unsafe {
            VmMap::find_entry(
                &mut *map.as_ptr(),
                size,
                size.wrapping_sub(1),
                object,
                VmProt::READ | VmProt::WRITE,
                VmProt::ALL,
            )
        };
        match found {
            Ok((addr, entry)) => {
                let offset = addr.wrapping_sub(VM_MIN_KERNEL_ADDRESS);
                // SAFETY: `entry` is the new or extended entry of the locked
                // map; only a fresh entry needs the object reference.
                unsafe {
                    if (*entry.as_ptr()).object.vm_object.is_null() {
                        reference(object);
                        (*entry.as_ptr()).object.vm_object = object;
                        (*entry.as_ptr()).offset = offset;
                    }
                }
                VmMap::unlock(map);
                // SAFETY: `object` owns the range `addr..addr + size` the
                // entry just took.
                unsafe {
                    alloc_pages(
                        object,
                        offset,
                        addr,
                        addr.wrapping_add(size),
                        VmProt::READ | VmProt::WRITE,
                        VM_PAGE_HIGHMEM,
                    )
                };
                return Ok(addr);
            }
            Err(error) => {
                VmMap::unlock(map);
                if attempts == 0 {
                    attempts += 1;
                    slab_collect();
                    continue;
                }
                // The C `printf_once` guards a diagnostic; a relaxed flag is
                // enough because the worst case is printing it twice.
                static PRINTED: AtomicBool = AtomicBool::new(false);
                if !PRINTED.swap(true, Ordering::Relaxed) {
                    // SAFETY: `map.name` is the map's NUL-terminated name.
                    let name =
                        unsafe { CStrArg::from_ptr((*map.as_ptr()).name) };
                    kprint!(
                        "no more room for kmem_alloc_aligned in {:x} ({})\n",
                        map.as_ptr().expose_provenance(),
                        name,
                    );
                }
                return Err(error);
            }
        }
    }
}

/// `kmem_alloc_pages()` in C: allocate wired pages of `object` in
/// `start..end`.
///
/// # Safety
///
/// `object` must be a live object mapped into the kernel map, and its lock
/// must not be held.
pub(crate) unsafe fn alloc_pages(
    object: *mut VmObject,
    mut offset: VmOffset,
    mut start: VmOffset,
    end: VmOffset,
    protection: VmProt,
    flags: c_uint,
) {
    // SAFETY: `kernel_pmap` is the boot pmap.
    unsafe { pmap_pageable(kernel_pmap, start, end, c_int::from(false)) };

    while start < end {
        // SAFETY: the caller promises a live object.
        let object_ref = unsafe { NonNull::new_unchecked(object) };
        // SAFETY: the object lock guards the page tables this walk reads.
        unsafe { (*object).lock.lock() };

        let mem = loop {
            // SAFETY: the object lock is held, as `vm_page_alloc_flags`
            // requires.
            if let Some(page) =
                unsafe { vm_resident::alloc_flags(object_ref, offset, flags) }
            {
                break page;
            }
            // SAFETY: the C drops the object lock before waiting for a page.
            unsafe {
                (*object).lock.unlock();
                vm_page::wait(None);
                (*object).lock.lock();
            }
        };

        // SAFETY: the page-queues lock is the live lock and the object lock
        // is held.
        unsafe {
            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_page::wire(mem);
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
        }
        // SAFETY: the page is wired; the C unlocks the object before entering
        // the mapping.
        unsafe { (*object).lock.unlock() };

        // SAFETY: `kernel_pmap` is the boot pmap, and the page's physical
        // address is live; the C masks the page's lock bits out.
        unsafe {
            pmap_enter(
                kernel_pmap,
                start,
                (*mem.as_ptr()).phys_addr,
                protection & !(*mem.as_ptr()).page_lock(),
                c_int::from(true),
            )
        };

        // SAFETY: the object lock guards the page's busy state.
        unsafe {
            (*object).lock.lock();
            vm_object::page_wakeup_done(mem.as_ptr());
            (*object).lock.unlock();
        }

        start = start.wrapping_add(PAGE_SIZE);
        offset = offset.wrapping_add(PAGE_SIZE);
    }
}

/// `kmem_remap_pages()` in C: wire pages `object` already holds into a new
/// kernel range.
///
/// # Safety
///
/// `object` must be a live object mapped into the kernel map, and its lock
/// must not be held.
pub(crate) unsafe fn remap_pages(
    object: *mut VmObject,
    mut offset: VmOffset,
    mut start: VmOffset,
    end: VmOffset,
    protection: VmProt,
) {
    // SAFETY: `kernel_pmap` is the boot pmap.
    unsafe { pmap_pageable(kernel_pmap, start, end, c_int::from(false)) };

    while start < end {
        // SAFETY: the caller promises a live object.
        let object_ref = unsafe { NonNull::new_unchecked(object) };
        // SAFETY: the object lock guards the page table this walk reads.
        unsafe { (*object).lock.lock() };

        // SAFETY: the object lock is held, as `vm_page_lookup` requires; the
        // C halts when the page is missing.
        let mem = unsafe { vm_resident::lookup(object_ref, offset) }
            .unwrap_or_else(|| die("kmem_remap_pages"));

        // SAFETY: the page-queues lock is the live lock and the object lock
        // is held.
        unsafe {
            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_page::wire(mem);
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
            (*object).lock.unlock();
        }

        // SAFETY: `kernel_pmap` is the boot pmap, and the page's physical
        // address is live; the C masks the page's lock bits out.
        unsafe {
            pmap_enter(
                kernel_pmap,
                start,
                (*mem.as_ptr()).phys_addr,
                protection & !(*mem.as_ptr()).page_lock(),
                c_int::from(true),
            )
        };

        start = start.wrapping_add(PAGE_SIZE);
        offset = offset.wrapping_add(PAGE_SIZE);
    }
}

/// The `vm_page_t *` range and the reservation `kmem_io_map_copyout()` made.
pub(crate) struct IoMap {
    pub addr: VmOffset,
    pub alloc_addr: VmOffset,
    pub alloc_size: VmSize,
}

/// `kmem_io_map_copyout()` in C: establish a temporary read-only mapping of a
/// page-list copy.
pub(crate) fn kmem_io_map_copyout(
    map: &mut VmMap,
    copy: NonNull<VmMapCopy>,
    min_size: VmSize,
) -> Result<IoMap, Error> {
    // SAFETY: the caller promises a live page-list copy.
    let (copy_offset, copy_size) =
        unsafe { ((*copy.as_ptr()).offset, (*copy.as_ptr()).size) };
    let min_size = round_page(
        min_size
            .wrapping_add(copy_offset.wrapping_sub(trunc_page(copy_offset))),
    );
    let mut mysize = round_page(copy_offset.wrapping_add(copy_size))
        .wrapping_sub(trunc_page(copy_offset));

    // SAFETY: as above.
    let mut pages = unsafe { VmMapCopy::page_list(copy) };
    // SAFETY: `pages` names the live page-list variant.
    let npages = unsafe { (*pages).npages };
    let list_size = usize::try_from(npages)
        .unwrap_or(0)
        .wrapping_shl(crate::arch::vm_param::PAGE_SHIFT);
    if mysize > list_size && list_size > min_size {
        mysize = list_size;
    }

    let mut myaddr = map.hdr.links.start;
    map.enter(EnterRequest {
        address: &mut myaddr,
        size: mysize,
        mask: 0,
        anywhere: true,
        object: ptr::null_mut(),
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    })?;

    let pmap = map.pmap;
    pmap_pageable(
        pmap,
        myaddr,
        myaddr.wrapping_add(mysize),
        c_int::from(true),
    );

    let addr =
        myaddr.wrapping_add(copy_offset.wrapping_sub(trunc_page(copy_offset)));
    let mut offset = myaddr;
    loop {
        // SAFETY: `pages` names the live page-list variant of `copy`.
        let npages = unsafe { (*pages).npages };
        let npages = usize::try_from(npages).unwrap_or(0);
        let mut i = 0;
        while i < npages {
            // SAFETY: `i` is below `npages`, which the page-list copyin
            // bounds by the array length.
            let page = unsafe { (*pages).page_list[i] };
            // SAFETY: `pmap` is the live pmap of the map, and the page's
            // physical address is live.
            unsafe {
                pmap_enter(
                    pmap,
                    offset,
                    (*page).phys_addr,
                    VmProt::READ,
                    c_int::from(true),
                )
            };
            offset = offset.wrapping_add(PAGE_SIZE);
            i += 1;
        }

        if offset == myaddr.wrapping_add(mysize) {
            break;
        }

        // SAFETY: `pages` names the live variant, whose continuation the
        // page-list copyin installed.
        let cont = unsafe { (*pages).cont };
        let cont_args = unsafe { (*pages).cont_args };
        let mut new_copy: *mut VmMapCopy = ptr::null_mut();
        let ret = match cont {
            Some(cont) => unsafe { cont(cont_args, &mut new_copy) },
            None => {
                kmem_io_map_deallocate(map, myaddr, mysize);
                return Err(Error::Failure);
            }
        };
        // SAFETY: the continuation is consumed by this call, as its contract
        // requires.
        unsafe { (*pages).cont = None };
        if ret != KERN_SUCCESS {
            kmem_io_map_deallocate(map, myaddr, mysize);
            return Err(error_from_kern_return(ret)
                .err()
                .unwrap_or(Error::Failure));
        }
        // SAFETY: a successful continuation returned a live copy.
        let new_copy = unsafe { NonNull::new_unchecked(new_copy) };
        // SAFETY: the old copy now owns the new one and discards it when its
        // own continuation runs.
        unsafe {
            (*pages).cont =
                Some(crate::vm::vm_map_ffi::vm_map_copy_discard_cont);
            (*pages).cont_args = new_copy.as_ptr().cast::<VmMapCopyinArgs>();
        }
        // SAFETY: `new_copy` is the new live page-list copy.
        pages = unsafe { VmMapCopy::page_list(new_copy) };
    }

    Ok(IoMap {
        addr,
        alloc_addr: myaddr,
        alloc_size: mysize,
    })
}

/// `copyinmap()` in C: `copyin()` from a kernel map or the current user map.
///
/// # Safety
///
/// `fromaddr` must be readable and `toaddr` writable for `length` bytes, as
/// the C `copyin` required.
pub(crate) unsafe fn copyinmap(
    map: &VmMap,
    fromaddr: *const c_char,
    toaddr: *mut c_char,
    length: c_int,
) -> c_int {
    // SAFETY: `kernel_pmap` is the boot pmap.
    if map.pmap == unsafe { kernel_pmap } {
        // SAFETY: the caller promises `length` readable bytes at `fromaddr`
        // and writable bytes at `toaddr`.
        unsafe {
            ptr::copy_nonoverlapping(
                fromaddr,
                toaddr,
                usize::try_from(length).unwrap_or(0),
            )
        };
        return 0;
    }

    // SAFETY: `current_task()` is the running task, whose map is live.
    let current_map = unsafe { (*current_task()).map };
    if current_map == ptr::from_ref(map).cast_mut().cast::<c_void>() {
        // SAFETY: `copyin` is the real asm routine, and the C's `int`
        // argument converts to its `size_t` parameter.
        return unsafe {
            copyin(
                fromaddr.cast::<c_void>(),
                toaddr.cast::<c_void>(),
                length as usize,
            )
        };
    }

    1
}

/// `copyoutmap()` in C: `copyout()` into a kernel map or the current user
/// map.
///
/// # Safety
///
/// `fromaddr` must be readable and `toaddr` writable for `length` bytes, as
/// the C `copyout` required.
pub(crate) unsafe fn copyoutmap(
    map: &VmMap,
    fromaddr: *const c_char,
    toaddr: *mut c_char,
    length: c_int,
) -> c_int {
    // SAFETY: `kernel_pmap` is the boot pmap.
    if map.pmap == unsafe { kernel_pmap } {
        // SAFETY: the caller promises `length` readable bytes at `fromaddr`
        // and writable bytes at `toaddr`.
        unsafe {
            ptr::copy_nonoverlapping(
                fromaddr,
                toaddr,
                usize::try_from(length).unwrap_or(0),
            )
        };
        return 0;
    }

    // SAFETY: `current_task()` is the running task, whose map is live.
    let current_map = unsafe { (*current_task()).map };
    if current_map == ptr::from_ref(map).cast_mut().cast::<c_void>() {
        // SAFETY: `copyout` is the real asm routine, and the C's `int`
        // argument converts to its `size_t` parameter.
        return unsafe {
            copyout(
                fromaddr.cast::<c_void>(),
                toaddr.cast::<c_void>(),
                length as usize,
            )
        };
    }

    1
}
