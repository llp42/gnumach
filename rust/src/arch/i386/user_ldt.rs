// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/user_ldt.c:
//   Copyright (c) 1994,1993,1992,1991 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-thread LDT and user GDT entries, which `i386/i386/user_ldt.c` used
//! to define and `i386/i386/user_ldt.h` and the MIG `mach_i386` interface
//! declare.
//!
//! The `extern "C"` edge is in [`user_ldt_ffi`].

use crate::arch::i386::ldt;
use crate::arch::i386::pcb::{self, RealDescriptor, UserLdt};
use crate::arch::i386::percpu::current_thread;
use crate::arch::i386::seg;
use crate::arch::types::{VmOffset, VmSize};
use crate::ipc::ipc_init;
use crate::kern::slab::{kalloc, kfree};
use crate::kern::thread::Thread;
use crate::vm::error::{
    Error as VmError, KERN_INVALID_ARGUMENT, KERN_NO_SPACE,
    KERN_RESOURCE_SHORTAGE,
};
use crate::vm::types::VmProt;
use crate::vm::vm_kern;
use crate::vm::vm_map::{self, VmMapCopy};
use core::ffi::{c_int, c_uint};
use core::mem::size_of;
use core::ptr::{self, NonNull};

/// The `switch` labels of `i386_set_ldt()` accepted in a user descriptor, the
/// C's `case` values.
const ACCESS_CALL_GATE: u8 = seg::ACC_P | seg::ACC_CALL_GATE;
const ACCESS_DATA: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_DATA;
const ACCESS_DATA_W: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_DATA_W;
const ACCESS_DATA_E: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_DATA_E;
const ACCESS_DATA_EW: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_DATA_EW;
const ACCESS_CODE: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_CODE;
const ACCESS_CODE_R: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_CODE_R;
const ACCESS_CODE_C: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_CODE_C;
const ACCESS_CODE_CR: u8 = seg::ACC_P | seg::ACC_PL_U | seg::ACC_CODE_CR;
const ACCESS_CALL_GATE_16: u8 =
    seg::ACC_P | seg::ACC_PL_U | seg::ACC_CALL_GATE_16;

/// The C's `template` local: an empty descriptor with only the present bit.
const TEMPLATE: RealDescriptor = RealDescriptor {
    limit_low_base_low: 0,
    access_and_base_high: (seg::ACC_P as u32) << 8,
};

/// `struct descriptor` of <mach/i386/mach_i386_types.h>, the MIG view of a
/// descriptor.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Descriptor {
    pub low_word: c_uint,
    pub high_word: c_uint,
}

const _: () = {
    assert!(size_of::<Descriptor>() == 8);
    assert!(
        core::mem::align_of::<Descriptor>() == core::mem::align_of::<c_uint>()
    );
    assert!(core::mem::offset_of!(Descriptor, low_word) == 0);
    assert!(core::mem::offset_of!(Descriptor, high_word) == 4);
};

/// `sel_idx()` of <i386/seg.h> on a signed selector.
fn sel_idx(selector: c_int) -> c_int {
    selector >> 3
}

/// The failures `i386_set_ldt()` and `i386_get_ldt()` report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    /// `KERN_INVALID_ARGUMENT`.
    InvalidArgument,
    /// `KERN_NO_SPACE`.
    NoSpace,
    /// `KERN_RESOURCE_SHORTAGE`.
    ResourceShortage,
    /// A failure of the VM map operations.
    Vm(VmError),
}

impl Error {
    /// The `kern_return_t` the C caller sees.
    pub(crate) const fn as_kern_return(self) -> c_int {
        match self {
            Error::InvalidArgument => KERN_INVALID_ARGUMENT,
            Error::NoSpace => KERN_NO_SPACE,
            Error::ResourceShortage => KERN_RESOURCE_SHORTAGE,
            Error::Vm(error) => error.as_kern_return(),
        }
    }
}

/// The `Error` a VM failure stands for.
fn kern_error(error: VmError) -> Error {
    Error::Vm(error)
}

