// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls.
//!
//! C *macros* cannot come through here; when Rust needs one, it gets a
//! small C shim function beside the header that defines it, and that
//! shim is declared below like any other C function.

use crate::arch::types::VmOffset;
use core::ffi::{c_char, c_int, c_uint, c_void};

// `panic()` in <kern/debug.h> is a macro over `Panic()`.
unsafe extern "C" {
    pub fn Panic(
        file: *const c_char,
        line: c_int,
        fun: *const c_char,
        s: *const c_char,
        ...
    ) -> !;

    // <kern/printf.h>
    pub fn printf(fmt: *const c_char, ...) -> c_int;

    // <kern/sched_prim.h>
    pub fn wakeup(channel: VmOffset);
    pub fn assert_wait(event: *mut c_void, interruptible: c_int);
    pub fn thread_block(continuation: Option<unsafe extern "C" fn()>);

    // <device/ds_routines.h>, the request passed as an opaque handle:
    // `struct io_req` itself belongs to its driver.
    pub fn iodone(ior: *mut c_void);
    pub fn device_read_alloc(ior: *mut c_void, size: usize) -> c_int;
    pub fn ds_read_done(ior: *mut c_void) -> c_int;

    // <machine/spl.h>: asm functions, `SPLKD` is a macro over `spltty`.
    pub fn splhi() -> c_int;
    pub fn spltty() -> c_int;
    pub fn splx(level: c_int) -> c_int;

    // <i386at/com.h>
    pub fn comgetc(unit: c_int) -> c_int;

    // <i386at/kd.h>
    pub fn kd_sendcmd(ch: u8);
    pub fn kd_cmdreg_write(value: c_int);
    pub fn kd_mouse_drain();
    pub fn kdintr(vector: c_int);

    // Shims for the C macros Rust cannot call: see i386/i386/pio_glue.c.
    pub fn pio_inb(port: u16) -> u8;
    pub fn pio_outb(port: u16, value: u8);

    // Shims for `mask_irq'/'unmask_irq' (static inline under APIC) and
    // for the NINTR-sized `ivect'/`iunit' arrays: see i386/i386/irq.c.
    pub fn irq_mask(irq: c_uint);
    pub fn irq_unmask(irq: c_uint);
    pub fn irq_set_handler(
        irq: c_int,
        handler: Option<unsafe extern "C" fn(c_int)>,
    );
    pub fn irq_get_handler(irq: c_int) -> Option<unsafe extern "C" fn(c_int)>;
    pub fn irq_set_unit(irq: c_int, unit: c_int);
    pub fn irq_get_unit(irq: c_int) -> c_int;

    // Shims for the NCOM-sized `cominfo' array: see i386/i386at/com.c.
    pub fn com_base_addr(unit: c_int) -> VmOffset;
    pub fn com_irq(unit: c_int) -> c_int;
}
