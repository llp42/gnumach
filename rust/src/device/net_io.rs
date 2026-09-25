// SPDX-License-Identifier: CMU-Mach AND BSD-4-Clause-Shortened
// Derived from device/net_io.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
//   The Berkeley Packet Filter section comes from the Stanford/CMU enet
//   packet filter distributed in 4.3BSD:
//   Copyright (c) 1990-1991 The Regents of the University of California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The packet-filter machinery of `device/net_io.c`: the `bpf_hash()` bucket
//! map, the BPF validator and interpreter, and the `struct net_rcv_port`,
//! `struct net_hash_entry` and `struct net_hash_header` layouts their bodies
//! read.
//!
//! The rest of `device/net_io.c` stays C for now, so the mirrors below must
//! track its definitions until the hash and receive paths move.

use core::ffi::{c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};

/// `NET_MAX_FILTER` of <device/net_status.h>: the `filter_t[]` a receive port
/// holds.
pub(crate) const NET_MAX_FILTER: usize = 128;
/// `NET_RCV_MAX` of <device/net_status.h>: the packet bytes a network message
/// carries.
pub(crate) const NET_RCV_MAX: u32 = 4095;
/// `NET_HASH_SIZE` of `device/net_io.c`: the buckets a filter hash has.
pub(crate) const NET_HASH_SIZE: u32 = 256;
/// `N_NET_HASH_KEYS` of `device/net_io.c`: the keys one match instruction
/// carries.
pub(crate) const N_NET_HASH_KEYS: usize = 4;
/// `BPF_MEMWORDS` of <device/bpf.h>: the interpreter's scratch words.
const BPF_MEMWORDS: usize = 16;

/// `struct bpf_insn` of <device/bpf.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BpfInsn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: c_int,
}

const _: () = assert!(size_of::<BpfInsn>() == 8);
const _: () = assert!(core::mem::align_of::<BpfInsn>() == 4);
const _: () = assert!(offset_of!(BpfInsn, code) == 0);
const _: () = assert!(offset_of!(BpfInsn, jt) == 2);
const _: () = assert!(offset_of!(BpfInsn, jf) == 3);
const _: () = assert!(offset_of!(BpfInsn, k) == 4);

/// `queue_chain_t` of <kern/queue.h>: the two links the network chains use.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct QueueChain {
    pub next: *mut QueueChain,
    pub prev: *mut QueueChain,
}

const _: () = assert!(size_of::<QueueChain>() == 2 * size_of::<*mut ()>());
const _: () = assert!(
    core::mem::align_of::<QueueChain>() == core::mem::align_of::<*mut ()>()
);
const _: () = assert!(offset_of!(QueueChain, next) == 0);
const _: () = assert!(offset_of!(QueueChain, prev) == size_of::<*mut ()>());

/// `struct net_rcv_port` of `device/net_io.c`.
#[repr(C)]
pub struct NetRcvPort {
    pub input: QueueChain,
    pub output: QueueChain,
    pub rcv_port: *mut c_void,
    pub rcv_qlimit: c_int,
    pub rcv_count: c_int,
    pub priority: c_int,
    pub filter_end: *mut u16,
    pub filter: [u16; NET_MAX_FILTER],
}

// The size, alignment and offsets are gdb's `ptype /o struct net_rcv_port`
// over build-64/gnumach and build-32/gnumach.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NetRcvPort>() == 320);
    assert!(core::mem::align_of::<NetRcvPort>() == 8);
    assert!(offset_of!(NetRcvPort, input) == 0);
    assert!(offset_of!(NetRcvPort, output) == 16);
    assert!(offset_of!(NetRcvPort, rcv_port) == 32);
    assert!(offset_of!(NetRcvPort, rcv_qlimit) == 40);
    assert!(offset_of!(NetRcvPort, rcv_count) == 44);
    assert!(offset_of!(NetRcvPort, priority) == 48);
    assert!(offset_of!(NetRcvPort, filter_end) == 56);
    assert!(offset_of!(NetRcvPort, filter) == 64);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NetRcvPort>() == 292);
    assert!(core::mem::align_of::<NetRcvPort>() == 4);
    assert!(offset_of!(NetRcvPort, input) == 0);
    assert!(offset_of!(NetRcvPort, output) == 8);
    assert!(offset_of!(NetRcvPort, rcv_port) == 16);
    assert!(offset_of!(NetRcvPort, rcv_qlimit) == 20);
    assert!(offset_of!(NetRcvPort, rcv_count) == 24);
    assert!(offset_of!(NetRcvPort, priority) == 28);
    assert!(offset_of!(NetRcvPort, filter_end) == 32);
    assert!(offset_of!(NetRcvPort, filter) == 36);
};