/// The descriptors of the kernel's IPC map, used as the source for the
/// copyout of an out-of-line descriptor list.
fn kernel_map() -> *mut crate::vm::vm_map::VmMap {
    ipc_init::kernel_map()
}

/// `user_ldt_free()` of <i386/user_ldt.h>.
///
/// # Safety
///
/// `user_ldt` must be a live LDT allocation from [`set_ldt()`], given up by
/// this call.
pub(crate) unsafe fn free(user_ldt: *mut UserLdt) {
    // SAFETY: the caller guarantees a live LDT allocation.
    let size = usize::from(unsafe { (*user_ldt).desc.limit_low() })
        + 1
        + size_of::<RealDescriptor>();
    // SAFETY: the object came from `kalloc()` with that size.
    unsafe { kfree(NonNull::new_unchecked(user_ldt.cast::<u8>()), size) };
}

/// `i386_set_ldt()` of `i386/i386/user_ldt.c`.
///
/// # Safety
///
/// `thread` must be null or a live thread; `desc_list` must point at `count`
/// writable descriptors when `desc_list_inline` is true, and at a live
/// `vm_map_copy` the caller owns when it is false.
pub(crate) unsafe fn set_ldt(
    thread: *mut Thread,
    first_selector: c_int,
    desc_list: *mut RealDescriptor,
    count: c_uint,
    desc_list_inline: bool,
) -> Result<(), Error> {
    if thread.is_null() {
        return Err(Error::InvalidArgument);
    }
    let min_selector = if thread == current_thread() {
        seg::LDTSZ as c_uint
    } else {
        0
    };
    // The C stored the signed shift in an `unsigned`, so a negative selector
    // becomes a large index and fails the bound below.
    let first_desc = sel_idx(first_selector) as c_uint;
    if first_desc < min_selector || first_desc > 8191 {
        return Err(Error::InvalidArgument);
    }
    // The MIG count is at most `0x7fffffff`, so the C's unsigned sum cannot
    // wrap before this check rejects the large values.
    if first_desc + count >= 8192 {
        return Err(Error::InvalidArgument);
    }

    let mut copyin_addr: Option<VmOffset> = None;
    let mut copy_object: Option<NonNull<VmMapCopy>> = None;
    let desc_list = if desc_list_inline {
        desc_list
    } else {
        // SAFETY: the caller's contract makes the pointer a live copy.
        let copy =
            unsafe { NonNull::new_unchecked(desc_list.cast::<VmMapCopy>()) };
        copy_object = Some(copy);
        // SAFETY: the kernel's IPC map is a live map.
        let map = unsafe { &mut *kernel_map() };
        // SAFETY: the caller owns the live copy, and the map is the kernel's
        // IPC map.
        let dst = match unsafe { map.copyout(VmMapCopy::duplicate(copy)) } {
            Ok(dst) => dst,
            Err(error) => return Err(kern_error(error)),
        };
        // The C ignores the pageable result.
        let _ = map.pageable(
            dst,
            dst + count as usize * size_of::<RealDescriptor>(),
            VmProt::READ | VmProt::WRITE,
            true,
            true,
        );
        copyin_addr = Some(dst);
        ptr::with_exposed_provenance_mut::<RealDescriptor>(dst)
    };

    let descriptors: &mut [RealDescriptor] = if count == 0 {
        &mut []
    } else {
        // SAFETY: the caller promises `count` writable descriptors for the
        // inline case, and the copyout above placed as many in the kernel map
        // otherwise.  The C mutates the list for a kernel call gate.
        unsafe { core::slice::from_raw_parts_mut(desc_list, count as usize) }
    };
    for dp in descriptors.iter_mut() {
        match dp.access() & !seg::ACC_A {
            0 | seg::ACC_P => (),
            ACCESS_CALL_GATE => {
                // SAFETY: the default LDT's first entry is the syscall gate.
                *dp = unsafe { ldt::entry(seg::sel_idx(seg::USER_SCALL)) };
            }
            ACCESS_DATA | ACCESS_DATA_W | ACCESS_DATA_E | ACCESS_DATA_EW
            | ACCESS_CODE | ACCESS_CODE_R | ACCESS_CODE_C | ACCESS_CODE_CR
            | ACCESS_CALL_GATE_16 => (),
            _ => {
                free_copy(copyin_addr, copy_object, count);
                return Err(Error::InvalidArgument);
            }
        }
    }

    let ldt_size_needed =
        size_of::<RealDescriptor>() * (first_desc + count) as usize;
    // SAFETY: the caller promises a live thread.
    let pcb = unsafe { (*thread).pcb };
    let mut new_ldt: Option<NonNull<UserLdt>> = None;

    loop {
        // SAFETY: the caller promises a live pcb, and `lock()` is the C
        // `simple_lock()` on it.
        unsafe { (*pcb).lock.lock() };
        // SAFETY: as above.
        let old_ldt = NonNull::new(unsafe { (*pcb).ims.ldt });
        let too_small = match old_ldt {
            None => true,
            Some(old) => {
                // SAFETY: `old` is the live LDT the pcb names.
                usize::from(unsafe { (*old.as_ptr()).desc.limit_low() }) + 1
                    < ldt_size_needed
            }
        };

        if too_small {
            let Some(new) = new_ldt else {
                // SAFETY: this CPU took the pcb lock above.
                unsafe { (*pcb).lock.unlock() };
                let Some(buf) =
                    kalloc(ldt_size_needed + size_of::<RealDescriptor>())
                else {
                    return Err(Error::ResourceShortage);
                };
                let new = buf.as_ptr().cast::<UserLdt>();
                // The C wrote the self descriptor field by field; a
                // `fill_descriptor()` would shift the wrapped
                // `ldt_size_needed - 1` of the zero-length case.  The size
                // is at most 8192 descriptors, so the `u32` cast is exact.
                //
                // SAFETY: the allocation holds the struct and the descriptor
                // table `ldt_size_needed` bytes long; `kvtolin()` is the
                // identity in both configured builds.
                unsafe {
                    let base = ptr::addr_of_mut!((*new).ldt) as VmOffset;
                    let desc = &mut (*new).desc;
                    desc.limit_low_base_low =
                        (ldt_size_needed as u32).wrapping_sub(1) & 0xffff
                            | (((base & 0xffff) as u32) << 16);
                    desc.access_and_base_high = ((base >> 16) & 0xff) as u32
                        | (u32::from(seg::ACC_P | seg::ACC_LDT) << 8)
                        | (((base >> 24) & 0xff) as u32) << 24;
                }
                new_ldt = NonNull::new(new);
                continue;
            };

            match old_ldt {
                Some(old) => {
                    // SAFETY: both LDTs are live allocations, and the old
                    // one's own size covers the bytes copied.
                    unsafe {
                        ptr::copy_nonoverlapping(
                            ptr::addr_of!((*old.as_ptr()).ldt).cast::<u8>(),
                            ptr::addr_of_mut!((*new.as_ptr()).ldt)
                                .cast::<u8>(),
                            usize::from((*old.as_ptr()).desc.limit_low()) + 1,
                        );
                    }
                }
                None => {
                    // SAFETY: `new` is the live allocation the branch above
                    // made.
                    let entries = unsafe {
                        ptr::addr_of_mut!((*new.as_ptr()).ldt)
                            .cast::<RealDescriptor>()
                    };
                    for i in 0..first_desc as usize {
                        // SAFETY: the index is below `LDTSZ` only in the
                        // first arm; the default LDT covers it there.
                        let entry = if i < seg::LDTSZ {
                            unsafe { ldt::entry(i) }
                        } else {
                            TEMPLATE
                        };
                        // SAFETY: the allocation holds `ldt_size_needed`
                        // descriptors and `first_desc` is inside that count.
                        unsafe { entries.add(i).write(entry) };
                    }
                }
            }

            // SAFETY: `new` is the live allocation the branch above made,
            // and this CPU holds the pcb lock.
            unsafe { (*pcb).ims.ldt = new.as_ptr() };
            new_ldt = old_ldt;
            if thread == current_thread() {
                // SAFETY: the caller passes a live thread whose pcb this is.
                unsafe { pcb::switch_ktss(pcb) };
            }
        }

        // SAFETY: the branch above made the pcb's LDT non-null.
        let target = unsafe { (*pcb).ims.ldt };
        // SAFETY: the branch above made the pcb's LDT at least
        // `ldt_size_needed` bytes, and `count` ends inside that size.
        unsafe {
            ptr::copy_nonoverlapping(
                descriptors.as_ptr(),
                ptr::addr_of_mut!((*target).ldt)
                    .cast::<RealDescriptor>()
                    .add(first_desc as usize),
                count as usize,
            );
            (*pcb).lock.unlock();
        }
        break;
    }

    if let Some(new) = new_ldt {
        // SAFETY: `new` is the allocation the loop replaced or never used.
        let size = usize::from(unsafe { (*new.as_ptr()).desc.limit_low() })
            + 1
            + size_of::<RealDescriptor>();
        // SAFETY: as above, and the object came from `kalloc()` with that
        // size.
        unsafe { kfree(new.cast::<u8>(), size) };
    }
    free_copy(copyin_addr, copy_object, count);

    Ok(())
}

