// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/ast_check.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Remote AST delivery, which `i386/i386/ast_check.c` used to define
//! and `kern/ast.h` declares.
//!
//! On i386, an AST is signalled to another processor with a local APIC
//! IPI.  `init_ast_check()` has nothing to initialize, and
//! `cause_ast_check()` translates the target's machine slot number
//! into the logical destination bit the IPI names and hands it to
//! `smp_remote_ast()`.

use crate::glue;
use crate::kern::processor::Processor;

/// `APIC_LOGICAL_CPU_GROUPS` in <i386/apic.h>: the logical destination
/// register has only eight mask bits, so it can name eight CPU groups.
const APIC_LOGICAL_CPU_GROUPS: u32 = 8;

/// Initialize for remote invocation of `ast_check`.  The body of
/// `init_ast_check()` in i386/i386/ast_check.c.
///
/// The i386 implementation does nothing, exactly as the C does, so it
/// carries no caller contract.
#[unsafe(no_mangle)]
pub extern "C" fn init_ast_check(_processor: *mut Processor) {}

/// Cause remote invocation of `ast_check`.  The body of
/// `cause_ast_check()` in i386/i386/ast_check.c.  The caller is at
/// splsched().
///
/// # Safety
///
/// `processor` must point at a live `struct processor`, as the C
/// dereferenced it to read `slot_num`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cause_ast_check(processor: *mut Processor) {
    // SAFETY: the caller promises a live processor; the C read the
    // same `slot_num` integer field here.
    let slot_num = unsafe { (*processor).slot_num };
    // `APIC_LOGICAL_ID(cpu)` in <i386/apic.h> is
    // `(1u << ((cpu) % APIC_LOGICAL_CPU_GROUPS))`.  The cast takes the
    // modulus in the unsigned domain as the macro's `1u` shift does:
    // `processor_init(processor_ptr(i), i)` seeds `slot_num` from the
    // small CPU index `i < NCPUS`, so it is non-negative and no bit can
    // be lost.
    let group = (slot_num as u32) % APIC_LOGICAL_CPU_GROUPS;
    let logical_id = 1u32 << group;
    // SAFETY: `smp_remote_ast()` is the real C IPI routine, and
    // `logical_id` is the APIC destination bit the C macro computes.
    unsafe { glue::smp_remote_ast(logical_id) };
}
