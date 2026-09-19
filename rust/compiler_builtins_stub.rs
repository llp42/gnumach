// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! An empty `compiler_builtins`.
//!
//! rustc insists that a `staticlib` has this crate available, but the kernel
//! already links libgcc for exactly these intrinsics (`__udivdi3` and
//! friends, see `libgcc_routines` in `Makefile.am`), and supplies its own
//! `memcpy`/`memset`.  Providing a second set would only add duplicate
//! symbols, so this crate deliberately defines nothing and lets the linker
//! resolve the intrinsics the way the C half of the kernel already does.

#![no_std]
#![feature(compiler_builtins)]
#![compiler_builtins]
#![allow(internal_features)]
