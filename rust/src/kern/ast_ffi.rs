// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ast.c and kern/ast.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C entry points of `kern/ast.c`, which the file used to define for
//! <kern/ast.h>.

use crate::kern::ast;

/// `ast_init()` of kern/ast.c.
///
/// # Safety
///
/// `kern/sched_prim.c` is the only caller; it runs this during boot, before
/// any CPU can take an AST.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ast_init() {
    // SAFETY: the caller's contract.
    unsafe { ast::init() };
}

/// `ast_taken()` of kern/ast.c: act on the ASTs pending on the running CPU.
///
/// # Safety
///
/// Machine-dependent code calls this on return to user mode with interrupts
/// disabled; the CPU must have no AST action in progress.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ast_taken() {
    // SAFETY: the caller's contract.
    unsafe { ast::taken() };
}

/// `ast_check()` of kern/ast.c: check the running processor for AST
/// conditions at `splsched`.
///
/// # Safety
///
/// The caller must be the running thread's CPU, able to take `splsched`, and
/// must not hold the run-queue lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ast_check() {
    // SAFETY: the caller's contract.
    unsafe { ast::check() };
}
