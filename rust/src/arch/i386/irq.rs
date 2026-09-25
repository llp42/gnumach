// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/irq.c:
//   Copyright (C) 1995 Shantanu Goel
//   Copyright (C) 2020 Free Software Foundation, Inc
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt vector accessors and the per-line disable counts, which
//! `i386/i386/irq.c` used to define and `i386/i386/irq.h` declares; the
//! `struct irqdev` table and the `user_intr_t` entries come from
//! <device/intr.h>.

use crate::arch::i386::ioapic::{self, InterruptHandler};
use crate::config::NINTR;
use crate::glue;
use crate::kern::queue::QueueEntry;
use crate::spin::Mutex;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::mem::{align_of, offset_of, size_of};

/// `struct irqdev` of <device/intr.h>: one interrupt controller's table.
#[repr(C)]
pub struct IrqDev {
    pub name: *mut c_char,
    pub irqdev_ack: Option<unsafe extern "C" fn(*mut IrqDev, c_int)>,
    pub intr_queue: *mut QueueEntry,
    pub tot_num_intr: c_int,
    pub irq: [c_uint; NINTR],
}

/// `user_intr_t` of <device/intr.h>: one userland interrupt registration.
#[repr(C)]
pub struct UserIntr {
    pub chain: QueueEntry,
    pub interrupts: c_int,
    pub n_unacked: c_int,
    pub dst_port: *mut c_void,
    pub id: c_int,
}

// The offsets below are the ones gdb prints for `struct irqdev` and
// `user_intr_t` in build-64/gnumach and build-32/gnumach.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IrqDev>() == 288);
    assert!(align_of::<IrqDev>() == 8);
    assert!(offset_of!(IrqDev, name) == 0);
    assert!(offset_of!(IrqDev, irqdev_ack) == 8);
    assert!(offset_of!(IrqDev, intr_queue) == 16);
    assert!(offset_of!(IrqDev, tot_num_intr) == 24);
    assert!(offset_of!(IrqDev, irq) == 28);

    assert!(size_of::<UserIntr>() == 40);
    assert!(align_of::<UserIntr>() == 8);
    assert!(offset_of!(UserIntr, chain) == 0);
    assert!(offset_of!(UserIntr, interrupts) == 16);
    assert!(offset_of!(UserIntr, n_unacked) == 20);
    assert!(offset_of!(UserIntr, dst_port) == 24);
    assert!(offset_of!(UserIntr, id) == 32);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IrqDev>() == 272);
    assert!(align_of::<IrqDev>() == 4);
    assert!(offset_of!(IrqDev, name) == 0);
    assert!(offset_of!(IrqDev, irqdev_ack) == 4);
    assert!(offset_of!(IrqDev, intr_queue) == 8);
    assert!(offset_of!(IrqDev, tot_num_intr) == 12);
    assert!(offset_of!(IrqDev, irq) == 16);

    assert!(size_of::<UserIntr>() == 24);
    assert!(align_of::<UserIntr>() == 4);
    assert!(offset_of!(UserIntr, chain) == 0);
    assert!(offset_of!(UserIntr, interrupts) == 8);
    assert!(offset_of!(UserIntr, n_unacked) == 12);
    assert!(offset_of!(UserIntr, dst_port) == 16);
    assert!(offset_of!(UserIntr, id) == 20);
};

/// `struct nested_irq` of `i386/i386/irq.c`: one line's disable count in its
/// own lock.  The C gave each entry a whole cache line, which the alignment
/// preserves.
#[repr(C, align(64))]
struct NestedIrq {
    ndisabled: Mutex<c_int>,
}

const _: () = assert!(size_of::<NestedIrq>() == 64);

static NESTED_IRQS: [NestedIrq; NINTR] = [const {
    NestedIrq {
        ndisabled: Mutex::new(0),
    }
}; NINTR];

/// The `irqtab.irq` initializer of `i386/i386/irq.c`: each line maps to
/// itself.
const fn irq_map() -> [c_uint; NINTR] {
    let mut table = [0; NINTR];
    let mut irq = 0;
    while irq < NINTR {
        // NINTR is 64, so the narrowing to `irq_t` loses nothing.
        table[irq] = irq as c_uint;
        irq += 1;
    }
    table
}

/// `irq_eoi()` of `i386/i386/irq.c`: acknowledge the line behind `dev`.
unsafe extern "C" fn irq_eoi(dev: *mut IrqDev, id: c_int) {
    let Ok(index) = usize::try_from(id) else {
        return;
    };
    // SAFETY: `dev` is the live `irqtab`, and `index` addresses one of its
    // NINTR `irq` entries.
    let Some(irq) = (unsafe { (*dev).irq.get(index) }) else {
        return;
    };
    let Ok(pin) = c_int::try_from(*irq) else {
        return;
    };
    ioapic::irq_eoi(pin);
}

