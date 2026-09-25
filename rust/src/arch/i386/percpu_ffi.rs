// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/percpu.c and i386/i386/percpu.h:
//   Copyright (c) 2023 Free Software Foundation, Inc.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `i386/i386/percpu.c`, one adapter per symbol
//! <i386/percpu.h> declares.

use crate::arch::i386::percpu;
use core::ffi::c_int;

/// `init_percpu()` of <i386/percpu.h>: fill one CPU's per-CPU block.
///
/// # Safety
///
/// `cpu` must be a CPU number the machine reports, below `NCPUS`, and this
/// must run on the CPU the block describes before anything reads it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_percpu(cpu: c_int) {
    // SAFETY: the caller promises a live CPU number.
    unsafe { percpu::init(cpu) };
}
