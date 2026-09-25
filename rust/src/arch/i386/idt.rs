// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/idt.c:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt descriptor table, which `i386/i386/idt.c` used to define and
//! `i386/i386at/idt.h` declares.
//!
//! The `extern "C"` edge is in [`idt_ffi`].

use crate::arch::i386::mp_desc::{self, IDTSZ, RealGate};
use crate::arch::i386::seg;
use crate::arch::types::VmOffset;
use core::ffi::{c_int, c_ulong, c_ushort};
use core::mem::size_of;
use core::ptr;

/// `idt` of <i386at/idt-gen.h>: the boot CPU's table, which the other CPUs
/// get copies of through `mp_desc_table`.
#[unsafe(no_mangle)]
pub(crate) static mut idt: [RealGate; IDTSZ] = [RealGate::ZERO; IDTSZ];

/// `struct idt_init_entry` of `i386/i386/idt.c`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IdtInitEntry {
    entrypoint: c_ulong,
    vector: c_ushort,
    type_: c_ushort,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IdtInitEntry>() == 8);
    assert!(
        core::mem::align_of::<IdtInitEntry>()
            == core::mem::align_of::<c_ulong>()
    );
    assert!(core::mem::offset_of!(IdtInitEntry, entrypoint) == 0);
    assert!(core::mem::offset_of!(IdtInitEntry, vector) == 4);
    assert!(core::mem::offset_of!(IdtInitEntry, type_) == 6);
};

/// `struct idt_init_entry` of `i386/i386/idt.c`, the 64-bit layout.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IdtInitEntry {
    entrypoint: c_ulong,
    vector: c_ushort,
    type_: c_ushort,
    ist: c_ushort,
    pad_0: c_ushort,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IdtInitEntry>() == 16);
    assert!(
        core::mem::align_of::<IdtInitEntry>()
            == core::mem::align_of::<c_ulong>()
    );
    assert!(core::mem::offset_of!(IdtInitEntry, entrypoint) == 0);
    assert!(core::mem::offset_of!(IdtInitEntry, vector) == 8);
    assert!(core::mem::offset_of!(IdtInitEntry, type_) == 10);
    assert!(core::mem::offset_of!(IdtInitEntry, ist) == 12);
};

/// The `limit` of the pseudo-descriptor `idt_fill()` loads, whose type in the
/// C `struct pseudo_descriptor` is 16 bits.
const IDT_LIMIT: usize = IDTSZ * size_of::<RealGate>() - 1;

const _: () = assert!(IDT_LIMIT <= u16::MAX as usize);

/// `idt_fill()` of `i386/i386/idt.c`.
///
/// # Safety
///
/// `myidt` must be valid for `IDTSZ` gates.
unsafe fn idt_fill(myidt: *mut RealGate) {
    let mut iie = ptr::addr_of!(crate::glue::idt_inittab);

    loop {
        // SAFETY: the generated table is an array terminated by a zero
        // entrypoint, and the walk starts at its first element.
        let entry = unsafe { iie.read() };
        if entry.entrypoint == 0 {
            break;
        }

        #[cfg(target_pointer_width = "32")]
        let ist = 0_u8;
        // The C passed the 16-bit field into an `unsigned char` parameter,
        // and the hardware keeps the IST in the low three bits.
        #[cfg(target_pointer_width = "64")]
        let ist = entry.ist as u8;

        // The table's `type` is the access byte, an `unsigned short` the C
        // passed to a parameter that is an `unsigned char`; the generated
        // values fit.
        let access = entry.type_ as u8;

        // SAFETY: the caller promises a full table, and the vector is a
        // hardware vector from the generated table.
        unsafe {
            seg::fill_idt_gate(
                myidt,
                c_int::from(entry.vector),
                entry.entrypoint as VmOffset,
                seg::KERNEL_CS,
                access,
                ist,
            );
        }
        // SAFETY: the walk stops at the terminated table's sentinel before
        // this pointer leaves it.
        iie = unsafe { iie.add(1) };
    }

    let pdesc = seg::PseudoDescriptor {
        limit: IDT_LIMIT as c_ushort,
        linear_base: myidt as VmOffset as c_ulong,
    };
    seg::lidt(&pdesc);
}

/// `idt_init()` of <i386at/idt.h>.
pub(crate) fn idt_init() {
    // SAFETY: `idt` is the boot CPU's table, valid for `IDTSZ` gates.
    unsafe { idt_fill(ptr::addr_of_mut!(idt).cast::<RealGate>()) };
}

/// `ap_idt_init()` of <i386at/idt.h>.
pub(crate) fn ap_idt_init(cpu: c_int) {
    // SAFETY: `mp_desc_init()` stored this CPU's table before any CPU ran
    // `ap_idt_init()` on it.
    let table =
        unsafe { (*ptr::addr_of!(mp_desc::mp_desc_table))[cpu as usize] };
    // SAFETY: `table` is this CPU's full table.
    unsafe { idt_fill(ptr::addr_of_mut!((*table).idt).cast::<RealGate>()) };
}
