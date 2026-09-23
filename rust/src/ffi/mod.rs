// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` entry points C still calls, one module per
//! interface definition file.
//!
//! Each module holds the adapters for one `.defs`: MIG's generated
//! server calls the symbol, the adapter validates what C handed over,
//! calls the safe core in `kern/`, `ipc/` or `vm/`, and turns the
//! answer back into a `kern_return_t`.  Raw pointers, out-parameters
//! and integer error codes stop here; nothing in this directory holds
//! logic of its own.
//!
//! This is the opposite direction from [`crate::glue`], which
//! declares the C functions Rust calls.

pub mod mach_host;
