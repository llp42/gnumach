// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Virtual memory; mirrors `vm/`.

pub mod error;
pub mod types;
pub mod vm_external;
pub mod vm_fault;
pub mod vm_fault_ffi;
pub mod vm_init;
pub mod vm_kern;
pub mod vm_kern_ffi;
pub mod vm_map;
pub mod vm_map_ffi;
pub mod vm_object;
pub mod vm_object_ffi;
pub mod vm_page;
pub mod vm_page_ffi;
pub mod vm_resident;
pub mod vm_resident_ffi;
pub mod vm_user;
pub mod vm_user_ffi;
