// SPDX-License-Identifier: CMU-Mach
// Derived from device/ds_routines.c:
//   Copyright (c) 1993,1991,1990,1989 Carnegie Mellon University.
//   Copyright (c) 1996 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `device/ds_routines.c`, which the MIG device
//! server, the device reply stubs and the C device drivers call.
//!
//! Every adapter here hands its raw arguments to the matching core in
//! [`ds_routines`] without adding an obligation of its own.

use crate::arch::i386::io_req::IoReq;
use crate::arch::types::{VmOffset, VmSize};
use crate::device::ds_routines;
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_ushort, c_void};

/// `ds_device_open()` of `device/ds_routines.c`, which the MIG server and
/// `ds_device_open_new()` call.
///
/// # Safety
///
/// `open_port` is the master device port, `reply_port` a valid port or
/// `IP_NULL`, `name` a NUL-terminated device name, and `devp` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_open(
    open_port: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    name: *const c_char,
    devp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_open(
            open_port,
            reply_port,
            reply_port_type,
            mode,
            name,
            devp,
        )
    }
}

/// `ds_device_open_new()` of the MIG <device/device.server.h>.
///
/// # Safety
///
/// Same contract as [`ds_device_open()`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_open_new(
    open_port: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    name: *const c_char,
    devp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_open(
            open_port,
            reply_port,
            reply_port_type,
            mode,
            name,
            devp,
        )
    }
}

/// `ds_device_close()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is `DEVICE_NULL` or a live `struct device`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_close(dev: *mut c_void) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_device_close(dev) }
}

/// `ds_device_write()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, `data` readable for `count` bytes when
/// non-null, and `bytes_written` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_write(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *mut c_char,
    count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_write(
            dev,
            reply_port,
            reply_port_type,
            mode,
            recnum,
            data,
            count,
            bytes_written,
        )
    }
}

/// `ds_device_write_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, `data` readable for `count` bytes when
/// non-null, and `bytes_written` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_write_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    data: *const c_char,
    count: c_uint,
    bytes_written: *mut c_int,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_write_inband(
            dev,
            reply_port,
            reply_port_type,
            mode,
            recnum,
            data,
            count,
            bytes_written,
        )
    }
}

/// `ds_device_read()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, and `data` and `bytes_read` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_read(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    count: c_int,
    data: *mut *mut c_char,
    bytes_read: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_read(
            dev,
            reply_port,
            reply_port_type,
            mode,
            recnum,
            count,
            data,
            bytes_read,
        )
    }
}

/// `ds_device_read_inband()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, `data` writable for the reply, and
/// `bytes_read` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_read_inband(
    dev: *mut c_void,
    reply_port: *mut c_void,
    reply_port_type: c_uint,
    mode: c_uint,
    recnum: c_ulong,
    count: c_int,
    data: *mut c_char,
    bytes_read: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_read_inband(
            dev,
            reply_port,
            reply_port_type,
            mode,
            recnum,
            count,
            data,
            bytes_read,
        )
    }
}

/// `ds_device_set_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, and `status` readable for `status_count`
/// integers when the emulation reads it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_set_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_set_status(dev, flavor, status, status_count)
    }
}

/// `ds_device_get_status()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, `status` writable for `*status_count`
/// integers, and `status_count` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_get_status(
    dev: *mut c_void,
    flavor: c_uint,
    status: *mut c_int,
    status_count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_get_status(dev, flavor, status, status_count)
    }
}

/// `ds_device_set_filter()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, `receive_port` a valid port, and `filter`
/// readable for `filter_count` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_set_filter(
    dev: *mut c_void,
    receive_port: *mut c_void,
    priority: c_int,
    filter: *mut c_ushort,
    filter_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_set_filter(
            dev,
            receive_port,
            priority,
            filter,
            filter_count,
        )
    }
}

/// `ds_device_map()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, and `pager` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_map(
    dev: *mut c_void,
    protection: c_int,
    offset: VmOffset,
    size: VmSize,
    pager: *mut *mut c_void,
    unmap: c_int,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_map(dev, protection, offset, size, pager, unmap)
    }
}

/// `ds_device_intr_register()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device` for a mach device, and `receive_port` a
/// valid port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_intr_register(
    dev: *mut c_void,
    id: c_int,
    flags: c_int,
    receive_port: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_intr_register(dev, id, flags, receive_port)
    }
}

/// `ds_device_intr_ack()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device` for a mach device, and `receive_port` a
/// valid port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_intr_ack(
    dev: *mut c_void,
    receive_port: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_device_intr_ack(dev, receive_port) }
}

/// `ds_notify()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `msg` is a live message whose header and no-senders body are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_notify(msg: *mut c_void) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_notify(msg) }
}

/// `ds_device_write_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_write_trap(
    dev: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    data: c_ulong,
    count: c_ulong,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_write_trap(dev, mode, recnum, data, count)
    }
}

/// `ds_device_writev_trap()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is a live `struct device`, and `iovec` readable for `count`
/// user-space entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_device_writev_trap(
    dev: *mut c_void,
    mode: c_uint,
    recnum: c_ulong,
    iovec: *mut ds_routines::RpcIoBufVec,
    count: c_ulong,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe {
        ds_routines::ds_device_writev_trap(dev, mode, recnum, iovec, count)
    }
}

/// `device_reference()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is null or a live `struct device`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_reference(dev: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::device_reference(dev) }
}

/// `device_deallocate()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `dev` is null or a live `struct device`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_deallocate(dev: *mut c_void) {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::device_deallocate(dev) }
}

/// `ds_open_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is the live open request `device_open()` built.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_open_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_open_done(ior) }
}

/// `ds_write_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is the live write request `device_write()` queued.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_write_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_write_done(ior) }
}

/// `ds_read_done()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is the live read request `device_read()` queued.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ds_read_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::ds_read_done(ior) }
}

/// `device_write_get()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is a live request whose `io_data` is readable for `io_count` bytes,
/// and `wait` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_write_get(
    ior: *mut IoReq,
    wait: *mut c_int,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::device_write_get(ior, wait) }
}

/// `device_write_dealloc()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is a live request from `io_req_alloc()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_write_dealloc(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::device_write_dealloc(ior) }
}

/// `device_read_alloc()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is a live request with `io_count` bytes to allocate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_read_alloc(
    ior: *mut IoReq,
    size: VmSize,
) -> c_int {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::device_read_alloc(ior, size) }
}

/// `iodone()` of <device/io_req.h>.
///
/// # Safety
///
/// `ior` is a live request whose completion path is not already running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iodone(ior: *mut IoReq) {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::iodone(ior) }
}

/// `iowait()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `ior` is a live request whose `io_done` callback was not `IO_CALL`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iowait(ior: *mut IoReq) {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::iowait(ior) }
}

/// `io_done_thread()` of `device/ds_routines.c`.
///
/// # Safety
///
/// Runs only as the io-done kernel thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn io_done_thread() {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::io_done_thread() }
}

/// `mach_device_init()` of `device/ds_routines.c`.
///
/// # Safety
///
/// `device_service_create()` is the only caller; it runs this once during
/// boot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_device_init() {
    // SAFETY: the caller's contract is the core's.
    unsafe { ds_routines::mach_device_init() }
}
