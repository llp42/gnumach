// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/smp.c and i386/i386/smp.h:
//   Copyright (C) 2020 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The i386 SMP controller, which `i386/i386/smp.c` used to define and
//! `i386/i386/smp.h` declares.

use crate::arch::i386::{apic, pit};
use crate::glue;
use crate::kern::machine;
use crate::kern::smp as kern_smp;
use core::arch::asm;
use core::ffi::{c_int, c_uint, c_ulong};

/// `STARTUP_VECTOR_SHIFT` of <i386/smp.h>: where a startup IPI carries its
/// target address.
const STARTUP_VECTOR_SHIFT: u32 = 20 - 8;
/// `NO_SHORTHAND` of <i386/apic.h>.
const NO_SHORTHAND: c_uint = 0;
/// `FIXED` of <i386/apic.h>.
const FIXED: c_uint = 0;
/// `PHYSICAL` of <i386/apic.h>.
const PHYSICAL: c_uint = 0;
/// `LOGICAL` of <i386/apic.h>.
const LOGICAL: c_uint = 1;
/// `EDGE` of <i386/apic.h>.
const EDGE: c_uint = 0;
/// `DE_ASSERT` of <i386/apic.h>.
const DE_ASSERT: c_uint = 0;
/// `ASSERT` of <i386/apic.h>.
const ASSERT: c_uint = 1;
/// `ALL_EXCLUDING_SELF` of <i386/apic.h>.
const ALL_EXCLUDING_SELF: c_uint = 3;
/// `INIT` of <i386/apic.h>.
const INIT: c_uint = 5;
/// `STARTUP` of <i386/apic.h>.
const STARTUP: c_uint = 6;
/// `CALL_AST_CHECK` of <i386at/idt.h>.
const CALL_AST_CHECK: c_uint = 0xfa;
/// `CALL_PMAP_UPDATE` of <i386at/idt.h>.
const CALL_PMAP_UPDATE: c_uint = 0xfb;

/// `cpu_pause()` of <i386/smp.h>: the spin-loop hint.
fn pause() {
    // SAFETY: `pause` touches no registers and the stack stays balanced; the
    // memory clobber of the C macro is the default.
    unsafe { asm!("pause", options(nostack, preserves_flags)) };
}

/// The local APIC's `error_status` register.
fn error_status() -> u32 {
    let ptr = apic::lapic_ptr();
    // SAFETY: `ptr` is the mapped local-APIC page, and the read is the C's
    // volatile access.
    unsafe { apic::reg_read(&raw const (*ptr).error_status) }
}

/// Clear the local APIC's `error_status` register.
fn clear_error_status() {
    let ptr = apic::lapic_ptr();
    // SAFETY: `ptr` is the mapped local-APIC page, and the write is the C's
    // volatile access.
    unsafe { apic::reg_write(&raw mut (*ptr).error_status, 0) };
}

/// `smp_data_init()` in C.
fn data_init() {
    let numcpus = apic::apic_get_numcpus();
    kern_smp::smp_set_numcpus(numcpus);

    for i in 0..usize::from(numcpus) {
        // SAFETY: `machine_slot` has `NCPUS` entries and the APIC probe never
        // reports more.
        unsafe { (*machine::slot(i)).is_cpu = 1 };
    }
}

/// `smp_send_ipi()` in C.
fn send_ipi(logical_id: c_uint, vector: c_uint) {
    let flags = apic::intr_save();

    while apic::ipi_pending() {
        pause();
    }

    apic::send_ipi(
        NO_SHORTHAND,
        FIXED,
        LOGICAL,
        ASSERT,
        EDGE,
        vector,
        logical_id,
    );

    apic::intr_restore(flags);
}

/// `wait_for_ipi()` in C.
fn wait_for_ipi() {
    while apic::ipi_pending() {
        pause();
    }
}

