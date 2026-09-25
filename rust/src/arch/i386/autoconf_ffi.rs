// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/autoconf.c and i386/i386at/autoconf.h:
//   Copyright (c) 1993,1992,1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386at/autoconf.c`, one adapter per
//! symbol `i386/i386at/autoconf.h` declares.

use crate::arch::i386::autoconf;
use crate::arch::i386::com::BusDevice;

/// `probeio()` of <i386at/autoconf.h>: probe and attach the AT-bus devices.
#[unsafe(no_mangle)]
pub extern "C" fn probeio() {
    autoconf::probeio();
}

/// `take_dev_irq()` of <i386at/autoconf.h>.
///
/// # Safety
///
/// `dev` must point at a live [`BusDevice`], as the driver tables and
/// `configure_bus_device()` pass.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn take_dev_irq(dev: *const BusDevice) {
    // SAFETY: the caller promises the live device.
    autoconf::take_dev_irq(unsafe { &*dev });
}
