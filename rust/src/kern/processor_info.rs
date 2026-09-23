// SPDX-License-Identifier: CMU-Mach
// Derived from include/mach/processor_info.h:
//   Copyright (c) 1993,1992,1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The MIG information records of `include/mach/processor_info.h`.

use core::ffi::{c_int, c_uint};
use core::mem::offset_of;

/// `PROCESSOR_BASIC_INFO` in <mach/processor_info.h>: the basic
/// information flavor.
pub const PROCESSOR_BASIC_INFO: c_int = 1;
/// `PROCESSOR_BASIC_INFO_COUNT`: the integers that flavor needs.
pub const PROCESSOR_BASIC_INFO_COUNT: c_uint = 5;

/// `PROCESSOR_SET_BASIC_INFO` in <mach/processor_info.h>: the basic
/// information flavor.
pub const PROCESSOR_SET_BASIC_INFO: c_int = 1;
/// `PROCESSOR_SET_BASIC_INFO_COUNT`: the integers that flavor needs.
pub const PROCESSOR_SET_BASIC_INFO_COUNT: c_uint = 5;

/// `PROCESSOR_SET_SCHED_INFO` in <mach/processor_info.h>: the
/// scheduling information flavor.
pub const PROCESSOR_SET_SCHED_INFO: c_int = 2;
/// `PROCESSOR_SET_SCHED_INFO_COUNT`: the integers that flavor needs.
pub const PROCESSOR_SET_SCHED_INFO_COUNT: c_uint = 2;

/// `struct processor_basic_info` of <mach/processor_info.h>: what the
/// `PROCESSOR_BASIC_INFO` flavor reports.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorBasicInfo {
    /// `cpu_type`: the type of cpu.
    pub cpu_type: c_int,
    /// `cpu_subtype`: the subtype of cpu.
    pub cpu_subtype: c_int,
    /// `running`: whether the processor is running.
    pub running: c_int,
    /// `slot_num`: the machine-independent slot number.
    pub slot_num: c_int,
    /// `is_master`: whether this is the master processor.
    pub is_master: c_int,
}

// `struct processor_basic_info`: five `integer_t`s; 20 bytes, which
// `PROCESSOR_BASIC_INFO_COUNT` counts.
const _: () = assert!(size_of::<ProcessorBasicInfo>() == 20);
const _: () = assert!(offset_of!(ProcessorBasicInfo, cpu_type) == 0);
const _: () = assert!(offset_of!(ProcessorBasicInfo, cpu_subtype) == 4);
const _: () = assert!(offset_of!(ProcessorBasicInfo, running) == 8);
const _: () = assert!(offset_of!(ProcessorBasicInfo, slot_num) == 12);
const _: () = assert!(offset_of!(ProcessorBasicInfo, is_master) == 16);

/// `struct processor_set_basic_info` of <mach/processor_info.h>: what
/// the `PROCESSOR_SET_BASIC_INFO` flavor reports.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorSetBasicInfo {
    /// `processor_count`: how many processors the set holds.
    pub processor_count: c_int,
    /// `task_count`: how many tasks are assigned.
    pub task_count: c_int,
    /// `thread_count`: how many threads are assigned.
    pub thread_count: c_int,
    /// `load_average`: the scaled load average.
    pub load_average: c_int,
    /// `mach_factor`: the scaled mach factor.
    pub mach_factor: c_int,
}

// `struct processor_set_basic_info`: five `integer_t`s in the C
// struct's order, `load_average` before `mach_factor`; 20 bytes.
const _: () = assert!(size_of::<ProcessorSetBasicInfo>() == 20);
const _: () = assert!(offset_of!(ProcessorSetBasicInfo, processor_count) == 0);
const _: () = assert!(offset_of!(ProcessorSetBasicInfo, task_count) == 4);
const _: () = assert!(offset_of!(ProcessorSetBasicInfo, thread_count) == 8);
const _: () = assert!(offset_of!(ProcessorSetBasicInfo, load_average) == 12);
const _: () = assert!(offset_of!(ProcessorSetBasicInfo, mach_factor) == 16);

/// `struct processor_set_sched_info` of <mach/processor_info.h>: what
/// the `PROCESSOR_SET_SCHED_INFO` flavor reports.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorSetSchedInfo {
    /// `policies`: the allowed policies.
    pub policies: c_int,
    /// `max_priority`: the maximum priority for new threads.
    pub max_priority: c_int,
}

// `struct processor_set_sched_info`: two `integer_t`s; 8 bytes, which
// `PROCESSOR_SET_SCHED_INFO_COUNT` counts.
const _: () = assert!(size_of::<ProcessorSetSchedInfo>() == 8);
const _: () = assert!(offset_of!(ProcessorSetSchedInfo, policies) == 0);
const _: () = assert!(offset_of!(ProcessorSetSchedInfo, max_priority) == 4);