/// `smp_send_ipi_init()` in C.
fn send_ipi_init(bsp_apic_id: c_uint) -> c_int {
    clear_error_status();
    let _ = error_status();

    apic::send_ipi(
        ALL_EXCLUDING_SELF,
        INIT,
        PHYSICAL,
        ASSERT,
        EDGE,
        0,
        bsp_apic_id,
    );
    wait_for_ipi();

    apic::send_ipi(
        ALL_EXCLUDING_SELF,
        INIT,
        PHYSICAL,
        DE_ASSERT,
        EDGE,
        0,
        bsp_apic_id,
    );
    wait_for_ipi();

    let error = error_status();
    if error != 0 {
        // SAFETY: `printf` accepts the C format and arguments.
        unsafe { glue::printf(c"ESR error upon INIT 0x%x\n".as_ptr(), error) };
    }
    0
}

/// `smp_send_ipi_startup_twice()` in C.
fn send_ipi_startup_twice(bsp_apic_id: c_uint, vector: c_uint) -> c_int {
    let mut send_err = 0;
    let mut accept_err = 0;

    for _ in 0..2 {
        clear_error_status();
        let _ = error_status();

        apic::send_ipi(
            ALL_EXCLUDING_SELF,
            STARTUP,
            PHYSICAL,
            DE_ASSERT,
            EDGE,
            vector,
            bsp_apic_id,
        );

        pit::udelay(10);
        wait_for_ipi();
        send_err = error_status();

        pit::udelay(10);
        clear_error_status();
        accept_err = error_status() & 0xef;

        if send_err != 0 || accept_err != 0 {
            break;
        }
    }

    if send_err != 0 {
        // SAFETY: `printf` accepts the C format and arguments.
        unsafe {
            glue::printf(c"ESR error: DID NOT SEND? 0x%x\n".as_ptr(), send_err)
        };
    }
    if accept_err != 0 {
        // SAFETY: as above.
        unsafe {
            glue::printf(c"ESR error: delivery 0x%x\n".as_ptr(), accept_err)
        };
    }

    // Every defined error-status bit fits a byte, so the conversion is exact.
    (send_err | accept_err) as c_int
}

/// The `smp_send_ipi()` delivery `smp_remote_ast()` asks for.
pub(crate) fn remote_ast(logical_id: c_uint) {
    send_ipi(logical_id, CALL_AST_CHECK);
}

/// The `smp_send_ipi()` delivery `smp_pmap_update()` asks for.
pub(crate) fn pmap_update(logical_id: c_uint) {
    send_ipi(logical_id, CALL_PMAP_UPDATE);
}

/// `smp_startup_cpus()` in C.
pub(crate) fn startup_cpus(bsp_apic_id: c_uint, start_eip: c_ulong) -> c_int {
    // SAFETY: `wbinvd` touches no registers and the stack stays balanced; the
    // memory clobber of the C macro is the default.
    unsafe { asm!("wbinvd", options(nostack, preserves_flags)) };

    // SAFETY: `printf` accepts the C format and arguments.
    unsafe {
        glue::printf(
            c"Sending IPIs from BSP APIC ID %u...\n".as_ptr(),
            bsp_apic_id,
        )
    };

    send_ipi_init(bsp_apic_id);
    // The C passed the shifted address through an `int`; only the vector
    // byte reaches the ICR, and that is what the shift leaves here.
    let vector = (start_eip >> STARTUP_VECTOR_SHIFT) as c_uint;
    let error = send_ipi_startup_twice(bsp_apic_id, vector);
    if error != 0 {
        // SAFETY: `printf` accepts the C format.
        unsafe { glue::printf(c"FATAL: APs failed to start\n".as_ptr()) };
        loop {
            pause();
        }
    }

    // SAFETY: as above.
    unsafe { glue::printf(c"done\n".as_ptr()) };
    0
}

/// `smp_init()` in C.
pub(crate) fn init() -> c_int {
    data_init();
    0
}