/// `struct net_hash_entry` of `device/net_io.c`.  Its `he_next` macro is
/// `chain.next`.
#[repr(C)]
pub struct NetHashEntry {
    pub chain: QueueChain,
    pub rcv_port: *mut c_void,
    pub rcv_qlimit: c_int,
    pub keys: [c_uint; N_NET_HASH_KEYS],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NetHashEntry>() == 48);
    assert!(core::mem::align_of::<NetHashEntry>() == 8);
    assert!(offset_of!(NetHashEntry, chain) == 0);
    assert!(offset_of!(NetHashEntry, rcv_port) == 16);
    assert!(offset_of!(NetHashEntry, rcv_qlimit) == 24);
    assert!(offset_of!(NetHashEntry, keys) == 28);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NetHashEntry>() == 32);
    assert!(core::mem::align_of::<NetHashEntry>() == 4);
    assert!(offset_of!(NetHashEntry, chain) == 0);
    assert!(offset_of!(NetHashEntry, rcv_port) == 8);
    assert!(offset_of!(NetHashEntry, rcv_qlimit) == 12);
    assert!(offset_of!(NetHashEntry, keys) == 16);
};

/// `struct net_hash_header` of `device/net_io.c`: a [`NetRcvPort`] with a
/// 256-bucket hash table bolted on, so both can live on the same port lists.
#[repr(C)]
pub struct NetHashHeader {
    pub rcv: NetRcvPort,
    pub n_keys: c_int,
    pub ref_count: c_int,
    pub table: [*mut NetHashEntry; NET_HASH_SIZE as usize],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NetHashHeader>() == 2376);
    assert!(core::mem::align_of::<NetHashHeader>() == 8);
    assert!(offset_of!(NetHashHeader, rcv) == 0);
    assert!(offset_of!(NetHashHeader, n_keys) == 320);
    assert!(offset_of!(NetHashHeader, ref_count) == 324);
    assert!(offset_of!(NetHashHeader, table) == 328);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NetHashHeader>() == 1324);
    assert!(core::mem::align_of::<NetHashHeader>() == 4);
    assert!(offset_of!(NetHashHeader, rcv) == 0);
    assert!(offset_of!(NetHashHeader, n_keys) == 292);
    assert!(offset_of!(NetHashHeader, ref_count) == 296);
    assert!(offset_of!(NetHashHeader, table) == 300);
};

/// `BPF_CLASS()` of <device/bpf.h>.
const BPF_CLASS_MASK: u16 = 0x07;
/// `BPF_OP()` of <device/bpf.h>.
const BPF_OP_MASK: u16 = 0xf0;
/// `BPF_MODE()` of <device/bpf.h>.
const BPF_MODE_MASK: u16 = 0xe0;

/// `BPF_JMP` of <device/bpf.h>.
const BPF_JMP: u16 = 0x05;
/// `BPF_JA` of <device/bpf.h>.
const BPF_JA: u16 = 0x00;
/// `BPF_ST` of <device/bpf.h>.
const BPF_ST: u16 = 0x02;
/// `BPF_LD` of <device/bpf.h>.
const BPF_LD: u16 = 0x00;
/// `BPF_MEM` of <device/bpf.h>.
const BPF_MEM: u16 = 0x60;
/// `BPF_RET` of <device/bpf.h>.
const BPF_RET: u16 = 0x06;

