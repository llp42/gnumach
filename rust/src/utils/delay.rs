// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/loose_ends.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The busy-wait delay, which `i386/i386/loose_ends.c` used to define.
//!
//! A best-effort spin at `CPU_SPEED` iterations per microsecond, the
//! old uncalibrated `cpuspeed` guess.  The `black_box` in the loop is
//! the compiler barrier the C got from its `volatile` counter.

use core::ffi::c_int;

/// Loop iterations per microsecond; the old cpuspeed value.
const CPU_SPEED: c_int = 4;

/// Busy-wait for `n` microseconds, best effort.  `delay()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn delay(n: c_int) {
    let mut remaining = CPU_SPEED.wrapping_mul(n);
    loop {
        remaining = remaining.wrapping_sub(1);
        if remaining <= 0 {
            break;
        }
        core::hint::black_box(remaining);
    }
}
