// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ast.h and kern/ast.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-CPU AST bits, which `kern/ast.h` declares as macros over
//! `need_ast[]` and `kern/ast.c` defines.
//!
//! Only the macro accessors move here: `ast_on()`, `ast_off()` and
//! `ast_needed()`.  `ast_init()`, `ast_taken()` and `ast_check()` stay
//! C and keep reading the same array through the same macros, so the
//! array stays C and Rust reaches it through its symbol.

use crate::kern::smp::smp_get_numcpus;
use core::ffi::c_int;

/// `AST_BLOCK` in <kern/ast.h>: the scheduling AST reason.
pub const AST_BLOCK: usize = 0x4;

// `need_ast` in <kern/ast.h>: the pending AST reasons, one per CPU.
//
// The C declares `volatile ast_t need_ast[NCPUS]`; the mirror names the
// first element because `NCPUS` is a C constant.  `ast_t` is
// `unsigned long`, which `usize` matches on both kernels, so the
// pointer arithmetic below strides one element at a time.
unsafe extern "C" {
    static mut need_ast: usize;
}

/// The `ast_needed()` macro of <kern/ast.h>: the reasons pending on
/// `cpu`.
pub fn ast_needed(cpu: c_int) -> usize {
    // SAFETY: `slot()` requires a live CPU number, as its caller must
    // give it; a read of the slot cannot invalidate anything.
    unsafe { slot(cpu).read_volatile() }
}

/// The `ast_on()` macro of <kern/ast.h>: set `reasons` on `cpu`.
pub fn ast_on(cpu: c_int, reasons: usize) {
    // SAFETY: as `ast_needed()`; the read-modify-write keeps the other
    // reasons of the same slot.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() | reasons);
    }
}

/// The `ast_off()` macro of <kern/ast.h>: clear `reasons` on `cpu`.
pub fn ast_off(cpu: c_int, reasons: usize) {
    // SAFETY: as `ast_needed()`.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() & !reasons);
    }
}

/// The address of `need_ast[cpu]`.
///
/// # Safety
///
/// `cpu` must be a CPU number of the machine, below
/// [`smp_get_numcpus()`].
unsafe fn slot(cpu: c_int) -> *mut usize {
    // The machine numbers its CPUs from zero, so only the upper bound
    // can be broken; the probe's count is the `NCPUS` the C array was
    // sized with.
    debug_assert!(
        0 <= cpu && cpu < c_int::from(smp_get_numcpus()),
        "AST cpu {cpu} outside the {} probed CPUs",
        smp_get_numcpus(),
    );
    // SAFETY: the caller promises a live CPU number; `cpu` is a
    // non-negative number, and widening it to pointer width is exact.
    unsafe { (&raw mut need_ast).offset(cpu as isize) }
}
