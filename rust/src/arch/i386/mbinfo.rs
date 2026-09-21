// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/mbinfo.c:
//   Copyright (c) 2024 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! `/dev/mbinfo`: `mbinfo.c`'s raw multiboot information device.
//!
//! The boot path hands the multiboot information block to
//! `mbinfo_register_boot_data()` and a user reads it back with
//! `mbinforead()`, one copy of the raw block, no parsing.

use crate::arch::i386::io_req::{
    D_INVALID_SIZE, D_SUCCESS, DevT, IoReq, KERN_SUCCESS,
};
use crate::glue;
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{c_int, c_long, c_void};
use core::mem::size_of;
use core::ptr;

/// `struct multiboot_framebuffer_info` of <mach/i386/multiboot.h>.
/// The trailing union is opaque here: the device copies the block, it
/// never reads a field.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootFramebufferInfo {
    pub framebuffer_addr: u64,
    pub framebuffer_pitch: u32,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_bpp: u8,
    pub framebuffer_type: u8,
    pub fields: [u8; 6],
}

const _: () = assert!(size_of::<MultibootFramebufferInfo>() == 28);

/// `struct multiboot_raw_info` of <mach/i386/multiboot.h>, `__packed`.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootRawInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub unused0: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
    pub shdr_num: u32,
    pub shdr_size: u32,
    pub shdr_addr: u32,
    pub shdr_strndx: u32,
    pub mmap_length: u32,
    pub mmap_addr: u32,
    pub unused1: [u32; 9],
    pub fb_info: MultibootFramebufferInfo,
}

const _: () = assert!(size_of::<MultibootRawInfo>() == 116);

/// The block the boot loader passed and the one `/dev/mbinfo` serves.
static MB_INFO: SyncCell<MultibootRawInfo> =
    SyncCell(UnsafeCell::new(unsafe { core::mem::zeroed() }));

/// Keep the boot loader's multiboot information block.
///
/// # Safety
///
/// Called once by `model_dep.c` with the block the loader left.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbinfo_register_boot_data(
    mbi: *const MultibootRawInfo,
) {
    // SAFETY: the caller passes the loader's block.
    let info = unsafe { *mbi };
    // SAFETY: the boot path is single-threaded and runs before any read.
    unsafe { *MB_INFO.0.get() = info };
}

/// `mbinforead()` in C.
///
/// # Safety
///
/// Called from the `/dev/mbinfo` device switch in `conf.c`; `ior` must
/// be the request the device layer passed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbinforead(_dev: DevT, ior: *mut IoReq) -> c_int {
    // SAFETY: the device layer owns the request for this call.
    let ior = unsafe { &mut *ior };
    let count = ior.count();
    if count > size_of::<MultibootRawInfo>() as c_long {
        return D_INVALID_SIZE;
    }
    // SAFETY: `count` bytes fit the info block, checked above.
    let err = unsafe {
        glue::device_read_alloc(
            ior as *mut IoReq as *mut c_void,
            count as usize,
        )
    };
    if err != KERN_SUCCESS {
        return err;
    }
    // SAFETY: the request now has a buffer of `count` bytes, and the
    // info block is at least that large.
    unsafe {
        ptr::copy_nonoverlapping(
            MB_INFO.0.get().cast::<u8>(),
            ior.data().cast::<u8>(),
            count as usize,
        )
    };
    ior.set_residual(0);
    D_SUCCESS
}
