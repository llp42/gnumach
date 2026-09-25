// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/gdt.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991 IBM Corporation.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The global descriptor table, which `i386/i386/gdt.c` used to define and
//! `i386/i386/gdt.h` declares.
//!
//! The `extern "C"` edge is in [`gdt_ffi`].

use crate::arch::i386::mp_desc::{self, GDTSZ};
#[cfg(target_pointer_width = "64")]
use crate::arch::i386::pcb;
use crate::arch::i386::pcb::RealDescriptor;
use crate::arch::i386::{percpu, seg};
use crate::arch::types::VmOffset;
use core::ffi::{c_int, c_ulong, c_ushort};
use core::mem::size_of;
use core::ptr;

/// `gdt` of <i386/gdt.h>: the boot CPU's table, which the other CPUs get
/// copies of through `mp_gdt`.
#[unsafe(no_mangle)]
pub(crate) static mut gdt: [RealDescriptor; GDTSZ] =
    [RealDescriptor::ZERO; GDTSZ];

/// The `limit` of the pseudo-descriptor `gdt_fill()` loads, whose type in the
/// C `struct pseudo_descriptor` is 16 bits.
const GDT_LIMIT: usize = GDTSZ * size_of::<RealDescriptor>() - 1;

const _: () = assert!(GDT_LIMIT <= u16::MAX as usize);

/// `LINEAR_MIN_KERNEL_ADDRESS - VM_MIN_KERNEL_ADDRESS` of
/// <i386/vm_param.h>, which is zero in the 32-bit build.
#[cfg(target_pointer_width = "32")]
const KERNEL_LINEAR_BASE: VmOffset = 0;
/// `LINEAR_MAX_KERNEL_ADDRESS - (LINEAR_MIN_KERNEL_ADDRESS
/// - VM_MIN_KERNEL_ADDRESS) - 1` of <i386/vm_param.h>, which is
/// `0xffffffff - 1` in the 32-bit build.
#[cfg(target_pointer_width = "32")]
const KERNEL_LINEAR_LIMIT: VmOffset = 0xffff_fffe;
/// `LINEAR_MAX_KERNEL_ADDRESS` of <i386/vm_param.h> in the 32-bit build.
#[cfg(target_pointer_width = "32")]
const LINEAR_DS_LIMIT: VmOffset = 0xffff_ffff;

/// `gdt_fill()` of `i386/i386/gdt.c`.
///
/// # Safety
///
/// `mygdt` must be valid for `GDTSZ` descriptors.
#[cfg_attr(target_pointer_width = "64", expect(unused_variables))]
unsafe fn gdt_fill(cpu: c_int, mygdt: *mut RealDescriptor) {
    #[cfg(target_pointer_width = "64")]
    // SAFETY: the caller promises a full table, and every selector names an
    // entry inside it.
    unsafe {
        seg::fill_gdt_descriptor(
            mygdt,
            seg::KERNEL_CS,
            0,
            0,
            seg::ACC_PL_K | seg::ACC_CODE_R,
            seg::SZ_64,
        );
        seg::fill_gdt_descriptor(
            mygdt,
            seg::KERNEL_DS,
            0,
            0,
            seg::ACC_PL_K | seg::ACC_DATA_W,
            seg::SZ_64,
        );
        seg::fill_gdt_descriptor(
            mygdt,
            seg::LINEAR_DS,
            0,
            0,
            seg::ACC_PL_K | seg::ACC_DATA_W,
            seg::SZ_64,
        );
    }

    #[cfg(target_pointer_width = "32")]
    // SAFETY: the caller promises a full table, and every selector names an
    // entry inside it.
    unsafe {
        seg::fill_gdt_descriptor(
            mygdt,
            seg::KERNEL_CS,
            KERNEL_LINEAR_BASE,
            KERNEL_LINEAR_LIMIT,
            seg::ACC_PL_K | seg::ACC_CODE_R,
            seg::SZ_32,
        );
        seg::fill_gdt_descriptor(
            mygdt,
            seg::KERNEL_DS,
            KERNEL_LINEAR_BASE,
            KERNEL_LINEAR_LIMIT,
            seg::ACC_PL_K | seg::ACC_DATA_W,
            seg::SZ_32,
        );
        seg::fill_gdt_descriptor(
            mygdt,
            seg::LINEAR_DS,
            0,
            LINEAR_DS_LIMIT,
            seg::ACC_PL_K | seg::ACC_DATA_W,
            seg::SZ_32,
        );
        // `kvtolin()` is the identity in both configured builds:
        // `LINEAR_MIN_KERNEL_ADDRESS` is `VM_MIN_KERNEL_ADDRESS` (x86_64) or
        // `VM_MAX_USER_ADDRESS`, which equals it (i386).
        let percpu_base = percpu::percpu_at(cpu) as VmOffset;
        seg::fill_gdt_descriptor(
            mygdt,
            seg::PERCPU_DS,
            percpu_base,
            percpu_base + size_of::<percpu::Percpu>() - 1,
            seg::ACC_PL_K | seg::ACC_DATA_W,
            seg::SZ_32,
        );
    }

    let pdesc = seg::PseudoDescriptor {
        limit: GDT_LIMIT as c_ushort,
        linear_base: mygdt as VmOffset as c_ulong,
    };
    seg::lgdt(&pdesc);
}

