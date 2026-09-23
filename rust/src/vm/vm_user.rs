// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_user.c and vm/vm_user.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The user-exported virtual memory calls, which `vm/vm_user.c` used to
//! define and `vm/vm_user.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue::{memory_object_lock_request, vm_object_reference};
use crate::vm::error::{Error, error_from_kern_return};
use crate::vm::types::{VmInherit, VmObject, VmProt};
use crate::vm::vm_kern::projected_buffer_in_range;
use crate::vm::vm_map::{
    EnterRequest, VmMap, VmMapCopy, round_page, trunc_page,
};
use core::ffi::c_int;
use core::ptr::{self, NonNull};

/// `MEMORY_OBJECT_RETURN_NONE` of <mach/memory_object.h>.
const MEMORY_OBJECT_RETURN_NONE: c_int = 0;
/// `MEMORY_OBJECT_RETURN_ALL` of <mach/memory_object.h>.
const MEMORY_OBJECT_RETURN_ALL: c_int = 2;

/// `VM_PROT_ALL | VM_PROT_NOTIFY`: the protection bits `vm_protect()` accepts.
const VM_PROT_SETTER_MASK: c_int = VmProt::ALL.bits() | VmProt::NOTIFY.bits();

/// `vm_allocate()` in C: allocate zero-filled memory in `map`.
pub(crate) fn allocate(
    map: &mut VmMap,
    addr: &mut VmOffset,
    size: VmSize,
    anywhere: bool,
) -> Result<(), Error> {
    if size == 0 {
        *addr = 0;
        return Ok(());
    }

    if anywhere {
        *addr = map.hdr.links.start;
    } else {
        *addr = trunc_page(*addr);
    }

    map.enter(EnterRequest {
        address: addr,
        size: round_page(size),
        mask: 0,
        anywhere,
        object: ptr::null_mut(),
        offset: 0,
        needs_copy: false,
        cur_protection: VmProt::READ | VmProt::WRITE,
        max_protection: VmProt::ALL,
        inheritance: VmInherit::COPY,
    })
}

/// `vm_deallocate()` in C: drop the pages covering `start..start + size`.
pub(crate) fn deallocate(
    map: &mut VmMap,
    start: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    if size == 0 {
        return Ok(());
    }

    map.remove(trunc_page(start), round_page(start.wrapping_add(size)))
}

/// `vm_inherit()` in C: set the inheritance of a range.
pub(crate) fn inherit(
    map: &mut VmMap,
    start: VmOffset,
    size: VmSize,
    new_inheritance: VmInherit,
) -> Result<(), Error> {
    match new_inheritance {
        VmInherit::NONE | VmInherit::COPY | VmInherit::SHARE => (),
        _ => return Err(Error::InvalidArgument),
    }

    let end = start.wrapping_add(size);
    if projected_buffer_in_range(map, start, end) {
        return Err(Error::InvalidArgument);
    }

    map.inherit(trunc_page(start), round_page(end), new_inheritance)
}

/// `vm_protect()` in C: set the protection of a range.
pub(crate) fn protect(
    map: &mut VmMap,
    start: VmOffset,
    size: VmSize,
    set_maximum: bool,
    new_protection: VmProt,
) -> Result<(), Error> {
    if new_protection.bits() & !VM_PROT_SETTER_MASK != 0 {
        return Err(Error::InvalidArgument);
    }

    let end = start.wrapping_add(size);
    if projected_buffer_in_range(map, start, end) {
        return Err(Error::InvalidArgument);
    }

    map.protect(
        trunc_page(start),
        round_page(end),
        new_protection,
        set_maximum,
    )
}

/// `vm_machine_attribute()` in C: hand a machine attribute to the map's
/// physical map.
pub(crate) fn machine_attribute(
    map: &VmMap,
    address: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    if projected_buffer_in_range(map, address, address.wrapping_add(size)) {
        return Err(Error::InvalidArgument);
    }

    VmMap::machine_attribute(NonNull::from(map), address, size)
}

