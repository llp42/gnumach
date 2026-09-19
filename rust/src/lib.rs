// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The Rust half of GNU Mach, built into `libmach-rs.a`.

#![no_std]
// Keeps LLVM from rewriting a byte-copy loop into a call to `memcpy` -- which
// in this crate is a call to itself.
#![no_builtins]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod glue;
pub mod utils;

mod panic;