/// `irqtab` of `i386/i386/irq.c`, which <i386/irq.h> declares.
#[unsafe(no_mangle)]
pub static mut irqtab: IrqDev = IrqDev {
    name: c"irq".as_ptr().cast_mut(),
    irqdev_ack: Some(irq_eoi),
    intr_queue: &raw mut glue::main_intr_queue,
    tot_num_intr: 0,
    irq: irq_map(),
};

/// The `NINTR`-bounded index of `irq`, or [`None`] outside the table.
fn index_of(irq: c_int) -> Option<usize> {
    let index = usize::try_from(irq).ok()?;
    if index < NINTR { Some(index) } else { None }
}

/// `irq_get_handler()` of `i386/i386/irq.c`.
pub(crate) fn handler(irq: c_int) -> InterruptHandler {
    let index = index_of(irq)?;
    // SAFETY: `index` is inside `ivect`, which the interrupt stubs and
    // `device/intr.c` read through the symbol C declares.
    unsafe {
        *(&raw const ioapic::ivect)
            .cast::<InterruptHandler>()
            .add(index)
    }
}

/// `irq_set_handler()` of `i386/i386/irq.c`.
pub(crate) fn set_handler(irq: c_int, handler: InterruptHandler) {
    let Some(index) = index_of(irq) else {
        return;
    };
    // SAFETY: `index` is inside `ivect`; nothing else writes that entry while
    // the caller adjusts the vector.
    unsafe {
        (&raw mut ioapic::ivect)
            .cast::<InterruptHandler>()
            .add(index)
            .write(handler)
    };
}

/// `irq_get_unit()` of `i386/i386/irq.c`.
pub(crate) fn unit(irq: c_int) -> c_int {
    let Some(index) = index_of(irq) else {
        return 0;
    };
    // SAFETY: `index` is inside `iunit`, the array `interrupt.S` reads.
    unsafe { *(&raw const ioapic::iunit).cast::<c_int>().add(index) }
}

/// `irq_set_unit()` of `i386/i386/irq.c`.
pub(crate) fn set_unit(irq: c_int, unit: c_int) {
    let Some(index) = index_of(irq) else {
        return;
    };
    // SAFETY: `index` is inside `iunit`.
    unsafe { *(&raw mut ioapic::iunit).cast::<c_int>().add(index) = unit };
}

/// `__disable_irq()` of `i386/i386/irq.c`: raise the line's disable count and
/// mask it on the first disable.
fn disable(irq: c_uint) {
    let Ok(pin) = c_int::try_from(irq) else {
        return;
    };
    let Ok(index) = usize::try_from(irq) else {
        return;
    };
    let Some(nested) = NESTED_IRQS.get(index) else {
        return;
    };

    // SAFETY: `splhigh()` is the real asm routine <machine/spl.h> declares,
    // and its result is only handed back to `splx()`.
    let saved = unsafe { glue::splhigh() };
    {
        let mut ndisabled = nested.ndisabled.lock();
        *ndisabled = ndisabled.wrapping_add(1);
        if *ndisabled == 1 {
            ioapic::mask(pin);
        }
    }
    // SAFETY: `saved` is the level `splhigh()` returned above.
    unsafe { glue::splx(saved) };
}

/// `__enable_irq()` of `i386/i386/irq.c`: lower the line's disable count and
/// unmask it on the last enable.
fn enable(irq: c_uint) {
    let Ok(pin) = c_int::try_from(irq) else {
        return;
    };
    let Ok(index) = usize::try_from(irq) else {
        return;
    };
    let Some(nested) = NESTED_IRQS.get(index) else {
        return;
    };

    // SAFETY: as `disable()`.
    let saved = unsafe { glue::splhigh() };
    {
        let mut ndisabled = nested.ndisabled.lock();
        *ndisabled = ndisabled.wrapping_sub(1);
        if *ndisabled == 0 {
            ioapic::unmask(pin);
        }
    }
    // SAFETY: `saved` is the level `splhigh()` returned above.
    unsafe { glue::splx(saved) };
}

/// `init_irqs()` of `i386/i386/irq.c`.
///
/// The C zeroed each entry's lock and count; the constructor above already
/// leaves every `NESTED_IRQS` entry in that state, so nothing is left to do.
#[unsafe(no_mangle)]
pub extern "C" fn init_irqs() {}

/// `__disable_irq()` of `i386/i386/irq.c`.
#[unsafe(no_mangle)]
pub extern "C" fn __disable_irq(irq: c_uint) {
    disable(irq);
}

/// `__enable_irq()` of `i386/i386/irq.c`.
#[unsafe(no_mangle)]
pub extern "C" fn __enable_irq(irq: c_uint) {
    enable(irq);
}
