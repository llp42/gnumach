// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Machine scalar types, from `i386/include/mach/i386/vm_types.h`, the
//! header both x86 kernels use (`x86_64/include/mach/x86_64` is a
//! symlink into the i386 headers).
#![cfg(any(target_arch = "x86", target_arch = "x86_64"))]

/// `vm_offset_t`: a type-neutral pointer, `uintptr_t` in the C.
pub type VmOffset = usize;

/// `vm_size_t`: the difference between two `vm_offset_t`s, likewise a
/// `uintptr_t` in the C.
pub type VmSize = usize;
