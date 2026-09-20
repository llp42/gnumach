// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Byte-order conversions, which `util/byteorder.c` used to define.
//!
//! Network order is big-endian.  `to_be`/`from_be` are the compiler's
//! byte swap on a little-endian machine and the identity on a
//! big-endian one, which is exactly the C's `__builtin_bswap` blocks.

/// Convert a 16-bit value from network to host order.  `ntohs()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ntohs(netshort: u16) -> u16 {
    u16::from_be(netshort)
}

/// Convert a 32-bit value from network to host order.  `ntohl()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ntohl(netlong: u32) -> u32 {
    u32::from_be(netlong)
}

/// Convert a 16-bit value from host to network order.  `htons()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn htons(hostshort: u16) -> u16 {
    hostshort.to_be()
}

/// Convert a 32-bit value from host to network order.  `htonl()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn htonl(hostlong: u32) -> u32 {
    hostlong.to_be()
}