/// `BPF_LD|BPF_W|BPF_ABS` of <device/bpf.h>.
const BPF_LD_W_ABS: u16 = 0x20;
/// `BPF_LD|BPF_H|BPF_ABS` of <device/bpf.h>.
const BPF_LD_H_ABS: u16 = 0x28;
/// `BPF_LD|BPF_B|BPF_ABS` of <device/bpf.h>.
const BPF_LD_B_ABS: u16 = 0x30;
/// `BPF_LD|BPF_W|BPF_LEN` of <device/bpf.h>.
const BPF_LD_W_LEN: u16 = 0x80;
/// `BPF_LDX|BPF_W|BPF_LEN` of <device/bpf.h>.
const BPF_LDX_W_LEN: u16 = 0x81;
/// `BPF_LD|BPF_W|BPF_IND` of <device/bpf.h>.
const BPF_LD_W_IND: u16 = 0x40;
/// `BPF_LD|BPF_H|BPF_IND` of <device/bpf.h>.
const BPF_LD_H_IND: u16 = 0x48;
/// `BPF_LD|BPF_B|BPF_IND` of <device/bpf.h>.
const BPF_LD_B_IND: u16 = 0x50;
/// `BPF_LDX|BPF_MSH|BPF_B` of <device/bpf.h>.
const BPF_LDX_MSH_B: u16 = 0xb1;
/// `BPF_LD|BPF_IMM` of <device/bpf.h>.
const BPF_LD_IMM: u16 = 0x00;
/// `BPF_LDX|BPF_IMM` of <device/bpf.h>.
const BPF_LDX_IMM: u16 = 0x01;
/// `BPF_LD|BPF_MEM` of <device/bpf.h>.
const BPF_LD_MEM: u16 = 0x60;
/// `BPF_LDX|BPF_MEM` of <device/bpf.h>.
const BPF_LDX_MEM: u16 = 0x61;
/// `BPF_STX` of <device/bpf.h>.
const BPF_STX: u16 = 0x03;
/// `BPF_JMP|BPF_JA` of <device/bpf.h>.
const BPF_JMP_JA: u16 = 0x05;
/// `BPF_JMP|BPF_JGT|BPF_K` of <device/bpf.h>.
const BPF_JMP_JGT_K: u16 = 0x25;
/// `BPF_JMP|BPF_JGE|BPF_K` of <device/bpf.h>.
const BPF_JMP_JGE_K: u16 = 0x35;
/// `BPF_JMP|BPF_JEQ|BPF_K` of <device/bpf.h>.
const BPF_JMP_JEQ_K: u16 = 0x15;
/// `BPF_JMP|BPF_JSET|BPF_K` of <device/bpf.h>.
const BPF_JMP_JSET_K: u16 = 0x45;
/// `BPF_JMP|BPF_JGT|BPF_X` of <device/bpf.h>.
const BPF_JMP_JGT_X: u16 = 0x2d;
/// `BPF_JMP|BPF_JGE|BPF_X` of <device/bpf.h>.
const BPF_JMP_JGE_X: u16 = 0x3d;
/// `BPF_JMP|BPF_JEQ|BPF_X` of <device/bpf.h>.
const BPF_JMP_JEQ_X: u16 = 0x1d;
/// `BPF_JMP|BPF_JSET|BPF_X` of <device/bpf.h>.
const BPF_JMP_JSET_X: u16 = 0x4d;
/// `BPF_ALU|BPF_ADD|BPF_X` of <device/bpf.h>.
const BPF_ALU_ADD_X: u16 = 0x0c;
/// `BPF_ALU|BPF_SUB|BPF_X` of <device/bpf.h>.
const BPF_ALU_SUB_X: u16 = 0x1c;
/// `BPF_ALU|BPF_MUL|BPF_X` of <device/bpf.h>.
const BPF_ALU_MUL_X: u16 = 0x2c;
/// `BPF_ALU|BPF_DIV|BPF_X` of <device/bpf.h>.
const BPF_ALU_DIV_X: u16 = 0x3c;
/// `BPF_ALU|BPF_MOD|BPF_X` of <device/bpf.h>.
const BPF_ALU_MOD_X: u16 = 0x9c;
/// `BPF_ALU|BPF_AND|BPF_X` of <device/bpf.h>.
const BPF_ALU_AND_X: u16 = 0x5c;
/// `BPF_ALU|BPF_OR|BPF_X` of <device/bpf.h>.
const BPF_ALU_OR_X: u16 = 0x4c;
/// `BPF_ALU|BPF_XOR|BPF_X` of <device/bpf.h>.
const BPF_ALU_XOR_X: u16 = 0xac;
/// `BPF_ALU|BPF_LSH|BPF_X` of <device/bpf.h>.
const BPF_ALU_LSH_X: u16 = 0x6c;
/// `BPF_ALU|BPF_RSH|BPF_X` of <device/bpf.h>.
const BPF_ALU_RSH_X: u16 = 0x7c;
/// `BPF_ALU|BPF_ADD|BPF_K` of <device/bpf.h>.
const BPF_ALU_ADD_K: u16 = 0x04;
/// `BPF_ALU|BPF_SUB|BPF_K` of <device/bpf.h>.
const BPF_ALU_SUB_K: u16 = 0x14;
/// `BPF_ALU|BPF_MUL|BPF_K` of <device/bpf.h>.
const BPF_ALU_MUL_K: u16 = 0x24;
/// `BPF_ALU|BPF_DIV|BPF_K` of <device/bpf.h>.
const BPF_ALU_DIV_K: u16 = 0x34;
/// `BPF_ALU|BPF_MOD|BPF_K` of <device/bpf.h>.
const BPF_ALU_MOD_K: u16 = 0x94;
/// `BPF_ALU|BPF_AND|BPF_K` of <device/bpf.h>.
const BPF_ALU_AND_K: u16 = 0x54;
/// `BPF_ALU|BPF_OR|BPF_K` of <device/bpf.h>.
const BPF_ALU_OR_K: u16 = 0x44;
/// `BPF_ALU|BPF_XOR|BPF_K` of <device/bpf.h>.
const BPF_ALU_XOR_K: u16 = 0xa4;
/// `BPF_ALU|BPF_LSH|BPF_K` of <device/bpf.h>.
const BPF_ALU_LSH_K: u16 = 0x64;
/// `BPF_ALU|BPF_RSH|BPF_K` of <device/bpf.h>.
const BPF_ALU_RSH_K: u16 = 0x74;
/// `BPF_ALU|BPF_NEG` of <device/bpf.h>.
const BPF_ALU_NEG: u16 = 0x84;
/// `BPF_MISC|BPF_TAX` of <device/bpf.h>.
const BPF_MISC_TAX: u16 = 0x07;
/// `BPF_MISC|BPF_TXA` of <device/bpf.h>.
const BPF_MISC_TXA: u16 = 0x87;
/// `BPF_RET|BPF_K` of <device/bpf.h>.
const BPF_RET_K: u16 = 0x06;
/// `BPF_RET|BPF_A` of <device/bpf.h>.
const BPF_RET_A: u16 = 0x16;
/// `BPF_RET|BPF_MATCH_IMM` of <device/bpf.h>.
const BPF_RET_MATCH_IMM: u16 = 0x1e;
/// `BPF_MISC|BPF_KEY` of <device/bpf.h>: the match instruction's key words.
const BPF_MISC_KEY: u16 = 0x17;