/// Discard the kernel-mapped copy an out-of-line descriptor list was copied
/// out to, and the emptied copy object it came from.
fn free_copy(
    copyin_addr: Option<VmOffset>,
    copy_object: Option<NonNull<VmMapCopy>>,
    count: c_uint,
) {
    if let Some(addr) = copyin_addr {
        // SAFETY: `addr` came from `vm_map_copyout` on the kernel map.
        let _ = vm_kern::kmem_free(
            unsafe { &mut *kernel_map() },
            addr,
            count as usize * size_of::<RealDescriptor>(),
        );
    }
    if let Some(copy) = copy_object {
        // SAFETY: the caller owns the live copy, emptied by `duplicate()`.
        unsafe { VmMapCopy::discard(copy) };
    }
}

/// `i386_get_ldt()` of `i386/i386/user_ldt.c`.
///
/// # Safety
///
/// `thread` must be null or a live thread, and `out` must be the caller's
/// writable descriptor storage.
pub(crate) unsafe fn get_ldt(
    thread: *mut Thread,
    first_selector: c_int,
    selector_count: c_int,
    out: Option<&mut [RealDescriptor]>,
) -> Result<(c_uint, Option<NonNull<VmMapCopy>>), Error> {
    if thread.is_null() {
        return Err(Error::InvalidArgument);
    }
    let first_desc = sel_idx(first_selector);
    if !(0..=8191).contains(&first_desc) {
        return Err(Error::InvalidArgument);
    }
    if first_desc.wrapping_add(selector_count) >= 8192 {
        return Err(Error::InvalidArgument);
    }

    let capacity = out.as_ref().map_or(0, |list| list.len());
    // SAFETY: the caller promises a live thread.
    let pcb = unsafe { (*thread).pcb };
    let mut addr: Option<VmOffset> = None;
    let mut size: VmSize = 0;

    let (user_ldt, ldt_count, ldt_size) = loop {
        // SAFETY: the caller promises a live pcb, and `lock()` is the C
        // `simple_lock()` on it.
        unsafe { (*pcb).lock.lock() };
        // SAFETY: as above.
        let user_ldt = unsafe { (*pcb).ims.ldt };
        if user_ldt.is_null() {
            // SAFETY: this CPU took the pcb lock above.
            unsafe { (*pcb).lock.unlock() };
            if let Some(addr) = addr {
                // SAFETY: `addr` is this call's live kernel allocation.
                let _ = vm_kern::kmem_free(
                    unsafe { &mut *kernel_map() },
                    addr,
                    size,
                );
            }
            return Ok((0, None));
        }

        // The C compared the unsigned count against the signed selector
        // count, so a negative one becomes large here.
        // SAFETY: `user_ldt` is the live LDT read above.
        let mut ldt_count =
            (u32::from(unsafe { (*user_ldt).desc.limit_low() }) + 1)
                / size_of::<RealDescriptor>() as u32;
        ldt_count = ldt_count.wrapping_sub(first_desc as u32);
        if ldt_count > selector_count as u32 {
            ldt_count = selector_count as u32;
        }
        let ldt_size =
            (ldt_count as usize).wrapping_mul(size_of::<RealDescriptor>());

        if ldt_count as usize <= capacity {
            break (user_ldt, ldt_count, ldt_size);
        }

        let size_needed = vm_map::round_page(ldt_size);
        if size_needed <= size {
            break (user_ldt, ldt_count, ldt_size);
        }

        // SAFETY: this CPU took the pcb lock above.
        unsafe { (*pcb).lock.unlock() };
        if let Some(addr) = addr {
            // SAFETY: `addr` is this call's live kernel allocation.
            let _ =
                vm_kern::kmem_free(unsafe { &mut *kernel_map() }, addr, size);
        }
        size = size_needed;

        // SAFETY: the kernel IPC map is live and unlocked here.
        let map = unsafe { NonNull::new_unchecked(kernel_map()) };
        match vm_kern::kmem_alloc(map, size) {
            Ok(allocated) => addr = Some(allocated),
            Err(_) => return Err(Error::ResourceShortage),
        }
    };

    // SAFETY: `user_ldt` is the live LDT the loop's lock held.
    let source = unsafe { ptr::addr_of!((*user_ldt).ldt) }
        .cast::<RealDescriptor>()
        .wrapping_add(first_desc as usize);
    match addr {
        Some(addr) => {
            // SAFETY: the loop's allocation is at least `ldt_size` bytes, and
            // `ldt_count` is inside it.
            unsafe {
                ptr::copy_nonoverlapping(
                    source,
                    ptr::with_exposed_provenance_mut::<RealDescriptor>(addr),
                    ldt_count as usize,
                );
            }
        }
        None => {
            if let Some(out) = out {
                // The loop only breaks into this arm when the caller's
                // storage covers the count.
                let Some(dst) = out.get_mut(..ldt_count as usize) else {
                    // SAFETY: this CPU holds the pcb lock here.
                    unsafe { (*pcb).lock.unlock() };
                    return Err(Error::InvalidArgument);
                };
                // SAFETY: both sides hold `ldt_count` descriptors.
                unsafe {
                    ptr::copy_nonoverlapping(
                        source,
                        dst.as_mut_ptr(),
                        ldt_count as usize,
                    );
                }
            }
        }
    }
    // SAFETY: this CPU holds the pcb lock here.
    unsafe { (*pcb).lock.unlock() };

    let mut copy = None;
    if let Some(addr) = addr {
        let size_used = vm_map::round_page(ldt_size);
        if size_used != size {
            // SAFETY: `addr` is this call's live kernel allocation of
            // `size` bytes.
            let _ = vm_kern::kmem_free(
                unsafe { &mut *kernel_map() },
                addr + size_used,
                size - size_used,
            );
        }
        let size_left = size_used - ldt_size;
        if size_left > 0 {
            // SAFETY: the allocation reaches `size_used` bytes.
            unsafe {
                ptr::write_bytes(
                    ptr::with_exposed_provenance_mut::<u8>(addr + ldt_size),
                    0,
                    size_left,
                );
            }
        }

        // SAFETY: `addr` is this call's live kernel allocation, and the map
        // is unlocked.
        let map = unsafe { &mut *kernel_map() };
        // The C copied the kernel buffer, not the caller's list, into the
        // copy object; the port keeps that.
        match map.copyin(addr, size_used, true) {
            Ok(memory) => copy = Some(memory),
            Err(error) => return Err(kern_error(error)),
        }
    }

    Ok((ldt_count, copy))
}

