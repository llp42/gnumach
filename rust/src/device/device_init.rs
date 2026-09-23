// SPDX-License-Identifier: CMU-Mach
// Derived from device/device_init.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device service's creation, which `device/device_init.c` used to
//! define and <device/device_init.h> declares.
//!
//! `device_service_create()` allocates the master device port in the
//! kernel's IPC space and then brings the device layer up in the C
//! order: the five module initializers, and the I/O-completion and
//! network service threads.  The port itself stays the C global that
//! `device/device_init.c` defines; this module fills it in.

use crate::glue;
use core::ffi::c_int;
use core::ptr;

/// Create the device service inside the kernel task.
/// `device_service_create()` in C.
///
/// # Safety
///
/// Called once, by `kern/startup.c`'s boot sequence after the kernel's
/// IPC space exists, and never concurrently with a device open.  The
/// C had no other caller.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when the master device port cannot be
/// allocated, as the C `panic()` did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_service_create() {
    // `ipc_port_alloc_kernel()` in C: the macro expands to
    // `ipc_port_alloc_special(ipc_space_kernel)`.
    // SAFETY: the kernel space is the global `ipc_init()` built earlier
    // in the boot; the allocator only reads it and takes its own locks.
    let master =
        unsafe { glue::ipc_port_alloc_special(glue::ipc_space_kernel) };
    // SAFETY: this boot step is the global's only writer, and it runs
    // once.
    unsafe { glue::master_device_port = master };
    if master.is_null() {
        // SAFETY: `Panic` does not return; the file, function and
        // message tags are the C `panic()` macro's, and the line is
        // this Rust file's.
        unsafe {
            glue::Panic(
                c"device/device_init.c".as_ptr(),
                // Only `c_int` widths can reach `Panic`'s varargs.
                line!() as c_int,
                c"device_service_create".as_ptr(),
                c"can't allocate master device port".as_ptr(),
            )
        }
    }

    // SAFETY: the five initializers take no arguments and build
    // separate module state; the C ran them in this order, before
    // starting the threads below.
    unsafe {
        glue::mach_device_init();
        glue::dev_lookup_init();
        glue::net_io_init();
        glue::device_pager_init();
        glue::chario_init();
    }

    // The C discarded both returned threads; `io_done_thread` and
    // `net_thread` run until the machine stops.
    // SAFETY: `kernel_task` is the kernel's own task, live since
    // startup, and both start routines take no argument.
    unsafe {
        glue::kernel_thread(
            glue::kernel_task,
            c"io_done".as_ptr(),
            Some(glue::io_done_thread),
            ptr::null_mut(),
        );
        glue::kernel_thread(
            glue::kernel_task,
            c"net".as_ptr(),
            Some(glue::net_thread),
            ptr::null_mut(),
        );
    }
}
