// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/com.c and i386/i386at/comreg.h:
//   Copyright (c) 1994,1993,1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386at/com.c`, which the i386 device
//! switch, the console table, the bus configuration and the Rust mouse driver
//! call.
//!
//! Every adapter here hands its raw arguments to the matching core in
//! [`com`] and converts the answer back.

use crate::arch::i386::com::{
    self, BusCtlr, BusDevice, TTY_CLEAR_BREAK, TTY_MODEM, TTY_SET_BREAK,
    TTY_STATUS,
};
use crate::arch::i386::io_req::{DevT, IoReq};
use crate::arch::i386::kd::ConsDev;
use crate::arch::types::VmOffset;
use crate::device::chario::{DMBIC, DMBIS, DMSET, TM_BRK, Tty};
use crate::device::chario_ffi;
use crate::device::r#return::DeviceError;
use core::ffi::{c_int, c_uint, c_void};
use core::ptr;

/// `comprobe()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `dev` must point at a live `bus_ctlr` table entry; the bus configuration
/// calls it that way.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comprobe(
    _port: VmOffset,
    dev: *mut BusCtlr,
) -> c_int {
    // SAFETY: the caller promises a live entry, and `comprobe_general()`
    // reads `unit` and `address`, which `bus_ctlr` and `bus_device` place at
    // the same offsets.
    let (unit, address) = unsafe {
        (
            ptr::addr_of!((*dev).unit).read(),
            ptr::addr_of!((*dev).address).read(),
        )
    };
    c_int::from(com::probe_general(address, unit, false))
}

/// `comcnprobe()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `cp` must point at the console table's entry, writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comcnprobe(cp: *mut ConsDev) -> c_int {
    // SAFETY: the caller promises the entry.
    com::cnprobe(unsafe { &mut *cp })
}

/// `comattach()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `dev` must point at a live `bus_device` table entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comattach(dev: *mut BusDevice) {
    // SAFETY: the caller promises the live entry.
    com::attach(unsafe { &*dev });
}

/// `comcninit()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `cp` must point at the console table's entry, initialized by
/// `comcnprobe()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comcninit(cp: *mut ConsDev) -> c_int {
    // SAFETY: the caller promises the entry.
    com::cninit(unsafe { &*cp })
}

/// `comopen()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `ior` must point at a live open request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comopen(
    dev: DevT,
    flag: c_int,
    ior: *mut IoReq,
) -> c_int {
    // SAFETY: the device layer passes a live request.
    com::open(c_int::from(dev), flag, unsafe { &mut *ior })
}

/// `comclose()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// The device layer calls this for an open unit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comclose(dev: DevT, _flag: c_int) {
    com::close(c_int::from(dev));
}

/// `comread()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `ior` must point at a live read request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comread(dev: DevT, ior: *mut IoReq) -> c_int {
    // SAFETY: the device layer passes a live request.
    com::read(c_int::from(dev), unsafe { &mut *ior })
}

/// `comwrite()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `ior` must point at a live write request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comwrite(dev: DevT, ior: *mut IoReq) -> c_int {
    // SAFETY: the device layer passes a live request.
    com::write(c_int::from(dev), unsafe { &mut *ior })
}

/// `comportdeath()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `port` is the reply port that died, as the device layer received it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comportdeath(dev: DevT, port: VmOffset) -> c_int {
    com::port_death(c_int::from(dev), ptr::with_exposed_provenance_mut(port))
}

/// `comgetstat()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `data` must be writable for `*count` integers and `count` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comgetstat(
    dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut u32,
) -> c_int {
    let unit = c_int::from(dev) & 0xff;
    if flavor == TTY_MODEM {
        let status = com::modem_status(unit);
        // SAFETY: the caller promises one writable integer and a writable
        // count, which the C wrote unconditionally.
        unsafe {
            *data = status;
            *count = 1;
        }
        return 0;
    }
    let Some(tp) = com::tty_mut(unit) else {
        return DeviceError::NoSuchDevice as c_int;
    };
    // SAFETY: the caller promises the live tty and the status operands.
    unsafe {
        chario_ffi::tty_get_status(ptr::from_mut(tp), flavor, data, count)
    }
}

/// `comsetstat()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `data` must be readable for `count` integers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comsetstat(
    dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: u32,
) -> c_int {
    let unit = c_int::from(dev) & 0xff;
    let Some(tp) = com::tty_mut(unit) else {
        return DeviceError::NoSuchDevice as c_int;
    };
    match flavor {
        TTY_SET_BREAK => {
            com::modem_ctl(tp, TM_BRK, DMBIS);
            0
        }
        TTY_CLEAR_BREAK => {
            com::modem_ctl(tp, TM_BRK, DMBIC);
            0
        }
        TTY_MODEM => {
            // SAFETY: the caller promises one readable integer.
            let bits = unsafe { *data };
            com::modem_ctl(tp, bits, DMSET);
            0
        }
        _ => {
            // SAFETY: the caller promises the live tty and the status
            // operands.
            let result = unsafe {
                chario_ffi::tty_set_status(
                    ptr::from_mut(tp),
                    flavor,
                    data,
                    count,
                )
            };
            if result == 0 && flavor == TTY_STATUS {
                com::apply_params(tp, unit);
            }
            result
        }
    }
}

/// `comintr()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// The interrupt vector calls this with the unit the device was attached at.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comintr(unit: c_int) {
    com::intr(unit);
}

/// `comstart()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `tp` must point at a live tty whose lock the caller holds, as the tty
/// layer's start contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comstart(tp: *mut Tty) {
    // SAFETY: the caller promises the live, locked tty.
    com::start(unsafe { &mut *tp });
}

/// `comtimer()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `timeout()` calls this with the timeout pool element; the parameter is
/// unused, as the C left it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comtimer(_param: *mut c_void) {
    com::timer();
}

/// `fix_modem_state()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `unit` must name a configured serial unit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fix_modem_state(unit: c_int, modem_stat: c_int) {
    com::fix_modem_state(unit, modem_stat);
}

/// `commodem_intr()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// The interrupt path calls this with the modem status register.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn commodem_intr(unit: c_int, stat: c_int) {
    com::modem_intr(unit, stat);
}

/// `commctl()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `tp` must point at a live tty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn commctl(
    tp: *mut Tty,
    bits: c_int,
    how: c_int,
) -> c_int {
    // SAFETY: the caller promises the live tty.
    com::modem_ctl(unsafe { &mut *tp }, bits, how)
}

/// `comstop()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `tp` must point at a live tty whose lock the caller holds, as the tty
/// layer's stop contract requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comstop(tp: *mut Tty, _flags: c_int) {
    // SAFETY: the caller promises the live, locked tty.
    com::stop(unsafe { &mut *tp });
}

/// `compr_addr()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// Called from the debugger with the port to dump; the reads are to the
/// caller's hardware.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compr_addr(addr: VmOffset) {
    com::print_regs(addr);
}

/// `compr()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// Called from the debugger with the unit to dump.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compr(unit: c_int) -> c_int {
    com::print_unit(unit)
}

/// `comgetc()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// `unit` must name a configured serial unit with an attached device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comgetc(unit: c_int) -> c_int {
    com::getc(unit)
}

/// `comcnputc()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// The console layer calls this for its own console unit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comcnputc(dev: DevT, c: c_int) -> c_int {
    com::console_putc(c_int::from(dev), c)
}

/// `comcngetc()` of `i386/i386at/com.c`.
///
/// # Safety
///
/// The console layer calls this for its own console unit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn comcngetc(dev: DevT, wait: c_int) -> c_int {
    com::console_getc(c_int::from(dev), wait != 0)
}