/// Sum `keys` with the C's wrapping addition, reduced modulo the bucket count.
pub(crate) fn hash(keys: &[c_uint]) -> u32 {
    let mut hval = 0u32;
    for key in keys {
        hval = hval.wrapping_add(*key);
    }
    hval % NET_HASH_SIZE
}

/// What a `bpf_match()` search found once the key counts agreed.
pub(crate) struct Match {
    /// The `table` slot the C wrote to `*hash_headpp`.
    pub(crate) slot: *mut *mut NetHashEntry,
    /// The entry whose keys matched, null when the bucket or the search was
    /// empty.
    pub(crate) entry: *mut NetHashEntry,
}

/// The C `bpf_match()`: the bucket slot of `header` the keys name, and the
/// entry in it whose keys equal `keys`.
///
/// # Safety
///
/// `header` must point at a live `struct net_hash_header` whose `table` is
/// well formed, and `keys` must be at most [`N_NET_HASH_KEYS`] long.  The C
/// read `keys[i]` up to `header.n_keys`, so a longer slice is outside its
/// contract.
pub(crate) unsafe fn find_match(
    header: *mut NetHashHeader,
    keys: &[c_uint],
) -> Option<Match> {
    // SAFETY: the caller promises a live header.
    let n_keys = unsafe { (*header).n_keys };
    if n_keys != keys.len() as c_int {
        return None;
    }
    if keys.len() > N_NET_HASH_KEYS {
        return None;
    }
    // SAFETY: as above; `hash()` stays below `NET_HASH_SIZE`, the table's
    // length.
    let slot =
        unsafe { (*header).table.as_mut_ptr().add(hash(keys) as usize) };
    // SAFETY: `slot` points into the live table.
    let head = unsafe { *slot };
    if head.is_null() {
        return Some(Match {
            slot,
            entry: core::ptr::null_mut(),
        });
    }
    let mut entry = head;
    loop {
        // SAFETY: `entry` is a chain element of the live table, so it points
        // at a live entry.
        let entry_keys = unsafe { (*entry).keys };
        if keys == &entry_keys[..keys.len()] {
            return Some(Match { slot, entry });
        }
        // SAFETY: as above; the chain is circular and ends at `head`.
        entry = unsafe { (*entry).chain.next }.cast::<NetHashEntry>();
        if entry == head {
            return Some(Match {
                slot,
                entry: core::ptr::null_mut(),
            });
        }
    }
}

/// The C `bpf_validate()` answer: invalid, valid without a match instruction,
/// or valid with the match instruction at an index.
pub(crate) enum Validated {
    Invalid,
    Plain,
    Match(usize),
}

