// SPDX-License-Identifier: CMU-Mach AND BSD-4-Clause-Shortened
// Derived from device/net_io.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
//   The Berkeley Packet Filter section comes from the Stanford/CMU enet
//   packet filter distributed in 4.3BSD:
//   Copyright (c) 1990-1991 The Regents of the University of California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The `extern "C"` exports of the packet-filter routines `device/net_io.c`
//! used to define, declared in <device/net_io.h>.
//!
//! Every adapter hands its raw arguments to the matching core in
//! [`net_io`] and converts the answer back to the C's `int`.

use crate::device::net_io::{
    self, BpfInsn, NetHashEntry, NetHashHeader, NetRcvPort, Validated,
};
use core::ffi::{c_char, c_int, c_uint};
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
