// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The i386 tree, shared by the i686 and x86_64 kernels; mirrors `i386/`.

pub mod ast_check;
pub mod atomic_bits;
pub mod fpu;
pub mod io_req;
pub mod kd;
pub mod kd_event;
pub mod kd_mouse;
pub mod mbinfo;
pub mod mem;
pub mod mp_desc;
pub mod percpu;
pub mod pio;