/// The C `bpf_validate()`: check jumps, memory addresses, constant division,
/// shifts and the single match instruction of `len` instructions at `f`.
///
/// # Safety
///
/// `f` must be readable for `len` [`BpfInsn`]s.  They may be only two-byte
/// aligned, which is what the `filter_t[]` the C cast from gives.
pub(crate) unsafe fn validate(f: *const BpfInsn, len: usize) -> Validated {
    if len == 0 {
        return Validated::Invalid;
    }
    let mut match_index: Option<usize> = None;
    let mut i: usize = 1;
    while i < len {
        // SAFETY: `i < len`, so the instruction is readable.
        let p = unsafe { f.add(i).read_unaligned() };
        if (p.code & BPF_CLASS_MASK) == BPF_JMP {
            let from = i as c_int + 1;
            if (p.code & BPF_OP_MASK) == BPF_JA {
                if from + p.k >= len as c_int {
                    return Validated::Invalid;
                }
            } else if from + c_int::from(p.jt) >= len as c_int
                || from + c_int::from(p.jf) >= len as c_int
            {
                return Validated::Invalid;
            }
        }
        let memory_op = (p.code & BPF_CLASS_MASK) == BPF_ST
            || ((p.code & BPF_CLASS_MASK) == BPF_LD
                && (p.code & BPF_MODE_MASK) == BPF_MEM);
        if memory_op && (p.k >= BPF_MEMWORDS as c_int || p.k < 0) {
            return Validated::Invalid;
        }
        if (p.code == BPF_ALU_DIV_K || p.code == BPF_ALU_MOD_K) && p.k == 0 {
            return Validated::Invalid;
        }
        if p.code == BPF_ALU_LSH_K && p.k >= 32 {
            return Validated::Invalid;
        }
        if p.code == BPF_RET_MATCH_IMM {
            if match_index.is_some()
                || p.jt == 0
                || p.jt as usize > N_NET_HASH_KEYS
            {
                return Validated::Invalid;
            }
            let found = i;
            i += p.jt as usize;
            if i as c_int + 1 > len as c_int {
                return Validated::Invalid;
            }
            for j in 1..=usize::from(p.jt) {
                // SAFETY: the bound check above keeps `found + j` below
                // `len`.
                let key = unsafe { f.add(found + j).read_unaligned() };
                if key.code != BPF_MISC_KEY {
                    return Validated::Invalid;
                }
            }
            match_index = Some(found);
        }
        i += 1;
    }
    // SAFETY: `len` is at least one, so the last instruction is readable.
    let last = unsafe { f.add(len - 1).read_unaligned() };
    if (last.code & BPF_CLASS_MASK) == BPF_RET {
        match match_index {
            Some(index) => Validated::Match(index),
            None => Validated::Plain,
        }
    } else {
        Validated::Invalid
    }
}

/// The C `bpf_eq()`: whether the `len` instructions at `f1` and `f2` agree,
/// ignoring the key words of two match instructions.
///
/// # Safety
///
/// Both pointers must be readable for `len` [`BpfInsn`]s, at the same
/// two-byte alignment `validate` accepts.
pub(crate) unsafe fn eq(
    f1: *const BpfInsn,
    f2: *const BpfInsn,
    len: usize,
) -> bool {
    for i in 0..len {
        // SAFETY: the caller promises both arrays readable for `len`.
        let one = unsafe { f1.add(i).read_unaligned() };
        // SAFETY: as above.
        let two = unsafe { f2.add(i).read_unaligned() };
        let both_keys = one.code == BPF_MISC_KEY && two.code == BPF_MISC_KEY;
        if one != two && !both_keys {
            return false;
        }
    }
    true
}

/// The C's `(pc->k <= wirelen) ? pc->k : wirelen`, on the unsigned bit
/// pattern of the signed `k`.
fn accept(k: c_int, wirelen: u32) -> c_int {
    // A negative `k` becomes a huge unsigned one and loses the comparison,
    // as it did in the C.  The C returned an `int`; a wire length is a
    // packet byte count.
    (k as u32).min(wirelen) as c_int
}

