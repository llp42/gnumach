// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ast.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/ast.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The per-CPU AST bits, which `kern/ast.h` declares as macros over
//! `need_ast[]` and `kern/ast.c` defines.

use crate::kern::smp::smp_get_numcpus;
use core::ffi::c_int;

/// `AST_BLOCK` in <kern/ast.h>: the scheduling AST reason.
pub const AST_BLOCK: usize = 0x4;

unsafe extern "C" {
    static mut need_ast: usize;
}

/// The `ast_needed()` macro of <kern/ast.h>: the reasons pending on `cpu`.
pub fn ast_needed(cpu: c_int) -> usize {
    // SAFETY: `slot()` requires a live CPU number, as its caller must give it;
    // a read of the slot cannot invalidate anything.
    unsafe { slot(cpu).read_volatile() }
}

/// The `ast_on()` macro of <kern/ast.h>: set `reasons` on `cpu`.
pub fn ast_on(cpu: c_int, reasons: usize) {
    // SAFETY: as `ast_needed()`; the read-modify-write keeps the other reasons
    // of the same slot.
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
/// `cpu` must be a CPU number of the machine, below [`smp_get_numcpus()`].
unsafe fn slot(cpu: c_int) -> *mut usize {
    debug_assert!(
        0 <= cpu && cpu < c_int::from(smp_get_numcpus()),
        "AST cpu {cpu} outside the {} probed CPUs",
        smp_get_numcpus(),
    );
    // SAFETY: the caller promises a live CPU number; `cpu` is a non-negative
    // number, and widening it to pointer width is exact.
    unsafe { (&raw mut need_ast).offset(cpu as isize) }
}
