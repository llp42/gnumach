// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/machine.h:
//   Copyright (C) 2008 Free Software Foundation, Inc.
// Derived from kern/machine.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entries `kern/machine.c` used to define.

use crate::kern::machine;
use crate::kern::processor::{Processor, ProcessorSet};
use core::ffi::{c_int, c_void};

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
    // SAFETY: the caller's contract.
    match unsafe { machine::host_reboot(host_priv, options) } {
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
    unsafe { machine::action_thread_continue() }
}

/// `action_thread_continue()` of kern/machine.c, declared in <kern/machine.h>.
///
/// # Safety
///
/// Only the action thread may run this, and `action_queue` may have no other
/// drainer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn action_thread_continue() -> ! {
    // SAFETY: the caller's contract.
    unsafe { machine::action_thread_continue() }
}

/// `cpu_up()` of kern/machine.c: flag `cpu` as up and running.
///
/// # Safety
///
/// `cpu` must be a CPU the machine reports, and the boot or the hot-plug path
/// must call this with nothing locked on the processor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpu_up(cpu: c_int) {
    // SAFETY: the caller's contract.
    unsafe { machine::cpu_up(cpu) };
}

/// `processor_assign()` of kern/machine.c.
///
/// # Safety
///
/// `processor` must be null or a live processor and `new_pset` null or a live
/// set; the caller must hold no lock, because the routine waits and may block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_assign(
    processor: *mut Processor,
    new_pset: *mut ProcessorSet,
    wait: c_int,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { machine::assign(processor, new_pset, wait != 0) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_shutdown()` of kern/machine.c.
///
/// # Safety
///
/// `processor` must be null or a live processor; the routine takes the
/// processor lock itself and may be called from interrupt level.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_shutdown(
    processor: *mut Processor,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { machine::shutdown(processor) } {
        Ok(()) => 0,
        Err(error) => c_int::from(error),
    }
}

/// `processor_doshutdown()` of kern/machine.c, the routine
/// `switch_to_shutdown_context()` runs on the dying CPU.
///
/// # Safety
///
/// The shutdown context of the live `processor` must call this, and it never
/// returns to the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn processor_doshutdown(processor: *mut Processor) {
    // SAFETY: the caller's contract.
    unsafe { machine::processor_doshutdown(processor) };
}
