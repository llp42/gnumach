// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_init.c and ipc/ipc_init.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The final IPC initialization, which `ipc/ipc_init.c` used to define and
//! `ipc/ipc_init.h` declares.

use crate::arch::types::VmOffset;
use crate::glue;

/// `ipc_init()` in C.
fn init() {
    let mut min: VmOffset = 0;
    let mut max: VmOffset = 0;

    // SAFETY: `ipc_kernel_map` and `kernel_map` are the live maps the boot
    // path has already built, `ipc_kernel_map_size` is the constant size of
    // the submap, and the two out-pointers are this function's live locals.
    unsafe {
        glue::kmem_submap(
            glue::ipc_kernel_map,
            glue::kernel_map,
            &mut min,
            &mut max,
            glue::ipc_kernel_map_size,
        );
    }

    // SAFETY: `ipc_host_init` takes no arguments and only builds the host's
    // special ports; the boot caller runs this once.
    unsafe { glue::ipc_host_init() };
}

/// `ipc_init()` in C.
///
/// # Safety
///
/// The boot path must have built `kernel_map` and the IPC bootstrap
/// structures, and must call this once, after them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_init() {
    init();
}
