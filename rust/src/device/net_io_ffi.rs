// SPDX-License-Identifier: CMU-Mach AND BSD-4-Clause-Shortened
// Derived from device/net_io.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
//   The Berkeley Packet Filter section comes from the Stanford/CMU enet
//   packet filter distributed in 4.3BSD:
//   Copyright (c) 1990-1991 The Regents of the University of California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of `device/net_io.c`, declared in
//! <device/net_io.h>.
//!
//! Every adapter hands its raw arguments to the matching core in
//! [`net_io`] and converts the answer back to the C's `int`.

use crate::arch::i386::io_req::IoReq;
use crate::device::net_io::{
    self, BpfInsn, IfNet, NetHashEntry, NetHashHeader, NetRcvPort, QueueChain,
    Validated,
};
use crate::device::r#return::{IoResult, IoResultExt};
use crate::ipc::ipc_kmsg::Kmsg;
use crate::kern::thread::IpcKmsgQueue;
use core::ffi::{c_char, c_int, c_short, c_uint, c_void};
use core::mem::size_of;
use core::ptr;
use core::slice;

/// `bpf_hash()` of device/net_io.c.
///
/// # Safety
///
/// `keys` must be readable for `n` elements when `n` is positive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_hash(n: c_int, keys: *const c_uint) -> c_uint {
    // A negative count is outside the supported contract: the C's `while
    // (n--)` would run away from zero instead of stopping at it.
    let count = n.max(0) as usize;
    // SAFETY: The caller promises `count` readable elements, and a zero count
    // forms the empty slice without touching `keys`.
    let keys: &[c_uint] = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(keys, count) }
    };
    net_io::hash(keys)
}

/// `bpf_validate()` of device/net_io.c.
///
/// # Safety
///
/// `f` must be readable for `bytes` bytes and `bytes` must not be negative.
/// `match` must be writable for one pointer that is null on entry, as
/// `net_set_filter()` leaves it.  The array may be two-byte aligned, which
/// is the alignment of the `filter_t[]` the C casts from.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_validate(
    f: *mut BpfInsn,
    bytes: c_int,
    match_: *mut *mut BpfInsn,
) -> c_int {
    let Ok(bytes) = usize::try_from(bytes) else {
        return 0;
    };
    let len = bytes / size_of::<BpfInsn>();
    if len == 0 || f.is_null() {
        return 0;
    }
    // SAFETY: The caller promises `len` readable instructions at `f`.
    match unsafe { net_io::validate(f, len) } {
        Validated::Invalid => 0,
        Validated::Plain => 1,
        Validated::Match(index) => {
            // SAFETY: The validated index is inside the array at `f`, and
            // the caller promises `match` is writable.
            unsafe { *match_ = f.add(index) };
            2
        }
    }
}

/// `bpf_eq()` of device/net_io.c.
///
/// # Safety
///
/// Both arrays must each be readable for `bytes` bytes, and `bytes` must not
/// be negative: the C's loop never terminated for one.  They may be two-byte
/// aligned, as the `filter_t[]` casts are.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_eq(
    f1: *mut BpfInsn,
    f2: *mut BpfInsn,
    bytes: c_int,
) -> c_int {
    let Ok(bytes) = usize::try_from(bytes) else {
        return 0;
    };
    let len = bytes / size_of::<BpfInsn>();
    if len == 0 {
        return 1;
    }
    if f1.is_null() || f2.is_null() {
        return 0;
    }
    // SAFETY: The caller promises `len` readable instructions at each.
    c_int::from(unsafe { net_io::eq(f1, f2, len) })
}

/// `bpf_match()` of device/net_io.c.
///
/// # Safety
///
/// `hash` must point at a live `struct net_hash_header` with a well-formed
/// `table`; `keys` must be readable for `n_keys` elements when positive;
/// `hash_headpp` and `entpp` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_match(
    hash: *mut NetHashHeader,
    n_keys: c_int,
    keys: *const c_uint,
    hash_headpp: *mut *mut *mut NetHashEntry,
    entpp: *mut *mut NetHashEntry,
) -> c_int {
    let Ok(count) = usize::try_from(n_keys) else {
        return 0;
    };
    if hash.is_null() || count > net_io::N_NET_HASH_KEYS {
        return 0;
    }
    let keys: &[c_uint] = if count == 0 {
        &[]
    } else {
        if keys.is_null() {
            return 0;
        }
        // SAFETY: The caller promises `count` readable keys.
        unsafe { slice::from_raw_parts(keys, count) }
    };
    // SAFETY: The caller promises a live header and writable out-params;
    // `find_match` returns without comparing any key when the counts differ.
    match unsafe { net_io::find_match(hash, keys) } {
        Some(m) => {
            // SAFETY: The caller promises both out-params writable.
            unsafe {
                *hash_headpp = m.slot;
            }
            if m.entry.is_null() {
                return 0;
            }
            // SAFETY: as above.
            unsafe {
                *entpp = m.entry;
            }
            1
        }
        None => 0,
    }
}

