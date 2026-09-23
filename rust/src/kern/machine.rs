// SPDX-License-Identifier: CMU-Mach
// Derived from include/mach/machine.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `struct machine_slot` of `include/mach/machine.h`.
//!
//! The C keeps `struct machine_slot machine_slot[NCPUS]`, and `NCPUS`
//! is a configure-time constant Rust cannot name; the glue declares
//! the first element and strides it, as `src/kern/ast.rs` does for
//! `need_ast`.  The arch probe fills the records.

use core::ffi::c_int;
use core::mem::offset_of;

/// `CPU_STATE_MAX` in <mach/machine.h>: the per-state tick counters
/// every machine slot carries.
pub const CPU_STATE_MAX: usize = 3;

/// `struct machine_slot` of <mach/machine.h>: what the arch probe
/// records about each possible CPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSlot {
    /// `is_cpu`: whether there is a cpu in this slot.
    pub is_cpu: c_int,
    /// `cpu_type`: the type of the cpu.
    pub cpu_type: c_int,
    /// `cpu_subtype`: the subtype of the cpu.
    pub cpu_subtype: c_int,
    /// `running`: whether the cpu is running.
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
