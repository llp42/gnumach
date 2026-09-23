// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/machine.h:
//   Copyright (C) 2008 Free Software Foundation, Inc.
// Derived from kern/machine.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from include/mach/machine.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct machine_slot` of `include/mach/machine.h`, and the `host_reboot`
//! and `action_thread` of `kern/machine.c`.

use crate::glue;
use crate::kern::debug;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr::{self, NonNull};

/// `CPU_STATE_MAX` in <mach/machine.h>: the per-state tick counters every
/// machine slot carries.
pub const CPU_STATE_MAX: usize = 3;

/// `struct machine_slot` of <mach/machine.h>: what the arch probe records
/// about each possible CPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSlot {
    pub is_cpu: c_int,
    pub cpu_type: c_int,
    pub cpu_subtype: c_int,
    pub running: c_int,
    /// `cpu_ticks`: the ticks accumulated per `CPU_STATE_*`.
    pub cpu_ticks: [c_int; CPU_STATE_MAX],
    /// `clock_freq`: the clock interrupt frequency.
    pub clock_freq: c_int,
}

const _: () = assert!(size_of::<MachineSlot>() == 32);
const _: () = assert!(align_of::<MachineSlot>() == align_of::<c_int>());
const _: () = assert!(offset_of!(MachineSlot, is_cpu) == 0);
const _: () = assert!(offset_of!(MachineSlot, cpu_type) == 4);
const _: () = assert!(offset_of!(MachineSlot, cpu_subtype) == 8);
const _: () = assert!(offset_of!(MachineSlot, running) == 12);
const _: () = assert!(offset_of!(MachineSlot, cpu_ticks) == 16);
const _: () = assert!(offset_of!(MachineSlot, clock_freq) == 28);

/// The C `machine_slot[cpu]` of <mach/machine.h>.
///
/// # Safety
///
/// `cpu` must be below the configured `NCPUS`, the C array's length.
pub(crate) unsafe fn slot(cpu: usize) -> *mut MachineSlot {
    // SAFETY: the caller promises `cpu < NCPUS`; `.cast()` keeps the element
    // pointer and `.add()` stays inside the array.
    unsafe {
        ptr::addr_of_mut!(glue::machine_slot)
            .cast::<MachineSlot>()
            .add(cpu)
    }
}

/// The `RB_*` flag word of <sys/reboot.h>, the `host_reboot()` options.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RebootOptions(c_int);

impl RebootOptions {
    /// `RB_DEBUGGER`: enter the kernel debugger from user level instead of
    /// rebooting.
    const DEBUGGER: Self = Self(0x1000);
    /// `RB_HALT`: do not reboot, just halt.
    const HALT: Self = Self(0x08);

    /// Whether every bit of `other` is set in `self`.
    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Reboot or halt the host, or enter the debugger.
fn reboot(
    host: Option<NonNull<c_void>>,
    options: RebootOptions,
) -> Result<(), KernError> {
    if host.is_none() {
        return Err(KernError::InvalidHost);
    }

    if options.contains(RebootOptions::DEBUGGER) {
        // SAFETY: `Debugger` takes a readable NUL-terminated message; this one
        // is a literal, and the call never returns.
        unsafe { debug::Debugger(c"Debugger".as_ptr()) };
    } else {
        // SAFETY: `halt_all_cpus` never returns.
        let reboot = c_int::from(!options.contains(RebootOptions::HALT));
        unsafe { glue::halt_all_cpus(reboot) };
    }

    Ok(())
}

/// `host_reboot()` of kern/machine.c, the routine <mach/mach_host.defs>
/// declares.
///
/// # Safety
///
/// `host_priv` must be `HOST_NULL` or the live host privilege pointer the MIG
/// stub converted the request port into.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_reboot(
    host_priv: *mut c_void,
    options: c_int,
) -> c_int {
    match reboot(NonNull::new(host_priv), RebootOptions(options)) {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `action_thread()` of kern/machine.c, declared in <kern/machine.h>.
///
/// # Safety
///
/// `kern/startup.c` is the only caller; it starts this during boot with
/// `kernel_thread()` and nothing locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn action_thread() -> ! {
    // SAFETY: `action_thread_continue()` drains the action queue in a loop
    // whose wait re-enters the routine itself, so it does not return.
    unsafe { glue::action_thread_continue() }
}