/// `bpf_do_filter()` of device/net_io.c.
///
/// # Safety
///
/// `infp` must point at a live `struct net_rcv_port` or `struct
/// net_hash_header` whose `filter` and `filter_end` delimit a program
/// `bpf_validate()` accepted; `p` must be readable for
/// [`net_io::NET_RCV_MAX`] bytes with the three bytes before it readable
/// too; `header` must be readable for `hlen` bytes; `hash_headpp` and `entpp`
/// must be writable, and `*entpp` must start null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_do_filter(
    infp: *mut NetRcvPort,
    p: *mut c_char,
    wirelen: c_uint,
    header: *mut c_char,
    hlen: c_uint,
    hash_headpp: *mut *mut *mut NetHashEntry,
    entpp: *mut *mut NetHashEntry,
) -> c_int {
    if infp.is_null() || hash_headpp.is_null() || entpp.is_null() {
        return 0;
    }
    // SAFETY: The caller promises `entpp` is writable, and the C left
    // `*entpp` at zero before running the program.
    unsafe { *entpp = ptr::null_mut() };
    // SAFETY: The caller promises the live port, packet windows and
    // out-params the interpreter reads and writes.
    unsafe {
        net_io::do_filter(
            infp,
            p.cast::<u8>().cast_const(),
            wirelen,
            header.cast::<u8>().cast_const(),
            hlen,
            hash_headpp,
            entpp,
        )
    }
}

/// `net_kmsg_get()` of <device/net_io.h>.
///
/// # Safety
///
/// Called at splimp; the caller owns the returned message and must return it
/// through `net_kmsg_put()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_kmsg_get() -> *mut c_void {
    // SAFETY: the caller's contract.
    unsafe { net_io::kmsg_get() }.map_or(ptr::null_mut(), Kmsg::as_ptr)
}

/// `net_kmsg_put()` of <device/net_io.h>.
///
/// # Safety
///
/// `kmsg` must be a message from `net_kmsg_get()` that nothing else uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_kmsg_put(kmsg: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { net_io::kmsg_put(kmsg) };
}

/// `net_kmsg_collect()` of <device/net_io.h>.
///
/// # Safety
///
/// Called when the free pool may be trimmed; nothing may hold a free-listed
/// message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_kmsg_collect() {
    // SAFETY: the caller's contract.
    unsafe { net_io::kmsg_collect() };
}

/// `net_ast()` of <device/net_io.h>.
///
/// # Safety
///
/// Called at splimp from `ast_taken()`, with the network AST bit set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_ast() {
    // SAFETY: the caller's contract.
    unsafe { net_io::ast() };
}

/// `net_thread()` of <device/net_io.h>.
///
/// # Safety
///
/// Started once as the network thread; it never returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_thread() {
    // SAFETY: the caller's contract.
    unsafe { net_io::thread() };
}

/// `net_packet()` of <device/net_io.h>.
///
/// # Safety
///
/// `ifp` must be a live interface and `kmsg` a live network message at
/// splimp.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_packet(
    ifp: *mut IfNet,
    kmsg: *mut c_void,
    count: c_uint,
    priority: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe { net_io::packet(ifp, kmsg, count, priority != 0) };
}

/// `net_filter()` of <device/net_io.h>.
///
/// # Safety
///
/// `kmsg` must be a live network message at spl0 holding the interface
/// pointer, and `send_list` an empty queue the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_filter(
    kmsg: *mut c_void,
    send_list: *mut IpcKmsgQueue,
) {
    // SAFETY: the caller's contract.
    unsafe { net_io::filter(Kmsg::from_raw(kmsg), send_list) };
}

