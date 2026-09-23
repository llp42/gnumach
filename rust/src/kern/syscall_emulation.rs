// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_emulation.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The user-space system call emulation module, which
//! `kern/syscall_emulation.h` declares.

/// `eml_init()` of kern/syscall_emulation.c.
#[unsafe(no_mangle)]
pub extern "C" fn eml_init() {}
