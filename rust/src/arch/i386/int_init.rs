// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/int_init.c:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt gate setup, which `i386/i386at/int_init.c` used to define and
//! `i386/i386at/int_init.h` declares.
//!
//! The `extern "C"` edge is in [`int_init_ffi`].

use crate::arch::i386::apic;
use crate::arch::i386::idt;
use crate::arch::i386::mp_desc::{self, RealGate};
use crate::arch::i386::seg;
use crate::config::NINTR;
use core::ffi::c_int;
use core::ptr;

/// `IOAPIC_INT_BASE` of <i386at/idt.h>: the first IOAPIC vector.
const IOAPIC_INT_BASE: usize = 0x30;
/// `CALL_AST_CHECK` of <i386at/idt.h>: the remote AST request vector.
const CALL_AST_CHECK: usize = 0xfa;
/// `CALL_PMAP_UPDATE` of <i386at/idt.h>: the TLB shootdown vector.
const CALL_PMAP_UPDATE: usize = 0xfb;

/// `int_entry_table[]` of `i386/i386/locore.S` and `x86_64/locore.S`, whose
/// entries `int_fill()` installs.
fn entry_table() -> *const crate::arch::types::VmOffset {
    ptr::addr_of!(crate::glue::int_entry_table)
}

/// `int_fill()` of `i386/i386at/int_init.c`, the `APIC` branch both
/// configured builds compile.
///
/// # Safety
///
/// `myidt` must be valid for `NINTR` interrupt gates and the three vectors
/// after them.
unsafe fn int_fill(myidt: *mut RealGate) {
    let table = entry_table();

    for i in 0..NINTR {
        // SAFETY: the generated table holds `NINTR` interrupt entries, and
        // the IOAPIC vectors are inside the IDT.
        unsafe {
            seg::fill_idt_gate(
                myidt,
                IOAPIC_INT_BASE as c_int + i as c_int,
                table.add(i).read(),
                seg::KERNEL_CS,
                seg::ACC_PL_K | seg::ACC_INTR_GATE,
                0,
            );
        }
    }

    // SAFETY: the generated table holds the three service entries after the
    // interrupt lines, and each vector is inside the IDT.
    unsafe {
        seg::fill_idt_gate(
            myidt,
            CALL_AST_CHECK as c_int,
            table.add(NINTR).read(),
            seg::KERNEL_CS,
            seg::ACC_PL_K | seg::ACC_INTR_GATE,
            0,
        );
        seg::fill_idt_gate(
            myidt,
            CALL_PMAP_UPDATE as c_int,
            table.add(NINTR + 1).read(),
            seg::KERNEL_CS,
            seg::ACC_PL_K | seg::ACC_INTR_GATE,
            0,
        );
        seg::fill_idt_gate(
            myidt,
            apic::IOAPIC_SPURIOUS_BASE as c_int,
            table.add(NINTR + 2).read(),
            seg::KERNEL_CS,
            seg::ACC_PL_K | seg::ACC_INTR_GATE,
            0,
        );
    }
}

/// `int_init()` of <i386at/int_init.h>.
pub(crate) fn int_init() {
    // SAFETY: `idt` is the boot CPU's table, valid for the vectors installed.
    unsafe { int_fill(ptr::addr_of_mut!(idt::idt).cast::<RealGate>()) };
}

/// `ap_int_init()` of <i386at/int_init.h>.
pub(crate) fn ap_int_init(cpu: c_int) {
    // SAFETY: `mp_desc_init()` stored this CPU's table before any CPU ran
    // `ap_int_init()` on it.
    let myidt =
        unsafe { (*ptr::addr_of!(mp_desc::mp_desc_table))[cpu as usize] };
    // SAFETY: `myidt` is this CPU's full table.
    unsafe { int_fill(ptr::addr_of_mut!((*myidt).idt).cast::<RealGate>()) };
}