/// `net_do_filter()` of <device/net_io.h>.
///
/// # Safety
///
/// `infp` must point at a live receive port whose filter `net_set_filter()`
/// accepted, and `data`/`header` must be readable for the words the program
/// addresses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_do_filter(
    infp: *mut NetRcvPort,
    data: *const c_char,
    data_count: c_uint,
    header: *const c_char,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe {
        net_io::net_do_filter(
            infp,
            data.cast::<u8>(),
            data_count,
            header.cast::<u8>(),
        )
    })
}

/// `net_set_filter()` of <device/net_io.h>.
///
/// # Safety
///
/// `ifp` must be a live interface; `rcv_port` a naked send right the caller
/// hands over on success; `filter` readable for `filter_count` `filter_t`
/// words; no interface lock may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_set_filter(
    ifp: *mut IfNet,
    rcv_port: *mut c_void,
    priority: c_int,
    filter: *mut u16,
    filter_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        net_io::set_filter(ifp, rcv_port, priority, filter, filter_count)
    }
    .as_io_return()
}

/// `net_getstat()` of <device/net_io.h>.
///
/// # Safety
///
/// `ifp` must be a live interface, `status` writable for `*count` words, and
/// `count` readable and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_getstat(
    ifp: *mut IfNet,
    flavor: c_int,
    status: *mut c_int,
    count: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { net_io::getstat(ifp, flavor, status, count) }.as_io_return()
}

/// `net_write()` of <device/net_io.h>.
///
/// # Safety
///
/// `ifp` must be a live interface, `ior` a live request the caller owns, and
/// `start` the driver's start routine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_write(
    ifp: *mut IfNet,
    start: Option<unsafe extern "C" fn(c_short) -> c_int>,
    ior: *mut IoReq,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { net_io::write(ifp, start, ior) } {
        Ok(success) => {
            let result: IoResult = Ok(success);
            result.as_io_return()
        }
        Err(net_io::WriteError::Device(error)) => {
            let result: IoResult = Err(error);
            result.as_io_return()
        }
        // The C returned the `kern_return_t` untouched.
        Err(net_io::WriteError::Kern(rc)) => rc,
    }
}

/// `net_io_init()` of <device/net_io.h>.
///
/// # Safety
///
/// Called once during boot, before the network thread or any interface
/// filter exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_io_init() {
    // SAFETY: the caller's contract.
    unsafe { net_io::init() };
}

/// `ethernet_priority()` of <device/net_io.h>.
///
/// # Safety
///
/// `kmsg` must be a live network message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ethernet_priority(kmsg: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe { net_io::ethernet_priority(Kmsg::from_raw(kmsg)) })
}

/// `hash_ent_remove()` of <device/net_io.h>.
///
/// # Safety
///
/// `hp` and `entp` must be live filter structures, `head` the bucket `entp`
/// is linked into, and `dead_p` a list head the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hash_ent_remove(
    ifp: *mut IfNet,
    hp: *mut NetHashHeader,
    used: c_int,
    head: *mut *mut NetHashEntry,
    entp: *mut NetHashEntry,
    dead_p: *mut *mut QueueChain,
) -> c_int {
    // SAFETY: the caller's contract.
    c_int::from(unsafe {
        net_io::hash_ent_remove(ifp, hp, used != 0, head, entp, dead_p)
    })
}

/// `net_add_q_info()` of <device/net_io.h>.
///
/// # Safety
///
/// `rcv_port` must be `IP_NULL`, `IP_DEAD` or a live port.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_add_q_info(rcv_port: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { net_io::add_q_info(rcv_port) }
}

/// `net_free_dead_infp()` of <device/net_io.h>.
///
/// # Safety
///
/// `dead_infp` must head a list of receive ports that nothing else uses, and
/// no lock may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_free_dead_infp(dead_infp: *mut QueueChain) {
    // SAFETY: the caller's contract.
    unsafe { net_io::free_dead_infp(dead_infp) };
}

/// `net_free_dead_entp()` of <device/net_io.h>.
///
/// # Safety
///
/// `dead_entp` must head a list of hash entries that nothing else uses, and
/// no lock may be held.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn net_free_dead_entp(dead_entp: *mut QueueChain) {
    // SAFETY: the caller's contract.
    unsafe { net_io::free_dead_entp(dead_entp) };
}
