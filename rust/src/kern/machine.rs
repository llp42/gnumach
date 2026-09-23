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

//! `struct machine_slot` of `include/mach/machine.h`, and the
//! `host_reboot` and `action_thread` of `kern/machine.c`.
//!
//! The C keeps `struct machine_slot machine_slot[NCPUS]`, and `NCPUS`
//! is a configure-time constant Rust cannot name; the glue declares
//! the first element and strides it, as `src/kern/ast.rs` does for
//! `need_ast`.  The arch probe fills the records.
//!
//! `reboot` is the body `host_reboot()` used to hold, and the
//! `host_reboot` adapter below keeps the MIG prototype of
//! <mach/mach_host.defs>.  `action_thread()` is the boot-time thread
//! that drains the action queue; the rest of `kern/machine.c` stays C.

use crate::glue;
use crate::kern::debug;
use crate::kern::types::KernError;
use core::ffi::{c_int, c_void};
use core::mem::offset_of;
use core::ptr::NonNull;

/// `CPU_STATE_MAX` in <mach/machine.h>: the per-state tick counters
/// every machine slot carries.
pub const CPU_STATE_MAX: usize = 3;

/// `struct machine_slot` of <mach/machine.h>: what the arch probe
/// records about each possible CPU.
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

// `struct machine_slot`: six `integer_t`s, with the three tick
// counters between `running` and `clock_freq`; the C compiler's size
// is 32 and its alignment 4.
const _: () = assert!(size_of::<MachineSlot>() == 32);
const _: () = assert!(align_of::<MachineSlot>() == align_of::<c_int>());
const _: () = assert!(offset_of!(MachineSlot, is_cpu) == 0);
const _: () = assert!(offset_of!(MachineSlot, cpu_type) == 4);
const _: () = assert!(offset_of!(MachineSlot, cpu_subtype) == 8);
const _: () = assert!(offset_of!(MachineSlot, running) == 12);
const _: () = assert!(offset_of!(MachineSlot, cpu_ticks) == 16);
const _: () = assert!(offset_of!(MachineSlot, clock_freq) == 28);

/// The `RB_*` flag word of <sys/reboot.h>, the `host_reboot()` options.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RebootOptions(c_int);

impl RebootOptions {
    /// `RB_DEBUGGER`: enter the kernel debugger from user level
    /// instead of rebooting.
    const DEBUGGER: Self = Self(0x1000);
    /// `RB_HALT`: do not reboot, just halt.
    const HALT: Self = Self(0x08);

    /// Whether every bit of `other` is set in `self`.
    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Reboot or halt the host, or enter the debugger.  The body of
/// `host_reboot()` in kern/machine.c.
///
/// `None` is the C `HOST_NULL`.  `Debugger` and `halt_all_cpus` never
/// return, so a call that gets past the host check does not come back.
fn reboot(
    host: Option<NonNull<c_void>>,
    options: RebootOptions,
) -> Result<(), KernError> {
    if host.is_none() {
        return Err(KernError::InvalidHost);
    }

    if options.contains(RebootOptions::DEBUGGER) {
        // SAFETY: `Debugger` takes a readable NUL-terminated message;
        // this one is a literal, and the call never returns.
        unsafe { debug::Debugger(c"Debugger".as_ptr()) };
    } else {
        // SAFETY: `halt_all_cpus` never returns.  The argument is the
        // C's `!(options & RB_HALT)`, one when the halt bit is clear.
        let reboot = c_int::from(!options.contains(RebootOptions::HALT));
        unsafe { glue::halt_all_cpus(reboot) };
    }

    Ok(())
}

/// Reboot the host.  `host_reboot()` of kern/machine.c, the routine
/// <mach/mach_host.defs> declares.
///
/// # Safety
///
/// `host_priv` must be `HOST_NULL` or the live host privilege pointer
/// the MIG stub converted the request port into.  `options` is the
/// user-supplied flag word: `RB_DEBUGGER` panics into the debugger,
/// `RB_HALT` halts instead of rebooting, and every other bit is
/// ignored.  Both paths diverge, so the caller must accept that the
/// machine stops.
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

/// Start the processor action thread.  `action_thread()` of
/// kern/machine.c, declared in <kern/machine.h>.
///
/// The C marks the routine `noreturn`, and so does this: the thread it
/// runs on blocks on the action queue and only resumes inside
/// `action_thread_continue()` itself.
///
/// # Safety
///
/// `kern/startup.c` is the only caller; it starts this during boot with
/// `kernel_thread()` and nothing locked.  The call never returns to its
/// caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn action_thread() -> ! {
    // SAFETY: `action_thread_continue()` drains the action queue in a
    // loop whose wait re-enters the routine itself, so it does not
    // return.
    unsafe { glue::action_thread_continue() }
}
