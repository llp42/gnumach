// SPDX-License-Identifier: CMU-Mach
// Derived from device/net_io.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The packet-filter hash of `device/net_io.c`, declared in
//! <device/net_io.h>.
//!
//! Only `bpf_hash()` has moved; the rest of `device/net_io.c` stays C,
//! so the header's declaration and both C callers are unchanged.  The
//! C hashes a caller-owned key array with an additive sum that wraps
//! as `unsigned int` does, then reduces it modulo the 256 buckets
//! `NET_HASH_SIZE` names.

use core::ffi::{c_int, c_uint};
use core::slice;

/// The bucket count the C's `NET_HASH_SIZE` names: 256.
const NET_HASH_SIZE: u32 = 256;

/// Sum `keys` with the C's wrapping addition, reduced modulo the
/// bucket count.
fn hash(keys: &[u32]) -> u32 {
    let mut hval = 0u32;
    for key in keys {
        hval = hval.wrapping_add(*key);
    }
    hval % NET_HASH_SIZE
}

/// The C `bpf_hash()`: hash the `n` keys at `keys` to a filter-table
/// bucket.
///
/// # Safety
///
/// `keys` must be readable for `n` elements when `n` is positive.  A
/// non-positive `n` reads nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bpf_hash(n: c_int, keys: *const c_uint) -> c_uint {
    // A negative count is outside the supported contract: the C's
    // `while (n--)` would run away from zero instead of stopping at
    // it.  Both callers pass `match->jt` or `n_keys` from filter
    // setup, never a negative count, so answering the empty hash here
    // is the port's one deliberate divergence.  A nonnegative `c_int`
    // converts to `usize` without loss on both targets.
    let count = n.max(0) as usize;
    // SAFETY: The caller promises `count` readable elements, and a
    // zero count forms the empty slice without touching `keys`.
    let keys: &[c_uint] = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(keys, count) }
    };
    hash(keys)
}
