// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Byte-order conversions, which `util/byteorder.c` used to define.
//!
//! Network order is big-endian.  `to_be`/`from_be` are the compiler's
//! byte swap on a little-endian machine and the identity on a
//! big-endian one, which is exactly the C's `__builtin_bswap` blocks.

#[unsafe(no_mangle)]
pub extern "C" fn ntohs(netshort: u16) -> u16 {
    u16::from_be(netshort)
}

#[unsafe(no_mangle)]
pub extern "C" fn ntohl(netlong: u32) -> u32 {
    u32::from_be(netlong)
}

#[unsafe(no_mangle)]
pub extern "C" fn htons(hostshort: u16) -> u16 {
    hostshort.to_be()
}

#[unsafe(no_mangle)]
pub extern "C" fn htonl(hostlong: u32) -> u32 {
    hostlong.to_be()
}