/// `vm_read()` in C: copy a range out as a map copy for the IPC layer.
pub(crate) fn read(
    map: &mut VmMap,
    address: VmOffset,
    size: VmSize,
) -> Result<Option<NonNull<VmMapCopy>>, Error> {
    copyin(map, address, size)
}

/// `vm_write()` in C: overwrite a range with the copy an IPC message
/// carried.
pub(crate) fn write(
    map: &mut VmMap,
    address: VmOffset,
    copy: Option<NonNull<VmMapCopy>>,
) -> Result<(), Error> {
    let Some(copy) = copy else {
        return Ok(());
    };

    // SAFETY: the caller owns the live copy; the C `vm_map_copy_overwrite()`
    // consumes it on success and leaves it to the caller on failure.
    unsafe { map.copy_overwrite(address, copy) }
}

/// `vm_copy()` in C: copy a range to another address in the same map.
pub(crate) fn copy(
    map: &mut VmMap,
    source_address: VmOffset,
    size: VmSize,
    dest_address: VmOffset,
) -> Result<(), Error> {
    let Some(copy) = copyin(map, source_address, size)? else {
        return Ok(());
    };

    // SAFETY: `copyin` returned a live copy this call owns;
    // `vm_map_copy_overwrite()` consumes it on success, and the C discards it
    // on failure.
    match unsafe { map.copy_overwrite(dest_address, copy) } {
        Ok(()) => Ok(()),
        Err(error) => {
            // SAFETY: the failed overwrite left the live copy to the caller.
            unsafe { VmMapCopy::discard(copy) };
            Err(error)
        }
    }
}

/// `vm_object_sync()` in C: write a range of `object` back to its memory
/// manager.
pub(crate) fn object_sync(
    object: NonNull<VmObject>,
    offset: VmOffset,
    size: VmSize,
    should_flush: bool,
    should_return: bool,
) -> Result<(), Error> {
    // SAFETY: the caller promises a live object; `memory_object_lock_request`
    // consumes the reference taken here.
    unsafe { vm_object_reference(object.as_ptr()) };

    let size =
        round_page(offset.wrapping_add(size)).wrapping_sub(trunc_page(offset));
    let offset = trunc_page(offset);
    let should_return = if should_return {
        MEMORY_OBJECT_RETURN_ALL
    } else {
        MEMORY_OBJECT_RETURN_NONE
    };

    // SAFETY: the object reference was just taken, and the C passes a null
    // reply port with no right to consume.
    error_from_kern_return(unsafe {
        memory_object_lock_request(
            object.as_ptr(),
            offset,
            size,
            should_return,
            c_int::from(should_flush),
            VmProt::NO_CHANGE,
            ptr::null_mut(),
            0,
        )
    })
}

/// `vm_msync()` in C: synchronize a range with its memory manager.
pub(crate) fn msync(
    map: &mut VmMap,
    address: VmOffset,
    size: VmSize,
    sync_flags: c_int,
) -> Result<(), Error> {
    VmMap::msync(Some(NonNull::from(map)), address, size, sync_flags)
}

/// `vm_get_size_limit()` in C: report the current and maximum virtual size
/// limits of `map`.
pub(crate) fn get_size_limit(map: &VmMap) -> (VmSize, VmSize) {
    map.lock.read();
    let limits = (map.size_cur_limit, map.size_max_limit);
    map.lock.done();
    limits
}

/// `vm_map_copyin()`'s zero-length case, which its FFI adapter keeps: a
/// zero-byte range yields no copy, while the map core rejects it as an
/// address overflow.
fn copyin(
    map: &mut VmMap,
    address: VmOffset,
    size: VmSize,
) -> Result<Option<NonNull<VmMapCopy>>, Error> {
    if size == 0 {
        return Ok(None);
    }

    map.copyin(address, size, false).map(Some)
}