/// The C `bpf_do_filter()` interpreter state.
struct Interp {
    /// `filter[0]`, the first instruction the C could jump back to.
    start: *const BpfInsn,
    /// The instruction being run, `pc` in the C.
    pc: *const BpfInsn,
    /// The C's `pc_end`, which can name a byte inside an instruction.
    end: *const BpfInsn,
    packet: *const u8,
    header: *const u8,
    hlen: u32,
    wirelen: u32,
    a: u32,
    x: u32,
    mem: [u32; BPF_MEMWORDS],
    /// Whether `infp->rcv_port` is `MACH_PORT_NULL`, the dummy hash filter.
    rcv_port_null: bool,
    /// `infp` seen as the `struct net_hash_header` a match instruction wants.
    hash: *mut NetHashHeader,
    /// The caller's `net_hash_entry_t **hash_headpp`.
    hash_headpp: *mut *mut *mut NetHashEntry,
    /// The caller's `net_hash_entry_t *entpp`.
    entpp: *mut *mut NetHashEntry,
}

impl Interp {
    /// The C's `data + k` window: an offset below `hlen` is the header's, one
    /// above it is the packet's.  `size` is the width the C added to `k`:
    /// four for a word, two for a half, and one for the byte and MSH loads,
    /// whose C comparisons are the strict `k < hlen` and `k < buflen`.
    /// A negative `k` fails both comparisons, as it did there.
    fn locate(&self, k: c_int, size: u32) -> Option<(*const u8, isize)> {
        let ku = u64::from(k as u32);
        if ku + u64::from(size) <= u64::from(self.hlen) {
            Some((self.header, k as isize))
        } else if ku + u64::from(size) <= u64::from(NET_RCV_MAX) {
            Some((self.packet, k as isize - self.hlen as isize))
        } else {
            None
        }
    }

    /// The C's `load_word:` tail.
    fn load_word(&mut self, k: c_int) -> bool {
        match self.locate(k, 4) {
            // SAFETY: `locate` keeps the read in the header window or in the
            // packet window plus its three readable leading bytes, and the
            // read is as unaligned as the C's.
            Some((data, off)) => {
                self.a = u32::from_be(unsafe {
                    data.offset(off).cast::<u32>().read_unaligned()
                });
                true
            }
            None => false,
        }
    }

    /// The C's `load_half:` tail.
    fn load_half(&mut self, k: c_int) -> bool {
        match self.locate(k, 2) {
            // SAFETY: as `load_word`, with a two-byte read.
            Some((data, off)) => {
                self.a = u32::from(u16::from_be(unsafe {
                    data.offset(off).cast::<u16>().read_unaligned()
                }));
                true
            }
            None => false,
        }
    }

    /// The C's `load_byte:` tail.  The C loaded through `char`, so a byte
    /// with the top bit set sign-extended into `A`.
    fn load_byte(&mut self, k: c_int) -> bool {
        match self.locate(k, 1) {
            // SAFETY: as `load_word`; the byte is inside the window.
            Some((data, off)) => {
                self.a =
                    i32::from(unsafe { data.offset(off).read() } as i8) as u32;
                true
            }
            None => false,
        }
    }

    /// The C's `pc += delta` followed by the loop's `++pc`, with the
    /// validated program's bounds kept even for a bad jump.
    fn jump(&mut self, delta: isize) -> bool {
        let target = self.pc.wrapping_offset(delta);
        let address = target as usize;
        if (self.start as usize) <= address && address <= (self.end as usize) {
            self.pc = target;
            true
        } else {
            false
        }
    }

