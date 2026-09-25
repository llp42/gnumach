// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/ast_check.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Remote AST delivery, which `i386/i386/ast_check.c` used to define and
//! `kern/ast.h` declares.

use crate::arch::i386::smp;
use crate::kern::processor::Processor;

/// `APIC_LOGICAL_CPU_GROUPS` in <i386/apic.h>: the logical destination
/// register has only eight mask bits, so it can name eight CPU groups.
const APIC_LOGICAL_CPU_GROUPS: u32 = 8;

/// Initialize for remote invocation of `ast_check`.
#[unsafe(no_mangle)]
pub extern "C" fn init_ast_check(_processor: *mut Processor) {}

/// Cause remote invocation of `ast_check`.
///
/// # Safety
///
/// `processor` must point at a live `struct processor`, as the C dereferenced
/// it to read `slot_num`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cause_ast_check(processor: *mut Processor) {
    // SAFETY: the caller promises a live processor; the C read the same
    // `slot_num` integer field here.
    let slot_num = unsafe { (*processor).slot_num };
    // `APIC_LOGICAL_ID(cpu)` in <i386/apic.h> is `(1u << ((cpu) %
    // APIC_LOGICAL_CPU_GROUPS))`.
    let group = (slot_num as u32) % APIC_LOGICAL_CPU_GROUPS;
    let logical_id = 1u32 << group;
    // The local APIC is initialized before any processor takes an AST, and
    // `logical_id` is the APIC destination bit the C macro computes.
    smp::remote_ast(logical_id);
}
