// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls.
//!
//! C *macros* cannot come through here; when Rust needs one, it gets a
//! small C shim function beside the header that defines it, and that
//! shim is declared below like any other C function.

use crate::arch::types::VmOffset;
use core::ffi::{c_char, c_int, c_short, c_uint, c_void};

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

    // <kern/mach_clock.h>
    pub fn timeout(
        fcn: Option<unsafe extern "C" fn(*mut c_void)>,
        param: *mut c_void,
        interval: c_int,
    ) -> *mut c_void;

    // <kern/machine.c>
    pub fn cpu_shutdown();

    // <util/atoi.h>
    pub fn mach_atoi(s: *const u8, nump: *mut c_int) -> c_int;

    // <i386at/kd.h>, the screen block moves in kdasm.S
    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

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
    pub fn splsoftclock() -> c_int;
    pub fn splx(level: c_int) -> c_int;

    // <i386at/com.h>
    pub fn comgetc(unit: c_int) -> c_int;

    // <device/tty.h> and <device/cirbuf.h>
    pub fn ttychars(tp: *mut c_void);
    pub fn char_open(
        dev: c_int,
        tp: *mut c_void,
        mode: c_int,
        ior: *mut c_void,
    ) -> c_int;
    pub fn ttyclose(tp: *mut c_void);
    pub fn tty_get_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: *mut u32,
    ) -> c_int;
    pub fn tty_set_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: u32,
    ) -> c_int;
    pub fn tty_portdeath(tp: *mut c_void, port: *mut c_void) -> c_int;
    pub fn tty_queue_completion(queue: *mut c_void);
    pub fn getc(buf: *mut c_void) -> c_int;

    // Shims in i386/i386at/kd_glue.c, for the tty lock macros, the
    // line discipline switch, `ttlowat[]` and `phystokv()`.
    pub fn kd_simple_lock_irq(lock: *mut c_void) -> c_int;
    pub fn kd_simple_unlock_irq(s: c_int, lock: *mut c_void);
    pub fn kd_simple_lock(lock: *mut c_void);
    pub fn kd_simple_unlock(lock: *mut c_void);
    pub fn kd_ldisc_read(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_write(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_rint(line: c_int, c: c_uint, tp: *mut c_void);
    pub fn kd_ttlowat(speed: c_int) -> c_short;

    // <kern/mach_clock.h> and <i386/i386at/model_dep.c>
    pub static hz: c_int;
    pub static rebootflag: c_int;

    // Shims for the C macros Rust cannot call: see i386/i386/pio_glue.c.
    pub fn pio_inb(port: u16) -> u8;
    pub fn pio_inw(port: u16) -> u16;
    pub fn pio_inl(port: u16) -> u32;
    pub fn pio_outb(port: u16, value: u8);
    pub fn pio_outw(port: u16, value: u16);
    pub fn pio_outl(port: u16, value: u32);

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
