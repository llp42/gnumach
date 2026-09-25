// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/ldt.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The default local descriptor table, which `i386/i386/ldt.c` used to define
//! and `i386/i386/ldt.h` declares.
//!
//! The `extern "C"` edge is in [`ldt_ffi`].

use crate::arch::i386::gdt;
use crate::arch::i386::mp_desc;
#[cfg(target_pointer_width = "64")]
use crate::arch::i386::pcb;
use crate::arch::i386::pcb::RealDescriptor;
#[cfg(target_pointer_width = "64")]
use crate::arch::i386::pmap;
use crate::arch::i386::seg;
use crate::arch::types::VmOffset;
use crate::arch::vm_param::VM_MAX_USER_ADDRESS;
use crate::glue;
#[cfg(target_pointer_width = "64")]
use crate::kern::debug::kpanic;
use core::ffi::c_int;
use core::mem::size_of;
use core::ptr;

/// `VM_MIN_USER_ADDRESS` of <i386/vm_param.h>.
const VM_MIN_USER_ADDRESS: VmOffset = 0;

/// `ldt` of <i386/ldt.h>: the default table every thread starts with.
#[unsafe(no_mangle)]
pub(crate) static mut ldt: [RealDescriptor; seg::LDTSZ] =
    [RealDescriptor::ZERO; seg::LDTSZ];

/// `EFL_IF` and `EFL_IOPL_USER` of <mach/machine/eflags.h>, the mask the
/// 64-bit build programs into `MSR_REG_FMASK`.
#[cfg(target_pointer_width = "64")]
const EFL_IF: u64 = 0x0000_0200;
#[cfg(target_pointer_width = "64")]
const EFL_IOPL_USER: u64 = 0x0000_3000;

/// `USER_SEGMENT_SIZEBITS` of `i386/i386/ldt.c`.
#[cfg(target_pointer_width = "64")]
const USER_SEGMENT_SIZEBITS: u8 = seg::SZ_64;
#[cfg(target_pointer_width = "32")]
const USER_SEGMENT_SIZEBITS: u8 = seg::SZ_32;

/// The `limit` of the LDT's own GDT descriptor.
const LDT_LIMIT: usize = seg::LDTSZ * size_of::<RealDescriptor>() - 1;

const _: () = assert!(LDT_LIMIT <= u16::MAX as usize);

/// The `i`th entry of the default LDT.
///
/// # Safety
///
/// `index` must be below `LDTSZ`.
pub(crate) unsafe fn entry(index: usize) -> RealDescriptor {
    // SAFETY: the caller promises the live index; a table entry is a value,
    // not a reference, so reading it cannot alias the table.
    unsafe {
        ptr::addr_of!(ldt)
            .cast::<RealDescriptor>()
            .add(index)
            .read()
    }
}

/// The 64-bit `syscall` enablement of `ldt_fill()`.
#[cfg(target_pointer_width = "64")]
fn enable_syscall() {
    if !pmap::cpu_has_feature(pmap::CPU_FEATURE_SEP) {
        kpanic!("ldt_fill", "syscall support is missing on 64 bit")
    }

    let efer = pcb::read_msr(pcb::MSR_REG_EFER) | pcb::MSR_EFER_SCE;
    pcb::write_msr(pcb::MSR_REG_EFER, efer);
    // The kernel addresses fit the 64-bit MSR of the LP64 target.
    let syscall64 = glue::syscall64 as *const () as usize as u64;
    pcb::write_msr(pcb::MSR_REG_LSTAR, syscall64);
    let star = ((u64::from(seg::USER_CS as u16) - 16) << 16
        | u64::from(seg::KERNEL_CS as u16))
        << 32;
    pcb::write_msr(pcb::MSR_REG_STAR, star);
    pcb::write_msr(pcb::MSR_REG_FMASK, EFL_IF | EFL_IOPL_USER);
}

/// `ldt_fill()` of `i386/i386/ldt.c`.
///
/// # Safety
///
/// `myldt` must be a live LDT and `mygdt` a table with a `KERNEL_LDT`
/// descriptor.
unsafe fn ldt_fill(myldt: *mut RealDescriptor, mygdt: *mut RealDescriptor) {
    // SAFETY: the caller promises the live table and LDT; `KERNEL_LDT` names
    // a system descriptor there.
    unsafe {
        seg::fill_gdt_sys_descriptor(
            mygdt,
            seg::KERNEL_LDT,
            myldt as VmOffset,
            LDT_LIMIT,
            seg::ACC_PL_K | seg::ACC_LDT,
            0,
        );
    }

    #[cfg(target_pointer_width = "64")]
    enable_syscall();

    #[cfg(target_pointer_width = "32")]
    // SAFETY: the caller promises the live table, and `USER_SCALL` names an
    // entry inside it.
    unsafe {
        seg::fill_ldt_gate(
            myldt,
            seg::USER_SCALL,
            glue::syscall as *const () as usize as VmOffset,
            seg::KERNEL_CS as u16,
            seg::ACC_PL_U | seg::ACC_CALL_GATE,
            0,
        );
    }

    let user_limit = VM_MAX_USER_ADDRESS - VM_MIN_USER_ADDRESS - 4096;
    // SAFETY: the caller promises the live table; the two selectors name
    // entries inside it.
    unsafe {
        seg::fill_ldt_descriptor(
            myldt,
            seg::USER_CS,
            VM_MIN_USER_ADDRESS,
            user_limit,
            seg::ACC_PL_U | seg::ACC_CODE_R,
            USER_SEGMENT_SIZEBITS,
        );
        seg::fill_ldt_descriptor(
            myldt,
            seg::USER_DS,
            VM_MIN_USER_ADDRESS,
            user_limit,
            seg::ACC_PL_U | seg::ACC_DATA_W,
            USER_SEGMENT_SIZEBITS,
        );
    }

    seg::lldt(seg::KERNEL_LDT as u16);
}

/// `ldt_init()` of <i386/ldt.h>.
pub(crate) fn ldt_init() {
    // SAFETY: `ldt` is the default table and `gdt` the boot CPU's full one.
    unsafe {
        ldt_fill(
            ptr::addr_of_mut!(ldt).cast::<RealDescriptor>(),
            ptr::addr_of_mut!(gdt::gdt).cast::<RealDescriptor>(),
        );
    }
}

/// `ap_ldt_init()` of <i386/ldt.h>.
pub(crate) fn ap_ldt_init(cpu: c_int) {
    // SAFETY: `mp_desc_init()` stored this CPU's table set before any CPU ran
    // `ap_ldt_init()` on it.
    let table =
        unsafe { (*ptr::addr_of!(mp_desc::mp_desc_table))[cpu as usize] };
    // SAFETY: as above.
    let mygdt = unsafe { (*ptr::addr_of!(mp_desc::mp_gdt))[cpu as usize] };
    // SAFETY: both are this CPU's live records.
    unsafe {
        ldt_fill(
            ptr::addr_of_mut!((*table).ldt).cast::<RealDescriptor>(),
            mygdt,
        );
    }
}