#[cfg(target_pointer_width = "64")]
/// `reload_gs_base()` of `i386/i386/gdt.c`.
fn reload_gs_base(cpu: c_int) {
    // Kernel addresses fit in the 64-bit MSR of the LP64 target.
    // SAFETY: the caller names a CPU whose per-CPU block exists.
    let base = unsafe { percpu::percpu_at(cpu) } as usize as u64;
    pcb::write_msr(pcb::MSR_REG_GSBASE, base);
    pcb::write_msr(pcb::MSR_REG_KGSBASE, 0);
}

/// `reload_segs()` of `i386/i386/gdt.c`.
fn reload_segs() {
    #[cfg(target_pointer_width = "32")]
    // SAFETY: `ds`, `es` and `ss` are loaded from the descriptor table
    // `gdt_fill()` just built; the far jump reloads `cs` the same way.
    unsafe {
        core::arch::asm!(
            "ljmp ${cs}, $2f",
            "2:",
            "movw {zero:x}, %ds",
            "movw {zero:x}, %es",
            "movw {zero:x}, %fs",
            "movw {zero:x}, %gs",
            "movw {ds:x}, %ds",
            "movw {ds:x}, %es",
            "movw {percpu:x}, %gs",
            "movw {ds:x}, %ss",
            cs = const seg::KERNEL_CS as u16,
            zero = in(reg) 0_u16,
            ds = in(reg) seg::KERNEL_DS as u16,
            percpu = in(reg) seg::PERCPU_DS as u16,
            options(att_syntax, nostack, preserves_flags),
        );
    }
}

/// `gdt_init()` of <i386/gdt.h>.
pub(crate) fn gdt_init() {
    // SAFETY: `gdt` is the boot CPU's table, valid for `GDTSZ` descriptors.
    unsafe { gdt_fill(0, ptr::addr_of_mut!(gdt).cast::<RealDescriptor>()) };
    reload_segs();
    #[cfg(target_pointer_width = "64")]
    reload_gs_base(0);
}

/// `ap_gdt_init()` of <i386/gdt.h>.
pub(crate) fn ap_gdt_init(cpu: c_int) {
    // SAFETY: `mp_desc_init()` stored this CPU's `mp_gdt[cpu]` entry, a full
    // table, before any CPU ran `ap_gdt_init()` on it.
    let mygdt = unsafe { (*ptr::addr_of!(mp_desc::mp_gdt))[cpu as usize] };
    // SAFETY: `mygdt` is this CPU's full table.
    unsafe { gdt_fill(cpu, mygdt) };
    reload_segs();
    #[cfg(target_pointer_width = "64")]
    reload_gs_base(cpu);
}
