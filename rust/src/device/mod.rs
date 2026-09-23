// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The Rust home of the C `device/` tree, the machine-independent
//! device layer.
//!
//! `cirbuf.rs` holds the circular character buffers, `dev_name.rs` the
//! device-name comparison and the empty device-table entries,
//! `return.rs` the device return codes, and `subrs.rs` the Ethernet
//! formatter and the BSD wait/wake wrappers.

pub mod cirbuf;
pub mod dev_name;
pub mod r#return;
pub mod subrs;
