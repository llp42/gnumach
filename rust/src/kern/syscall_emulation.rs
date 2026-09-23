// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_emulation.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The user-space system call emulation module, which
//! `kern/syscall_emulation.h` declares.
//!
//! Only [`eml_init()`] moves here so far.  The rest of
//! `kern/syscall_emulation.c` stays C: it works in
//! `struct eml_dispatch`, whose size the emulation vector's length
//! decides at run time and whose offsets
//! `i386/i386/i386asm.sym` projects into the trap path.

/// Initialize the user-space emulation module.  `eml_init()` of
/// kern/syscall_emulation.c.
///
/// The emulation vector is per task and is built by
/// `task_set_emulation_vector()` when a task first asks for one, so
/// the module has no state of its own to set up and the C body is
/// empty.  `task_init()` calls this once at boot and the symbol has
/// to stay.
#[unsafe(no_mangle)]
pub extern "C" fn eml_init() {}
