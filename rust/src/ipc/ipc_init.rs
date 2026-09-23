// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_init.c and ipc/ipc_init.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The final IPC initialization, which `ipc/ipc_init.c` used to define
//! and `ipc/ipc_init.h` declares.
//!
//! [`ipc_init()`] carves the IPC kernel submap out of `kernel_map` and
//! brings up the host ports.  The earlier half of the file,
//! `ipc_bootstrap()`, and the `ipc_kernel_map` globals stay C: the rest
//! of the IPC layer reads them directly.

use crate::arch::types::VmOffset;
use crate::glue;

/// Final initialization of the IPC system.  `ipc_init()` in C.
fn init() {
    // `kmem_submap` writes both back; the C passed uninitialized
    // locals.
    let mut min: VmOffset = 0;
    let mut max: VmOffset = 0;

    // SAFETY: `ipc_kernel_map` and `kernel_map` are the live maps the
    // boot path has already built, `ipc_kernel_map_size` is the
    // constant size of the submap, and the two out-pointers are this
    // function's live locals.  `kmem_submap` halts instead of
    // returning on failure.
    unsafe {
        glue::kmem_submap(
            glue::ipc_kernel_map,
            glue::kernel_map,
            &mut min,
            &mut max,
            glue::ipc_kernel_map_size,
        );
    }

    // SAFETY: `ipc_host_init` takes no arguments and only builds the
    // host's special ports; the boot caller runs this once.
    unsafe { glue::ipc_host_init() };
}

//
// The C edge: the one adapter `ipc/ipc_init.h` declares.
//

/// Final initialization of the IPC system.  `ipc_init()` in C.
///
/// # Safety
///
/// The boot path must have built `kernel_map` and the IPC bootstrap
/// structures, and must call this once, after them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_init() {
    init();
}
