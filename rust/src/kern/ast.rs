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

use crate::config::NCPUS;
use crate::kern::smp::smp_get_numcpus;
use crate::kern::thread::Thread;
use core::ffi::c_int;

/// `AST_HALT` in <kern/ast.h>: the thread has been asked to halt at a clean
/// point.
pub const AST_HALT: c_int = 0x1;
/// `AST_TERMINATE` in <kern/ast.h>: the thread is terminating.
pub const AST_TERMINATE: c_int = 0x2;
/// `AST_BLOCK` in <kern/ast.h>: the scheduling AST reason.
pub const AST_BLOCK: usize = 0x4;
/// `AST_NETWORK` in <kern/ast.h>: the network thread has packets to deliver.
pub const AST_NETWORK: usize = 0x8;
/// `AST_SCHEDULING` in <kern/ast.h>: the reasons the scheduler holds back
/// while the idle loop waits.
pub const AST_SCHEDULING: usize =
    (AST_HALT | AST_TERMINATE) as usize | AST_BLOCK;

/// `AST_PER_THREAD` in <kern/ast.h>: the reasons reset from the thread at a
/// context switch.
const AST_PER_THREAD: usize = (AST_HALT | AST_TERMINATE) as usize;

unsafe extern "C" {
    static mut need_ast: [usize; NCPUS];
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

/// Whether CPU `cpu` has an AST other than the scheduling reasons, which the
/// idle loop handles itself.
pub fn ast_scheduling_pending(cpu: c_int) -> bool {
    ast_needed(cpu) & !AST_SCHEDULING != 0
}

/// The `need_ast[mycpu] &= ~AST_SCHEDULING` of kern/sched_prim.c.
pub fn ast_clear_scheduling(cpu: c_int) {
    // SAFETY: as `ast_needed()`.
    unsafe {
        let cell = slot(cpu);
        cell.write_volatile(cell.read_volatile() & !AST_SCHEDULING);
    }
}

/// The `ast_context()` macro of <kern/ast.h>: replace the per-thread reasons
/// of `cpu` with `thread`'s pending ones.
///
/// # Safety
///
/// `thread` must be a live thread, and `cpu` a CPU number below
/// [`smp_get_numcpus()`].
pub unsafe fn ast_context(thread: *mut Thread, cpu: c_int) {
    // SAFETY: the caller promises a live thread and CPU, and `slot()` serves
    // the same `need_ast` array the C macro touched.
    unsafe {
        let cell = slot(cpu);
        let per_thread = cell.read_volatile() & !AST_PER_THREAD;
        cell.write_volatile(per_thread | (*thread).ast as usize);
    }
}

/// `ast_init()` of kern/ast.c.
///
/// # Safety
///
/// `kern/sched_prim.c` is the only caller; it runs this during boot, before any
/// CPU can take an AST.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ast_init() {
    for cpu in 0..NCPUS {
        // SAFETY: `cpu` indexes the `NCPUS` C slots, and no other thread can
        // set an AST before the boot reaches the scheduler.
        unsafe {
            (&raw mut need_ast)
                .cast::<usize>()
                .add(cpu)
                .write_volatile(0)
        };
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
    // SAFETY: the caller promises a live CPU number and the debug assertion
    // holds it below the probed count, at most `NCPUS`; the cast cannot wrap
    // because `cpu` is non-negative.
    unsafe { (&raw mut need_ast).cast::<usize>().add(cpu as usize) }
}