/// `i386_set_gdt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread.  On return `selector` is written
/// when it was `-1` and a slot was free.
pub(crate) unsafe fn set_gdt(
    thread: *mut Thread,
    selector: *mut c_int,
    descriptor: Descriptor,
) -> Result<(), Error> {
    if thread.is_null() {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a live thread.
    let pcb = unsafe { (*thread).pcb };
    // SAFETY: as above; the `user_gdt` array is inside the live pcb.
    let user_gdt = unsafe { &mut (*pcb).ims.user_gdt };
    // SAFETY: the caller promises `selector` is readable.
    let selector_value = unsafe { *selector };
    let idx;

    if selector_value == -1 {
        let Some(free) = user_gdt
            .iter()
            .position(|entry| entry.access() & seg::ACC_P == 0)
        else {
            return Err(Error::NoSpace);
        };
        idx = free;
        // The slot index is below `USER_GDT_SLOTS`, so this widens exactly.
        let selector_int = free as c_int + sel_idx(seg::USER_GDT);
        // SAFETY: the caller promises `selector` is writable.
        unsafe { *selector = (selector_int << 3) | seg::SEL_PL_U };
    } else if (selector_value & (seg::SEL_LDT | seg::SEL_PL)) != seg::SEL_PL_U
        || sel_idx(selector_value) < sel_idx(seg::USER_GDT)
        || sel_idx(selector_value)
            >= sel_idx(seg::USER_GDT) + seg::USER_GDT_SLOTS as c_int
    {
        return Err(Error::InvalidArgument);
    } else {
        let index = sel_idx(selector_value) - sel_idx(seg::USER_GDT);
        // The bound above keeps the index inside `USER_GDT_SLOTS`.
        idx = index as usize;
    }

    let desc = RealDescriptor {
        limit_low_base_low: descriptor.low_word,
        access_and_base_high: descriptor.high_word,
    };
    if desc.access() & seg::ACC_P == 0 {
        user_gdt[idx] = RealDescriptor::ZERO;
    } else if (desc.access() & (seg::ACC_TYPE_USER | seg::ACC_PL))
        != (seg::ACC_TYPE_USER | seg::ACC_PL_U)
        || desc.granularity() & seg::SZ_64 != 0
    {
        return Err(Error::InvalidArgument);
    } else {
        user_gdt[idx] = desc;
    }

    if thread == current_thread() {
        // SAFETY: the caller passes a live thread whose pcb this is.
        unsafe { pcb::switch_ktss(pcb) };
    }
    Ok(())
}

/// `i386_get_gdt()` of the MIG `mach_i386` interface.
///
/// # Safety
///
/// `thread` must be null or a live thread, and `descriptor` writable.
pub(crate) unsafe fn get_gdt(
    thread: *mut Thread,
    selector: c_int,
    descriptor: *mut Descriptor,
) -> Result<(), Error> {
    if thread.is_null() {
        return Err(Error::InvalidArgument);
    }
    if (selector & (seg::SEL_LDT | seg::SEL_PL)) != seg::SEL_PL_U
        || sel_idx(selector) < sel_idx(seg::USER_GDT)
        || sel_idx(selector)
            >= sel_idx(seg::USER_GDT) + seg::USER_GDT_SLOTS as c_int
    {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises a live thread.
    let pcb = unsafe { (*thread).pcb };
    let index = (sel_idx(selector) - sel_idx(seg::USER_GDT)) as usize;
    // SAFETY: the bound check above keeps the index inside `user_gdt`.
    let entry = unsafe { (*pcb).ims.user_gdt[index] };
    // SAFETY: the caller promises the descriptor is writable.
    unsafe {
        *descriptor = Descriptor {
            low_word: entry.limit_low_base_low,
            high_word: entry.access_and_base_high,
        };
    }
    Ok(())
}