    /// The C `bpf_do_filter()` instruction loop.
    ///
    /// # Safety
    ///
    /// The caller must have accepted the program with the C's `bpf_validate()`
    /// and must keep `packet`, `header` and the match outputs valid.
    unsafe fn execute(&mut self) -> c_int {
        while self.pc < self.end {
            // SAFETY: `pc` is inside the program, so the instruction is
            // readable; a two-byte alignment is read the way the C did.
            let insn = unsafe { self.pc.read_unaligned() };
            let pc = self.pc;
            // SAFETY: `pc < end`, so the result is at most `end`.
            self.pc = unsafe { pc.add(1) };
            match insn.code {
                BPF_RET_K => {
                    if self.rcv_port_null && self.entp_is_null() {
                        return 0;
                    }
                    return accept(insn.k, self.wirelen);
                }
                BPF_RET_A => {
                    if self.rcv_port_null && self.entp_is_null() {
                        return 0;
                    }
                    return accept(self.a as c_int, self.wirelen);
                }
                BPF_RET_MATCH_IMM => {
                    let n_keys = usize::from(insn.jt);
                    if n_keys == 0 || n_keys > BPF_MEMWORDS {
                        return 0;
                    }
                    // SAFETY: the caller promises a live hash header behind
                    // this port and a validated instruction.
                    if let Some(m) =
                        unsafe { find_match(self.hash, &self.mem[..n_keys]) }
                    {
                        // SAFETY: the caller promises both out-params are
                        // writable.
                        unsafe {
                            *self.hash_headpp = m.slot;
                        }
                        if m.entry.is_null() {
                            return 0;
                        }
                        // SAFETY: as above.
                        unsafe {
                            *self.entpp = m.entry;
                        }
                        return accept(insn.k, self.wirelen);
                    }
                    return 0;
                }
                BPF_LD_W_ABS => {
                    if !self.load_word(insn.k) {
                        return 0;
                    }
                }
                BPF_LD_H_ABS => {
                    if !self.load_half(insn.k) {
                        return 0;
                    }
                }
                BPF_LD_B_ABS => {
                    if !self.load_byte(insn.k) {
                        return 0;
                    }
                }
                BPF_LD_W_LEN => self.a = self.wirelen,
                BPF_LDX_W_LEN => self.x = self.wirelen,
                BPF_LD_W_IND => {
                    let k = self.x.wrapping_add(insn.k as u32) as c_int;
                    if !self.load_word(k) {
                        return 0;
                    }
                }
                BPF_LD_H_IND => {
                    let k = self.x.wrapping_add(insn.k as u32) as c_int;
                    if !self.load_half(k) {
                        return 0;
                    }
                }
                BPF_LD_B_IND => {
                    let k = self.x.wrapping_add(insn.k as u32) as c_int;
                    if !self.load_byte(k) {
                        return 0;
                    }
                }
                BPF_LDX_MSH_B => {
                    match self.locate(insn.k, 1) {
                        // SAFETY: as `load_byte`; the masked byte stays in
                        // bounds.
                        Some((data, off)) => {
                            self.x = u32::from(
                                unsafe { data.offset(off).read() } & 0xf,
                            ) << 2;
                        }
                        None => return 0,
                    }
                }
                BPF_LD_IMM => self.a = insn.k as u32,
                BPF_LDX_IMM => self.x = insn.k as u32,
                // The C's validator bounds `BPF_ST` and `BPF_LD|BPF_MEM`
                // only; `STX` and `LDX|BPF_MEM` with a negative `k` would
                // index outside `mem`, so the port rejects the index
                // instead.
                BPF_LD_MEM => match self.mem.get(insn.k as usize) {
                    Some(value) => self.a = *value,
                    None => return 0,
                },
                BPF_LDX_MEM => match self.mem.get(insn.k as usize) {
                    Some(value) => self.x = *value,
                    None => return 0,
                },
                BPF_ST => match self.mem.get_mut(insn.k as usize) {
                    Some(slot) => *slot = self.a,
                    None => return 0,
                },
                BPF_STX => match self.mem.get_mut(insn.k as usize) {
                    Some(slot) => *slot = self.x,
                    None => return 0,
                },
                BPF_JMP_JA => {
                    if !self.jump(insn.k as isize) {
                        return 0;
                    }
                }
                BPF_JMP_JGT_K => {
                    let delta = jump_delta(self.a > insn.k as u32, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JGE_K => {
                    let delta = jump_delta(self.a >= insn.k as u32, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JEQ_K => {
                    let delta = jump_delta(self.a == insn.k as u32, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JSET_K => {
                    let delta = jump_delta(self.a & insn.k as u32 != 0, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JGT_X => {
                    let delta = jump_delta(self.a > self.x, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JGE_X => {
                    let delta = jump_delta(self.a >= self.x, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JEQ_X => {
                    let delta = jump_delta(self.a == self.x, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_JMP_JSET_X => {
                    let delta = jump_delta(self.a & self.x != 0, &insn);
                    if !self.jump(delta) {
                        return 0;
                    }
                }
                BPF_ALU_ADD_X => self.a = self.a.wrapping_add(self.x),
                BPF_ALU_SUB_X => self.a = self.a.wrapping_sub(self.x),
                BPF_ALU_MUL_X => self.a = self.a.wrapping_mul(self.x),
                BPF_ALU_DIV_X => {
                    if self.x == 0 {
                        return 0;
                    }
                    self.a /= self.x;
                }
                BPF_ALU_MOD_X => {
                    if self.x == 0 {
                        return 0;
                    }
                    self.a %= self.x;
                }
                BPF_ALU_AND_X => self.a &= self.x,
                BPF_ALU_OR_X => self.a |= self.x,
                BPF_ALU_XOR_X => self.a ^= self.x,
                BPF_ALU_LSH_X => {
                    self.a = if self.x < 32 {
                        self.a.wrapping_shl(self.x)
                    } else {
                        0
                    };
                }
                BPF_ALU_RSH_X => {
                    self.a = if self.x < 32 {
                        self.a.wrapping_shr(self.x)
                    } else {
                        0
                    };
                }
                BPF_ALU_ADD_K => self.a = self.a.wrapping_add(insn.k as u32),
                BPF_ALU_SUB_K => self.a = self.a.wrapping_sub(insn.k as u32),
                BPF_ALU_MUL_K => self.a = self.a.wrapping_mul(insn.k as u32),
                BPF_ALU_DIV_K => {
                    if insn.k == 0 {
                        return 0;
                    }
                    self.a /= insn.k as u32;
                }
                BPF_ALU_MOD_K => {
                    if insn.k == 0 {
                        return 0;
                    }
                    self.a %= insn.k as u32;
                }
                BPF_ALU_AND_K => self.a &= insn.k as u32,
                BPF_ALU_OR_K => self.a |= insn.k as u32,
                BPF_ALU_XOR_K => self.a ^= insn.k as u32,
                BPF_ALU_LSH_K => {
                    // The C shifted for every `k` below 32, negative counts
                    // included; i386 masks the count, which `wrapping_shl`
                    // reproduces.
                    self.a = if insn.k < 32 {
                        self.a.wrapping_shl(insn.k as u32)
                    } else {
                        0
                    };
                }
                BPF_ALU_RSH_K => {
                    self.a = if insn.k < 32 {
                        self.a.wrapping_shr(insn.k as u32)
                    } else {
                        0
                    };
                }
                BPF_ALU_NEG => self.a = 0u32.wrapping_sub(self.a),
                BPF_MISC_TAX => self.x = self.a,
                BPF_MISC_TXA => self.a = self.x,
                _ => return 0,
            }
        }
        0
    }

    /// The C's `*entpp == 0` test.
    fn entp_is_null(&self) -> bool {
        // SAFETY: the caller promises the out-param is readable.
        unsafe { (*self.entpp).is_null() }
    }
}

/// The C's `pc->jt`/`pc->jf` selection.
fn jump_delta(taken: bool, insn: &BpfInsn) -> isize {
    if taken {
        isize::from(insn.jt)
    } else {
        isize::from(insn.jf)
    }
}

/// The C `bpf_do_filter()` body.
///
/// # Safety
///
/// `port` must point at a live `struct net_rcv_port` or `struct
/// net_hash_header` whose `filter` and `filter_end` delimit a program the
/// C's `bpf_validate()` accepted; `packet` must be readable for
/// [`NET_RCV_MAX`] bytes with the three bytes before it readable too;
/// `header` must be readable for `hlen` bytes; `hash_headpp` and `entpp`
/// must be writable, and `entpp` must be left null before the call.
pub(crate) unsafe fn do_filter(
    port: *mut NetRcvPort,
    packet: *const u8,
    wirelen: u32,
    header: *const u8,
    hlen: u32,
    hash_headpp: *mut *mut *mut NetHashEntry,
    entpp: *mut *mut NetHashEntry,
) -> c_int {
    // SAFETY: the caller promises a live port.
    let filter = unsafe { (*port).filter.as_ptr() };
    // SAFETY: as above.
    let filter_end = unsafe { (*port).filter_end };
    let bytes = (filter_end as usize).saturating_sub(filter as usize);
    // `net_set_filter()` sets `filter_end` from the filter's byte length,
    // which need not be a multiple of one instruction; the C's `pc` walks
    // bytes and stops at the address, so a program of one instruction or
    // less never runs.
    if bytes <= size_of::<BpfInsn>() {
        return 0;
    }
    // SAFETY: the caller promises a live port.
    let rcv_port_null = unsafe { (*port).rcv_port.is_null() };
    let start = filter.cast::<BpfInsn>();
    let mut interp = Interp {
        // SAFETY: `bytes` is more than one instruction, so the second
        // instruction is inside the filter array.
        pc: unsafe { start.add(1) },
        start,
        end: filter_end.cast::<BpfInsn>(),
        packet,
        header,
        hlen,
        wirelen,
        a: 0,
        x: 0,
        mem: [0; BPF_MEMWORDS],
        rcv_port_null,
        hash: port.cast::<NetHashHeader>(),
        hash_headpp,
        entpp,
    };
    // SAFETY: the caller promises a validated program and live windows.
    unsafe { interp.execute() }
}
