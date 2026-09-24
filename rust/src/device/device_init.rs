// SPDX-License-Identifier: CMU-Mach
// Derived from device/device_init.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device service's creation, which `device/device_init.c` used to define
//! and <device/device_init.h> declares.

use crate::device::chario;
use crate::glue;
use crate::ipc::{IpcPort, ipc_port, ipc_space};
use core::ffi::c_int;
use core::ptr;

/// `device_service_create()` in C.
///
/// # Safety
///
/// Called once, by `kern/startup.c`'s boot sequence after the kernel's IPC
/// space exists, and never concurrently with a device open.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when the master device port cannot be
/// allocated, as the C `panic()` did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_service_create() {
    // SAFETY: the kernel space is the global `ipc_init()` built earlier in
    // the boot; the allocator only reads it and takes its own locks.
    let master = unsafe { ipc_port::alloc_special(ipc_space::kernel()) };
    // SAFETY: this boot step is the global's only writer, and it runs once.
    unsafe {
        glue::master_device_port =
            master.map_or(ptr::null_mut(), IpcPort::as_ptr);
    }
    if master.is_none() {
        // SAFETY: `Panic` does not return; the file, function and message tags
        // are the C `panic()` macro's, and the line is this Rust file's.
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

    // SAFETY: the five initializers take no arguments and build separate
    // module state; the C ran them in this order, before starting the threads
    // below.
    unsafe {
        glue::mach_device_init();
        glue::dev_lookup_init();
        glue::net_io_init();
        glue::device_pager_init();
        chario::chario_init();
    }

    // SAFETY: `kernel_task` is the kernel's own task, live since startup, and
    // both start routines take no argument.
    unsafe {
        crate::kern::thread::kernel_thread(
            crate::kern::task::kernel_task,
            c"io_done".as_ptr(),
            Some(glue::io_done_thread),
            ptr::null_mut(),
        );
        crate::kern::thread::kernel_thread(
            crate::kern::task::kernel_task,
            c"net".as_ptr(),
            Some(glue::net_thread),
            ptr::null_mut(),
        );
    }
}
