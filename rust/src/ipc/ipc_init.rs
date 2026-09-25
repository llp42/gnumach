// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_init.c and ipc/ipc_init.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The IPC initialization routines, which `ipc/ipc_init.c` defines and
//! `ipc/ipc_init.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue;
use crate::ipc::{ipc_entry, ipc_marequest, ipc_object, ipc_port, ipc_space};
use crate::vm::vm_kern_ffi::kmem_submap;
use crate::vm::vm_map::VmMap;
use core::ptr;

/// `ipc_kernel_map_store` of ipc/ipc_init.c: the storage for the kernel's IPC
/// submap.
static mut IPC_KERNEL_MAP_STORE: VmMap =
    unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

/// `ipc_kernel_map` of ipc/ipc_init.c: the kernel's IPC submap.
#[unsafe(export_name = "ipc_kernel_map")]
static mut IPC_KERNEL_MAP: *mut VmMap =
    ptr::addr_of_mut!(IPC_KERNEL_MAP_STORE);

/// `ipc_kernel_map_size` of ipc/ipc_init.c: the submap's fixed size.
#[unsafe(export_name = "ipc_kernel_map_size")]
static IPC_KERNEL_MAP_SIZE: VmSize = 8 * 1024 * 1024;

/// The kernel's IPC submap.
pub(crate) fn kernel_map() -> *mut VmMap {
    // SAFETY: the initializer is the only writer, and every accessor only
    // reads the pointer.
    unsafe { IPC_KERNEL_MAP }
}

/// `ipc_bootstrap()` in C.
fn bootstrap() {
    ipc_port::init_static_locks();
    ipc_space::init_cache();
    ipc_entry::init_cache();
    ipc_object::init_caches();

    ipc_space::create_specials();

    // SAFETY: the boot path runs this once, before the table is used.
    unsafe { crate::ipc::ipc_table::ipc_table_init() };

    // SAFETY: `ipc_notify_init` takes no arguments and only builds the
    // notification templates; the boot caller runs this once.
    unsafe { glue::ipc_notify_init() };

    // SAFETY: `ipc_marequest_init` only builds the message-accepted table;
    // the boot caller runs this once.
    unsafe { ipc_marequest::init() };
}

/// `ipc_init()` in C.
fn init() {
    let mut min: VmOffset = 0;
    let mut max: VmOffset = 0;

    // SAFETY: `ipc_kernel_map` and `kernel_map` are the live maps the boot
    // path has already built, `ipc_kernel_map_size` is the constant size of
    // the submap, and the two out-pointers are this function's live locals.
    unsafe {
        kmem_submap(
            kernel_map(),
            glue::kernel_map.cast::<VmMap>(),
            &mut min,
            &mut max,
            IPC_KERNEL_MAP_SIZE,
        );
    }

    // SAFETY: `ipc_host_init` takes no arguments and only builds the host's
    // special ports; the boot caller runs this once.
    unsafe { glue::ipc_host_init() };
}

/// `ipc_bootstrap()` of ipc/ipc_init.c.
///
/// # Safety
///
/// The boot path must call this once, before the kernel task can be created.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_bootstrap() {
    bootstrap();
}

/// `ipc_init()` of ipc/ipc_init.c.
///
/// # Safety
///
/// The boot path must have built `kernel_map` and the IPC bootstrap
/// structures, and must call this once, after them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_init() {
    init();
}
