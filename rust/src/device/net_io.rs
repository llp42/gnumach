// SPDX-License-Identifier: CMU-Mach AND BSD-4-Clause-Shortened
// Derived from device/net_io.c:
//   Copyright (c) 1993-1989 Carnegie Mellon University.
//   The Berkeley Packet Filter section comes from the Stanford/CMU enet
//   packet filter distributed in 4.3BSD:
//   Copyright (c) 1990-1991 The Regents of the University of California.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The whole of `device/net_io.c`: the packet-filter machinery, the kmsg pool
//! the receive thread fills, the filter lists `struct ifnet` heads, and the
//! layouts their bodies read.
//!
//! The C file is gone; `net_io_ffi.rs` holds the `extern "C"` entries
//! <device/net_io.h> declares, and the four C `def_simple_lock_data(static,
//! ...)` locks are [`SimpleLock`] values here.

use crate::arch::i386::io_req::IoReq;
use crate::arch::i386::percpu::cpu_number;
use crate::device::ds_routines;
use crate::device::r#return::{DeviceError, DeviceSuccess, IoResult};
use crate::glue;
use crate::ipc::ipc_kmsg::{self, Kmsg, MsgReturn, ikm_plus_overhead};
use crate::ipc::ipc_mqueue;
use crate::ipc::ipc_port;
use crate::ipc::{IpcPort, MachMsgHeader, MachMsgType};
use crate::kern::ast::{AST_NETWORK, ast_off, ast_on};
use crate::kern::lock::SimpleLock;
use crate::kern::queue::{
    QueueEntry, enqueue_tail, queue_enter_tail, queue_init,
    queue_remove_generic,
};
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_wakeup_prim,
};
use crate::kern::slab::{self, CacheInitFlags, KmemCache};
use crate::kern::thread::{IpcKmsgQueue, Thread};
use crate::utils::byteorder::htonl;
use crate::utils::cell::SyncCell;
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use core::slice;
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

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

/* ======== the kmsg pool, the filter lists and the receive thread ======== */

/// `N_NET_HASH` of `device/net_io.c`: the filter hash headers it keeps.
const N_NET_HASH: usize = 4;
/// `NET_HDW_HDR_MAX` of <device/net_status.h>.
const NET_HDW_HDR_MAX: usize = 64;
/// The `unsigned short` words a header filter may address.
const NET_HDR_WORDS: usize = NET_HDW_HDR_MAX / 2;
/// `NET_FILTER_STACK_DEPTH` of <device/net_status.h>.
const NET_FILTER_STACK_DEPTH: usize = 32;
/// `NET_HI_PRI` of <device/net_status.h>: the priority that stops delivery.
const NET_HI_PRI: c_int = 100;
/// `NET_STATUS` of <device/net_status.h>.
const NET_STATUS: c_int = ('n' as c_int) << 16 | 1;
/// `NET_ADDRESS` of <device/net_status.h>.
const NET_ADDRESS: c_int = ('n' as c_int) << 16 | 2;
/// `NET_STATUS_COUNT` of <device/net_status.h>.
const NET_STATUS_COUNT: c_uint = 7;
/// `IFF_UP` of <device/if_hdr.h>.
const IFF_UP: c_int = 0x0001;
/// `IFF_RUNNING` of <device/if_hdr.h>.
const IFF_RUNNING: c_int = 0x0040;
/// `MACH_SEND_TIMEOUT` of <mach/message.h>.
const MACH_SEND_TIMEOUT: c_uint = 0x10;
/// `MACH_MSG_TYPE_PORT_SEND` of <mach/message.h>.
const MACH_MSG_TYPE_PORT_SEND: c_uint = 17;
/// `MACH_MSG_TYPE_BYTE` of <mach/message.h>.
const MACH_MSG_TYPE_BYTE: u32 = 9;
/// `NET_RCV_MSG_ID` of <device/net_status.h>.
const NET_RCV_MSG_ID: c_int = 2999;

/// `NETF_TYPE_MASK` of <device/net_status.h>.
const NETF_TYPE_MASK: u16 = 0xfc00;
/// `NETF_BPF` of <device/net_status.h>.
const NETF_BPF: u16 = 0x400;
/// `NETF_IN` of <device/net_status.h>.
const NETF_IN: u16 = 0x1;
/// `NETF_OUT` of <device/net_status.h>.
const NETF_OUT: u16 = 0x2;
/// `NETF_NOPUSH` of <device/net_status.h>.
const NETF_NOPUSH: u16 = 0;
/// `NETF_PUSHLIT` of <device/net_status.h>.
const NETF_PUSHLIT: u16 = 1;
/// `NETF_PUSHZERO` of <device/net_status.h>.
const NETF_PUSHZERO: u16 = 2;
/// `NETF_PUSHIND` of <device/net_status.h>.
const NETF_PUSHIND: u16 = 14;
/// `NETF_PUSHHDRIND` of <device/net_status.h>.
const NETF_PUSHHDRIND: u16 = 15;
/// `NETF_PUSHWORD` of <device/net_status.h>.
const NETF_PUSHWORD: u16 = 16;
/// `NETF_PUSHHDR` of <device/net_status.h>.
const NETF_PUSHHDR: u16 = 960;
/// `NETF_PUSHSTK` of <device/net_status.h>.
const NETF_PUSHSTK: u16 = 992;

/// The descriptor word of a `mach_msg_type_t` initializer, whose bitfield
/// layout follows the pointer width.
#[cfg(target_pointer_width = "64")]
const fn descriptor_word(name: u32, size: u32) -> u32 {
    name | (size << 8) | (1 << 29)
}

/// The descriptor word of a `mach_msg_type_t` initializer, whose bitfield
/// layout follows the pointer width.
#[cfg(target_pointer_width = "32")]
const fn descriptor_word(name: u32, size: u32) -> u32 {
    name | (size << 8) | (1 << 28)
}

/// `header_type` of `device/net_io.c`: the 64-byte hardware header.
const HEADER_TYPE: MachMsgType =
    MachMsgType::new(descriptor_word(MACH_MSG_TYPE_BYTE, 8), 64);
/// `packet_type` of `device/net_io.c`: the variable-length packet body.
const PACKET_TYPE: MachMsgType =
    MachMsgType::new(descriptor_word(MACH_MSG_TYPE_BYTE, 8), 0);

/// `struct packet_header` of <device/net_status.h>: the length and type
/// words the BPF filter window skips.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHeader {
    pub length: u16,
    pub type_: u16,
}

const _: () = assert!(size_of::<PacketHeader>() == 4);
const _: () = assert!(core::mem::align_of::<PacketHeader>() == 2);
const _: () = assert!(offset_of!(PacketHeader, length) == 0);
const _: () = assert!(offset_of!(PacketHeader, type_) == 2);

/// `struct ifqueue` of <device/if_hdr.h>: an interface's output queue.
#[repr(C)]
pub struct IfQueue {
    pub ifq_head: QueueChain,
    pub ifq_len: c_int,
    pub ifq_maxlen: c_int,
    pub ifq_drops: c_int,
    pub ifq_lock: SimpleLock,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IfQueue>() == 32);
    assert!(core::mem::align_of::<IfQueue>() == 8);
    assert!(offset_of!(IfQueue, ifq_head) == 0);
    assert!(offset_of!(IfQueue, ifq_len) == 16);
    assert!(offset_of!(IfQueue, ifq_maxlen) == 20);
    assert!(offset_of!(IfQueue, ifq_drops) == 24);
    assert!(offset_of!(IfQueue, ifq_lock) == 28);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IfQueue>() == 24);
    assert!(core::mem::align_of::<IfQueue>() == 4);
    assert!(offset_of!(IfQueue, ifq_head) == 0);
    assert!(offset_of!(IfQueue, ifq_len) == 8);
    assert!(offset_of!(IfQueue, ifq_maxlen) == 12);
    assert!(offset_of!(IfQueue, ifq_drops) == 16);
    assert!(offset_of!(IfQueue, ifq_lock) == 20);
};

/// `struct ifnet` of <device/if_hdr.h>: a network interface's header,
/// shared with the C drivers through <device/if_hdr.h>.
#[repr(C)]
pub struct IfNet {
    pub if_unit: c_short,
    pub if_flags: c_short,
    pub if_timer: c_short,
    pub if_mtu: c_short,
    pub if_header_size: c_short,
    pub if_header_format: c_short,
    pub if_address_size: c_short,
    pub if_alloc_size: c_short,
    pub if_address: *mut c_char,
    pub if_snd: IfQueue,
    pub if_rcv_port_list: QueueChain,
    pub if_snd_port_list: QueueChain,
    pub if_rcv_port_list_lock: SimpleLock,
    pub if_snd_port_list_lock: SimpleLock,
    pub if_ipackets: c_int,
    pub if_ierrors: c_int,
    pub if_opackets: c_int,
    pub if_oerrors: c_int,
    pub if_collisions: c_int,
    pub if_rcvdrops: c_int,
}

// The sizes, alignments and offsets are gdb's `ptype /o struct ifnet` over
// build-64/gnumach and build-32/gnumach.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<IfNet>() == 120);
    assert!(core::mem::align_of::<IfNet>() == 8);
    assert!(offset_of!(IfNet, if_unit) == 0);
    assert!(offset_of!(IfNet, if_flags) == 2);
    assert!(offset_of!(IfNet, if_timer) == 4);
    assert!(offset_of!(IfNet, if_mtu) == 6);
    assert!(offset_of!(IfNet, if_header_size) == 8);
    assert!(offset_of!(IfNet, if_header_format) == 10);
    assert!(offset_of!(IfNet, if_address_size) == 12);
    assert!(offset_of!(IfNet, if_alloc_size) == 14);
    assert!(offset_of!(IfNet, if_address) == 16);
    assert!(offset_of!(IfNet, if_snd) == 24);
    assert!(offset_of!(IfNet, if_rcv_port_list) == 56);
    assert!(offset_of!(IfNet, if_snd_port_list) == 72);
    assert!(offset_of!(IfNet, if_rcv_port_list_lock) == 88);
    assert!(offset_of!(IfNet, if_snd_port_list_lock) == 92);
    assert!(offset_of!(IfNet, if_ipackets) == 96);
    assert!(offset_of!(IfNet, if_ierrors) == 100);
    assert!(offset_of!(IfNet, if_opackets) == 104);
    assert!(offset_of!(IfNet, if_oerrors) == 108);
    assert!(offset_of!(IfNet, if_collisions) == 112);
    assert!(offset_of!(IfNet, if_rcvdrops) == 116);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<IfNet>() == 92);
    assert!(core::mem::align_of::<IfNet>() == 4);
    assert!(offset_of!(IfNet, if_unit) == 0);
    assert!(offset_of!(IfNet, if_flags) == 2);
    assert!(offset_of!(IfNet, if_timer) == 4);
    assert!(offset_of!(IfNet, if_mtu) == 6);
    assert!(offset_of!(IfNet, if_header_size) == 8);
    assert!(offset_of!(IfNet, if_header_format) == 10);
    assert!(offset_of!(IfNet, if_address_size) == 12);
    assert!(offset_of!(IfNet, if_alloc_size) == 14);
    assert!(offset_of!(IfNet, if_address) == 16);
    assert!(offset_of!(IfNet, if_snd) == 20);
    assert!(offset_of!(IfNet, if_rcv_port_list) == 44);
    assert!(offset_of!(IfNet, if_snd_port_list) == 52);
    assert!(offset_of!(IfNet, if_rcv_port_list_lock) == 60);
    assert!(offset_of!(IfNet, if_snd_port_list_lock) == 64);
    assert!(offset_of!(IfNet, if_ipackets) == 68);
    assert!(offset_of!(IfNet, if_ierrors) == 72);
    assert!(offset_of!(IfNet, if_opackets) == 76);
    assert!(offset_of!(IfNet, if_oerrors) == 80);
    assert!(offset_of!(IfNet, if_collisions) == 84);
    assert!(offset_of!(IfNet, if_rcvdrops) == 88);
};

/// `struct net_status` of <device/net_status.h>: the `NET_STATUS` reply.
#[repr(C)]
pub struct NetStatus {
    pub min_packet_size: c_int,
    pub max_packet_size: c_int,
    pub header_format: c_int,
    pub header_size: c_int,
    pub address_size: c_int,
    pub flags: c_int,
    pub mapped_size: c_int,
}

const _: () = assert!(size_of::<NetStatus>() == 28);
const _: () = assert!(core::mem::align_of::<NetStatus>() == 4);
const _: () = assert!(offset_of!(NetStatus, min_packet_size) == 0);
const _: () = assert!(offset_of!(NetStatus, mapped_size) == 24);

/// `struct net_rcv_msg` of <device/net_status.h>: the message a network
/// receive port gets, laid over the `struct ipc_kmsg` header.
#[cfg_attr(target_pointer_width = "64", repr(C, align(8)))]
#[cfg_attr(target_pointer_width = "32", repr(C))]
pub(crate) struct NetRcvMsg {
    pub(crate) msg_hdr: MachMsgHeader,
    pub(crate) header_type: MachMsgType,
    pub(crate) header: [c_char; NET_HDW_HDR_MAX],
    pub(crate) packet_type: MachMsgType,
    pub(crate) packet: [u8; NET_RCV_MAX as usize],
    pub(crate) sent: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NetRcvMsg>() == 4216);
    assert!(core::mem::align_of::<NetRcvMsg>() == 8);
    assert!(offset_of!(NetRcvMsg, msg_hdr) == 0);
    assert!(offset_of!(NetRcvMsg, header_type) == 32);
    assert!(offset_of!(NetRcvMsg, header) == 40);
    assert!(offset_of!(NetRcvMsg, packet_type) == 104);
    assert!(offset_of!(NetRcvMsg, packet) == 112);
    assert!(offset_of!(NetRcvMsg, sent) == 4208);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NetRcvMsg>() == 4196);
    assert!(core::mem::align_of::<NetRcvMsg>() == 4);
    assert!(offset_of!(NetRcvMsg, msg_hdr) == 0);
    assert!(offset_of!(NetRcvMsg, header_type) == 24);
    assert!(offset_of!(NetRcvMsg, header) == 28);
    assert!(offset_of!(NetRcvMsg, packet_type) == 92);
    assert!(offset_of!(NetRcvMsg, packet) == 96);
    assert!(offset_of!(NetRcvMsg, sent) == 4192);
};

/// `net_queue_lock` of `device/net_io.c`: the high and low send queues and
/// `net_thread_awake`.
static NET_QUEUE_LOCK: SimpleLock = SimpleLock::new();
/// `net_queue_free_lock`: the free kmsg pool.
static NET_QUEUE_FREE_LOCK: SimpleLock = SimpleLock::new();
/// `net_kmsg_total_lock`: the allocation counters.
static NET_KMSG_TOTAL_LOCK: SimpleLock = SimpleLock::new();
/// `net_hash_header_lock`: picks a free `filter_hash_header[]` slot.
static NET_HASH_HEADER_LOCK: SimpleLock = SimpleLock::new();

/// `net_thread_awake`, under [`NET_QUEUE_LOCK`].
static NET_THREAD_AWAKE: SyncCell<bool> = SyncCell(UnsafeCell::new(false));
/// `net_queue_high`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_HIGH: SyncCell<IpcKmsgQueue> =
    SyncCell(UnsafeCell::new(IpcKmsgQueue {
        base: ptr::null_mut(),
    }));
/// `net_queue_high_size`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_HIGH_SIZE: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_high_max`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_HIGH_MAX: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_low`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_LOW: SyncCell<IpcKmsgQueue> =
    SyncCell(UnsafeCell::new(IpcKmsgQueue {
        base: ptr::null_mut(),
    }));
/// `net_queue_low_size`: `net_kmsg_want_more` reads it without the queue
/// lock, so it is an atomic; the queue lock still serializes its updates.
static NET_QUEUE_LOW_SIZE: AtomicI32 = AtomicI32::new(0);
/// `net_queue_low_max`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_LOW_MAX: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_free`, under [`NET_QUEUE_FREE_LOCK`].
static NET_QUEUE_FREE: SyncCell<IpcKmsgQueue> =
    SyncCell(UnsafeCell::new(IpcKmsgQueue {
        base: ptr::null_mut(),
    }));
/// `net_queue_free_size`: `net_kmsg_want_more` reads it without the free
/// lock, so it is an atomic; the free lock still serializes its updates.
static NET_QUEUE_FREE_SIZE: AtomicI32 = AtomicI32::new(0);
/// `net_queue_free_max`, under [`NET_QUEUE_FREE_LOCK`].
static NET_QUEUE_FREE_MAX: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_free_min`: how many free buffers to keep.  `net_kmsg_want_more`
/// reads it without a lock, so it is an atomic.
static NET_QUEUE_FREE_MIN: AtomicI32 = AtomicI32::new(3);
/// `net_queue_free_hits`, under [`NET_QUEUE_FREE_LOCK`].
static NET_QUEUE_FREE_HITS: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_free_steals`, under [`NET_QUEUE_LOCK`].
static NET_QUEUE_FREE_STEALS: SyncCell<c_int> = SyncCell(UnsafeCell::new(0));
/// `net_queue_free_misses`: a debug counter the C incremented without a lock.
static NET_QUEUE_FREE_MISSES: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_send_high_hits`: a debug counter the C incremented unlocked.
static NET_KMSG_SEND_HIGH_HITS: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_send_low_hits`: a debug counter the C incremented unlocked.
static NET_KMSG_SEND_LOW_HITS: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_send_high_misses`: a debug counter the C touched unlocked.
static NET_KMSG_SEND_HIGH_MISSES: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_send_low_misses`: a debug counter the C touched unlocked.
static NET_KMSG_SEND_LOW_MISSES: AtomicI32 = AtomicI32::new(0);
/// `net_thread_awaken`: a debug counter the C incremented unlocked.
static NET_THREAD_AWAKEN: AtomicI32 = AtomicI32::new(0);
/// `net_ast_taken`: a debug counter the C incremented unlocked.
static NET_AST_TAKEN: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_total`: how many network messages exist.  `net_kmsg_want_more`
/// reads it without the total lock, so it is an atomic.
static NET_KMSG_TOTAL: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_max`: the allocation cap, read by `net_kmsg_want_more` too.
static NET_KMSG_MAX: AtomicI32 = AtomicI32::new(0);
/// `net_kmsg_size`: the allocation size, written once by `net_io_init()`.
static NET_KMSG_SIZE: AtomicUsize = AtomicUsize::new(0);
/// `net_filter_queue_reorder`: non-zero to enable queue reordering.  The C
/// left it a global for debuggers; the symbol is kept.
#[unsafe(export_name = "net_filter_queue_reorder")]
static NET_FILTER_QUEUE_REORDER: AtomicI32 = AtomicI32::new(0);

/// `net_rcv_cache` of `device/net_io.c`.
static NET_RCV_CACHE: SyncCell<KmemCache> =
    SyncCell(UnsafeCell::new(KmemCache::zeroed()));
/// `net_hash_entry_cache` of `device/net_io.c`.
static NET_HASH_ENTRY_CACHE: SyncCell<KmemCache> =
    SyncCell(UnsafeCell::new(KmemCache::zeroed()));
/// `filter_hash_header` of `device/net_io.c`: the filter groups whose BPF
/// program carries a match instruction.  [`NET_HASH_HEADER_LOCK`] chooses a
/// slot; the `struct ifnet` port-list locks serialize its fields.
static FILTER_HASH_HEADER: SyncCell<[NetHashHeader; N_NET_HASH]> =
    SyncCell(UnsafeCell::new(unsafe { core::mem::zeroed() }));

/// `net_rcv_cache`'s address, for the cache calls that take `&mut self`.
fn rcv_cache() -> *mut KmemCache {
    NET_RCV_CACHE.0.get()
}

/// `net_hash_entry_cache`'s address.
fn hash_entry_cache() -> *mut KmemCache {
    NET_HASH_ENTRY_CACHE.0.get()
}

/// `filter_hash_header[index]`'s address.
fn hash_header_slot(index: usize) -> *mut NetHashHeader {
    FILTER_HASH_HEADER
        .0
        .get()
        .cast::<NetHashHeader>()
        .wrapping_add(index)
}

/// `net_queue_high`'s address.
fn queue_high() -> *mut IpcKmsgQueue {
    NET_QUEUE_HIGH.0.get()
}

/// `net_queue_low`'s address.
fn queue_low() -> *mut IpcKmsgQueue {
    NET_QUEUE_LOW.0.get()
}

/// `net_queue_free`'s address.
fn queue_free() -> *mut IpcKmsgQueue {
    NET_QUEUE_FREE.0.get()
}

/// The `if_rcv_port_list` or `if_snd_port_list` head a direction uses.
unsafe fn port_list(ifp: *mut IfNet, sent: bool) -> *mut QueueChain {
    if sent {
        // SAFETY: the caller promises a live interface.
        unsafe { addr_of_mut!((*ifp).if_snd_port_list) }
    } else {
        // SAFETY: as above.
        unsafe { addr_of_mut!((*ifp).if_rcv_port_list) }
    }
}

/// A receive port's chain in a direction: `input` or `output`.
unsafe fn port_chain(port: *mut NetRcvPort, sent: bool) -> *mut QueueChain {
    if sent {
        // SAFETY: the caller promises a live receive port.
        unsafe { addr_of_mut!((*port).output) }
    } else {
        // SAFETY: as above.
        unsafe { addr_of_mut!((*port).input) }
    }
}

/// The hash header sharing a receive port's storage.
unsafe fn port_hash_header(port: *mut NetRcvPort) -> *mut NetHashHeader {
    port.cast()
}

/// `net_kmsg(kmsg)` of <device/net_io.h>: the receive message over the
/// kmsg's header.
unsafe fn net_kmsg(kmsg: Kmsg) -> *mut NetRcvMsg {
    // SAFETY: the caller promises a live message.
    unsafe { kmsg.header().cast() }
}

/// `P2ROUND()` of <kern/macros.h>: round `x` up to a multiple of the power
/// of two `align`.
const fn p2round(x: usize, align: usize) -> usize {
    (x + (align - 1)) & !(align - 1)
}

/// The `net_kmsg_want_more()` macro of `device/net_io.c`.  The C comment
/// says a misread value is not critical, so the loads are `Relaxed`.
fn want_more() -> bool {
    let free = NET_QUEUE_FREE_SIZE.load(Ordering::Relaxed);
    let low = NET_QUEUE_LOW_SIZE.load(Ordering::Relaxed);
    let min = NET_QUEUE_FREE_MIN.load(Ordering::Relaxed);
    let total = NET_KMSG_TOTAL.load(Ordering::Relaxed);
    let max = NET_KMSG_MAX.load(Ordering::Relaxed);
    free.wrapping_add(low) < min && total < max
}

/// `net_kmsg_alloc()` of <device/net_io.h>.
unsafe fn kmsg_alloc() -> *mut c_void {
    let size = NET_KMSG_SIZE.load(Ordering::Relaxed);
    slab::kalloc(size).map_or(ptr::null_mut(), |buf| buf.as_ptr().cast())
}

/// `net_kmsg_free()` of <device/net_io.h>.
unsafe fn kmsg_free(kmsg: *mut c_void) {
    let Some(buf) = NonNull::new(kmsg.cast::<u8>()) else {
        return;
    };
    // SAFETY: the caller promises an allocation this call owns.
    unsafe { slab::kfree(buf, NET_KMSG_SIZE.load(Ordering::Relaxed)) };
}

/// `net_kmsg_get()` of <device/net_io.h>.
pub(crate) unsafe fn kmsg_get() -> Option<Kmsg> {
    let s = unsafe { glue::splimp() };

    NET_QUEUE_FREE_LOCK.lock();
    // SAFETY: the caller holds the free lock, which serializes the queue.
    let mut kmsg = unsafe { ipc_kmsg::dequeue(queue_free()) };
    if kmsg.is_some() {
        NET_QUEUE_FREE_SIZE.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: the free lock serializes this counter.
        unsafe { *NET_QUEUE_FREE_HITS.0.get() += 1 };
    }
    NET_QUEUE_FREE_LOCK.unlock();

    if kmsg.is_none() {
        NET_QUEUE_LOCK.lock();
        // SAFETY: the caller holds the queue lock, which serializes the low
        // queue.
        kmsg = unsafe { ipc_kmsg::dequeue(queue_low()) };
        if kmsg.is_some() {
            NET_QUEUE_LOW_SIZE.fetch_sub(1, Ordering::Relaxed);
            // SAFETY: the queue lock serializes this counter.
            unsafe { *NET_QUEUE_FREE_STEALS.0.get() += 1 };
        }
        NET_QUEUE_LOCK.unlock();
    }

    if kmsg.is_none() {
        NET_QUEUE_FREE_MISSES.fetch_add(1, Ordering::Relaxed);
    }
    let _ = unsafe { glue::splx(s) };

    if want_more() || kmsg.is_none() {
        let s = unsafe { glue::splimp() };
        NET_QUEUE_LOCK.lock();
        // SAFETY: the queue lock serializes the flag.
        let awake = unsafe { *NET_THREAD_AWAKE.0.get() };
        // SAFETY: as above.
        unsafe { *NET_THREAD_AWAKE.0.get() = true };
        NET_QUEUE_LOCK.unlock();
        let _ = unsafe { glue::splx(s) };

        if !awake {
            // SAFETY: the caller is at a level where the wakeup cannot be
            // lost, and the address is the live flag's.
            unsafe {
                thread_wakeup_prim(
                    NET_THREAD_AWAKE.0.get().cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                )
            };
        }
    }

    kmsg
}

/// `net_kmsg_put()` of <device/net_io.h>.
pub(crate) unsafe fn kmsg_put(kmsg: *mut c_void) {
    let Some(kmsg) = NonNull::new(kmsg) else {
        return;
    };

    let s = unsafe { glue::splimp() };
    NET_QUEUE_FREE_LOCK.lock();
    // SAFETY: the caller owns the message, which is not queued.
    unsafe { ipc_kmsg::enqueue(queue_free(), Kmsg::from_raw(kmsg.as_ptr())) };
    let size = NET_QUEUE_FREE_SIZE
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    // SAFETY: the free lock serializes the maximum.
    let max = unsafe { &mut *NET_QUEUE_FREE_MAX.0.get() };
    if size > *max {
        *max = size;
    }
    NET_QUEUE_FREE_LOCK.unlock();
    let _ = unsafe { glue::splx(s) };
}

/// `net_kmsg_collect()` of <device/net_io.h>.
pub(crate) unsafe fn kmsg_collect() {
    let mut s = unsafe { glue::splimp() };
    NET_QUEUE_FREE_LOCK.lock();
    while NET_QUEUE_FREE_SIZE.load(Ordering::Relaxed)
        > NET_QUEUE_FREE_MIN.load(Ordering::Relaxed)
    {
        // SAFETY: the free lock serializes the queue.
        let kmsg = unsafe { ipc_kmsg::dequeue(queue_free()) };
        NET_QUEUE_FREE_SIZE.fetch_sub(1, Ordering::Relaxed);
        NET_QUEUE_FREE_LOCK.unlock();
        let _ = unsafe { glue::splx(s) };

        if let Some(kmsg) = kmsg {
            unsafe { kmsg_free(kmsg.as_ptr()) };
            NET_KMSG_TOTAL_LOCK.lock();
            NET_KMSG_TOTAL.fetch_sub(1, Ordering::Relaxed);
            NET_KMSG_TOTAL_LOCK.unlock();
        }

        s = unsafe { glue::splimp() };
        NET_QUEUE_FREE_LOCK.lock();
    }
    NET_QUEUE_FREE_LOCK.unlock();
    let _ = unsafe { glue::splx(s) };
}

/// `net_kmsg_more()` of `device/net_io.c`.
unsafe fn kmsg_more() {
    while want_more() {
        NET_KMSG_TOTAL_LOCK.lock();
        NET_KMSG_TOTAL.fetch_add(1, Ordering::Relaxed);
        NET_KMSG_TOTAL_LOCK.unlock();

        // SAFETY: the allocation is a raw pool buffer.
        let kmsg = unsafe { kmsg_alloc() };
        if kmsg.is_null() {
            // The C enqueued the null and dereferenced it; stopping keeps
            // the pool consistent instead of corrupting it.
            break;
        }
        // SAFETY: the fresh allocation is unowned.
        unsafe { kmsg_put(kmsg) };
    }
}

/// `net_deliver()` of `device/net_io.c`.
///
/// # Safety
///
/// Called holding [`NET_QUEUE_LOCK`] at splimp; it returns holding the lock.
unsafe fn deliver(nonblocking: bool) -> bool {
    let kmsg;
    let high_priority;
    // SAFETY: the caller holds the queue lock, which serializes both queues.
    match unsafe { ipc_kmsg::dequeue(queue_high()) } {
        Some(first) => {
            // SAFETY: the queue lock serializes the size.
            unsafe { *NET_QUEUE_HIGH_SIZE.0.get() -= 1 };
            kmsg = first;
            high_priority = true;
        }
        None => {
            // SAFETY: as above.
            match unsafe { ipc_kmsg::dequeue(queue_low()) } {
                Some(first) => {
                    NET_QUEUE_LOW_SIZE.fetch_sub(1, Ordering::Relaxed);
                    kmsg = first;
                    high_priority = false;
                }
                None => return false,
            }
        }
    }
    NET_QUEUE_LOCK.unlock();
    let _ = unsafe { glue::spl0() };

    let mut send_list = IpcKmsgQueue {
        base: ptr::null_mut(),
    };
    // SAFETY: the message is live and holds the interface pointer, the list
    // is an empty local queue, and only [`NET_QUEUE_LOCK`] is held.
    unsafe { filter(kmsg, &mut send_list) };

    if !nonblocking {
        // SAFETY: the queue lock is not held here.
        unsafe { kmsg_more() };
    }

    // SAFETY: the list holds messages this call owns, not queued elsewhere.
    while let Some(queued) = unsafe { ipc_kmsg::dequeue(&mut send_list) } {
        // SAFETY: the message is live and held by the list.
        let count = unsafe { (*net_kmsg(queued)).packet_type.number() };
        // SAFETY: the message is live and this call owns it.
        unsafe { queued.init_network() };
        // SAFETY: the message is live and this call owns it.
        let header = unsafe { queued.header() };
        let size = p2round(
            size_of::<NetRcvMsg>() - size_of::<c_int>() - NET_RCV_MAX as usize
                + count as usize,
            size_of::<usize>(),
        );
        // SAFETY: the header is live and this call owns the message.
        unsafe {
            (*header).set_bits(MACH_MSG_TYPE_PORT_SEND);
            (*header).set_size(u32::try_from(size).unwrap_or(u32::MAX));
            (*header).set_local(0);
            (*header).set_id(NET_RCV_MSG_ID);
            queued.set_header_seqno(0);

            let msg = net_kmsg(queued);
            (*msg).header_type = HEADER_TYPE;
            (*msg).packet_type = MachMsgType::new(PACKET_TYPE.word(), count);
        }

        // SAFETY: the message is live and holds the destination right.
        match unsafe {
            ipc_mqueue::send(queued.as_ptr(), MACH_SEND_TIMEOUT, 0)
        } {
            MsgReturn::SUCCESS => {
                let counter = if high_priority {
                    &NET_KMSG_SEND_HIGH_HITS
                } else {
                    &NET_KMSG_SEND_LOW_HITS
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                let counter = if high_priority {
                    &NET_KMSG_SEND_HIGH_MISSES
                } else {
                    &NET_KMSG_SEND_LOW_MISSES
                };
                counter.fetch_add(1, Ordering::Relaxed);
                // SAFETY: the send failed, so this call still owns the
                // message.
                unsafe { ipc_kmsg::destroy(queued) };
            }
        }
    }

    let _ = unsafe { glue::splimp() };
    NET_QUEUE_LOCK.lock();
    true
}

/// `net_ast()` of `device/net_io.c`.
pub(crate) unsafe fn ast() {
    NET_AST_TAKEN.fetch_add(1, Ordering::Relaxed);

    let s = unsafe { glue::splimp() };
    NET_QUEUE_LOCK.lock();
    // SAFETY: the lock is held, and `deliver` returns holding it.
    while unsafe { !*NET_THREAD_AWAKE.0.get() && deliver(true) } {}
    NET_QUEUE_LOCK.unlock();
    let _ = unsafe { glue::splsched() };
    ast_off(cpu_number(), AST_NETWORK);
    let _ = unsafe { glue::splx(s) };
}

/// `net_thread_continue()` of `device/net_io.c`, the receive thread body,
/// which never returns.
unsafe fn thread_continue_inner() -> ! {
    loop {
        NET_THREAD_AWAKEN.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the receive thread may allocate.
        unsafe { kmsg_more() };

        let s = unsafe { glue::splimp() };
        NET_QUEUE_LOCK.lock();
        // SAFETY: the lock is held, and `deliver` returns holding it.
        while unsafe { deliver(false) } {}
        // SAFETY: the queue lock serializes the flag.
        unsafe { *NET_THREAD_AWAKE.0.get() = false };
        // SAFETY: the thread is not waiting on an event yet.
        unsafe { assert_wait(NET_THREAD_AWAKE.0.get().cast::<c_void>(), 0) };
        NET_QUEUE_LOCK.unlock();
        let _ = unsafe { glue::splx(s) };

        // SAFETY: the current thread holds no spin lock and has set its wait
        // state.
        unsafe { thread_block(Some(thread_continue)) };
    }
}

/// The continuation form of `net_thread_continue()` `thread_block()` takes.
unsafe extern "C" fn thread_continue() {
    // SAFETY: the continuation re-enters the loop, which never returns.
    unsafe { thread_continue_inner() };
}

/// `net_thread()` of `device/net_io.c`.
pub(crate) unsafe fn thread() -> ! {
    // SAFETY: this is the current thread's own entry, and it holds no thread
    // lock.
    unsafe { Thread::set_own_priority(0) };

    let s = unsafe { glue::splimp() };
    NET_QUEUE_LOCK.lock();
    // SAFETY: the queue lock serializes the flag.
    unsafe { *NET_THREAD_AWAKE.0.get() = false };
    // SAFETY: the thread is not waiting on an event yet.
    unsafe { assert_wait(NET_THREAD_AWAKE.0.get().cast::<c_void>(), 0) };
    NET_QUEUE_LOCK.unlock();
    let _ = unsafe { glue::splx(s) };

    // SAFETY: the current thread holds no spin lock and has set its wait
    // state.
    unsafe { thread_block(Some(thread_continue)) };
    // SAFETY: the receive loop re-enters and never returns.
    unsafe { thread_continue_inner() }
}

/// The `NETF_OP(NETF_*)` value of each operator in <device/net_status.h>.
const NETF_OP_NOP: u32 = 0;
const NETF_OP_EQ: u32 = 1;
const NETF_OP_LT: u32 = 2;
const NETF_OP_LE: u32 = 3;
const NETF_OP_GT: u32 = 4;
const NETF_OP_GE: u32 = 5;
const NETF_OP_AND: u32 = 6;
const NETF_OP_OR: u32 = 7;
const NETF_OP_XOR: u32 = 8;
const NETF_OP_COR: u32 = 9;
const NETF_OP_CAND: u32 = 10;
const NETF_OP_CNOR: u32 = 11;
const NETF_OP_CNAND: u32 = 12;
const NETF_OP_NEQ: u32 = 13;
const NETF_OP_LSH: u32 = 14;
const NETF_OP_RSH: u32 = 15;
const NETF_OP_ADD: u32 = 16;
const NETF_OP_SUB: u32 = 17;

/// `net_do_filter()` of `device/net_io.c`: run the old `filter_t` program.
///
/// # Safety
///
/// `infp` must point at a live receive port whose filter `net_set_filter()`
/// accepted, and `data`/`header` must be readable for the words the program
/// addresses.
pub(crate) unsafe fn net_do_filter(
    infp: *mut NetRcvPort,
    data: *const u8,
    data_count: c_uint,
    header: *const u8,
) -> bool {
    let mut stack = [0u32; NET_FILTER_STACK_DEPTH + 1];
    let mut sp = NET_FILTER_STACK_DEPTH;
    stack[sp] = 1;

    let words = (data_count / (size_of::<u16>() as c_uint)) as usize;
    // SAFETY: the caller promises a live port.
    let mut fp = unsafe { (*infp).filter.as_ptr().add(1) };
    // SAFETY: as above.
    let fpe = unsafe { (*infp).filter_end };

    while fp < fpe {
        // SAFETY: `fp < fpe` keeps the read inside the filter array.
        let word = unsafe { *fp };
        fp = unsafe { fp.add(1) };
        let op = u32::from((word >> 10) & 0x3f);
        let raw = word & 0x3ff;
        let mut arg = u32::from(raw);

        match raw {
            NETF_NOPUSH => {
                let Some(&top) = stack.get(sp) else {
                    return false;
                };
                arg = top;
                sp += 1;
            }
            NETF_PUSHZERO => arg = 0,
            NETF_PUSHLIT => {
                if fp >= fpe {
                    return false;
                }
                // SAFETY: `fp < fpe`, so the literal is in the array.
                arg = u32::from(unsafe { *fp });
                fp = unsafe { fp.add(1) };
            }
            NETF_PUSHIND => {
                let Some(&top) = stack.get(sp) else {
                    return false;
                };
                sp += 1;
                if top >= words as u32 {
                    return false;
                }
                // SAFETY: the index is below the `data` word count.
                arg = u32::from(unsafe {
                    data.cast::<u16>().add(top as usize).read_unaligned()
                });
            }
            NETF_PUSHHDRIND => {
                let Some(&top) = stack.get(sp) else {
                    return false;
                };
                sp += 1;
                if top >= NET_HDR_WORDS as u32 {
                    return false;
                }
                // SAFETY: the index is below the header's word count.
                arg = u32::from(unsafe {
                    header.cast::<u16>().add(top as usize).read_unaligned()
                });
            }
            _ => {
                if arg >= u32::from(NETF_PUSHSTK) {
                    let index = (arg - u32::from(NETF_PUSHSTK)) as usize;
                    let Some(&value) = stack.get(sp + index) else {
                        return false;
                    };
                    arg = value;
                } else if arg >= u32::from(NETF_PUSHHDR) {
                    let index = (arg - u32::from(NETF_PUSHHDR)) as usize;
                    // SAFETY: the index is below the header's word count.
                    arg = u32::from(unsafe {
                        header.cast::<u16>().add(index).read_unaligned()
                    });
                } else {
                    let index =
                        arg.wrapping_sub(u32::from(NETF_PUSHWORD)) as usize;
                    if index >= words {
                        return false;
                    }
                    // SAFETY: the index is below the `data` word count.
                    arg = u32::from(unsafe {
                        data.cast::<u16>().add(index).read_unaligned()
                    });
                }
            }
        }

        if op == NETF_OP_NOP {
            sp -= 1;
            let Some(slot) = stack.get_mut(sp) else {
                return false;
            };
            *slot = arg;
            continue;
        }
        let Some(top) = stack.get_mut(sp) else {
            return false;
        };
        match op {
            NETF_OP_AND => *top &= arg,
            NETF_OP_OR => *top |= arg,
            NETF_OP_XOR => *top ^= arg,
            NETF_OP_EQ => *top = u32::from(*top == arg),
            NETF_OP_NEQ => *top = u32::from(*top != arg),
            NETF_OP_LT => *top = u32::from(*top < arg),
            NETF_OP_LE => *top = u32::from(*top <= arg),
            NETF_OP_GT => *top = u32::from(*top > arg),
            NETF_OP_GE => *top = u32::from(*top >= arg),
            NETF_OP_COR => {
                let value = *top;
                sp += 1;
                if value == arg {
                    return true;
                }
            }
            NETF_OP_CAND => {
                let value = *top;
                sp += 1;
                if value != arg {
                    return false;
                }
            }
            NETF_OP_CNOR => {
                let value = *top;
                sp += 1;
                if value == arg {
                    return false;
                }
            }
            NETF_OP_CNAND => {
                let value = *top;
                sp += 1;
                if value != arg {
                    return true;
                }
            }
            // The C shifted an `int` by the argument; the machine masks the
            // count, which is what `wrapping_shl`/`wrapping_shr` reproduce.
            NETF_OP_LSH => *top = top.wrapping_shl(arg),
            // The C shifted a signed `int`; the machine shifts the sign bit
            // in, which the `i32` round trip reproduces.
            NETF_OP_RSH => *top = (*top as i32).wrapping_shr(arg) as u32,
            NETF_OP_ADD => *top = top.wrapping_add(arg),
            NETF_OP_SUB => *top = top.wrapping_sub(arg),
            _ => (),
        }
    }
    stack.get(sp).is_some_and(|top| *top != 0)
}

/// `parse_net_filter()` of `device/net_io.c`: check the program's
/// operations and its stack use.
fn parse_filter(filter: &[u16]) -> bool {
    let depth = NET_FILTER_STACK_DEPTH as i32;
    let mut sp = depth;
    let mut i = 1usize;

    while i < filter.len() {
        let word = filter[i];
        let op = u32::from((word >> 10) & 0x3f);
        let arg = word & 0x3ff;
        i += 1;

        match arg {
            NETF_NOPUSH => (),
            NETF_PUSHZERO => sp -= 1,
            NETF_PUSHLIT => {
                if i >= filter.len() {
                    return false;
                }
                i += 1;
                sp -= 1;
            }
            NETF_PUSHIND | NETF_PUSHHDRIND => (),
            _ => {
                if arg >= NETF_PUSHSTK {
                    let index = i32::from(arg - NETF_PUSHSTK);
                    if index + sp > depth {
                        return false;
                    }
                } else if arg >= NETF_PUSHHDR
                    && usize::from(arg - NETF_PUSHHDR) >= NET_HDR_WORDS
                {
                    return false;
                }
                sp -= 1;
            }
        }
        if sp < 2 {
            return false;
        }
        if op == NETF_OP_NOP {
            continue;
        }
        if sp > (NET_MAX_FILTER - 2) as i32 {
            return false;
        }
        sp += 1;
        if !matches!(
            op,
            NETF_OP_EQ
                | NETF_OP_LT
                | NETF_OP_LE
                | NETF_OP_GT
                | NETF_OP_GE
                | NETF_OP_AND
                | NETF_OP_OR
                | NETF_OP_XOR
                | NETF_OP_COR
                | NETF_OP_CAND
                | NETF_OP_CNOR
                | NETF_OP_CNAND
                | NETF_OP_NEQ
                | NETF_OP_LSH
                | NETF_OP_RSH
                | NETF_OP_ADD
                | NETF_OP_SUB
        ) {
            return false;
        }
    }
    true
}

/// `reorder_queue()` of `device/net_io.c`: move `last` directly after
/// `first` in their list.
///
/// # Safety
///
/// `first` and `last` must be linked into one initialized list.
unsafe fn reorder_queue(first: *mut QueueChain, last: *mut QueueChain) {
    // SAFETY: the caller promises both are linked into one list.
    unsafe {
        let prev = (*first).prev;
        let next = (*last).next;

        (*prev).next = last;
        (*next).prev = first;

        (*last).prev = prev;
        (*last).next = first;

        (*first).next = next;
        (*first).prev = last;
    }
}

/// The C's `REORDER_PRIO()` macro: promote `port` ahead of an equal-priority
/// predecessor whose count lags by more than the threshold.
///
/// # Safety
///
/// `port` must be a live receive port linked into the list `sent` names.
unsafe fn reorder_prio(
    ifp: *mut IfNet,
    port: *mut NetRcvPort,
    sent: bool,
    rcount: c_int,
) {
    let flag = if sent { NETF_OUT } else { NETF_IN };
    // SAFETY: the caller promises a live port.
    if unsafe { (*port).filter[0] } & flag == 0 {
        return;
    }
    let list = unsafe { port_list(ifp, sent) };
    let chain = unsafe { port_chain(port, sent) };
    // SAFETY: the port is linked into this list, so its predecessor is the
    // list head or another port.
    let prevfp = unsafe { (*chain).prev.cast::<NetRcvPort>() };
    if prevfp.cast::<QueueChain>() == list {
        return;
    }
    // SAFETY: the predecessor is a live port, as above.
    let equal = unsafe { (*port).priority == (*prevfp).priority };
    let behind = NET_FILTER_QUEUE_REORDER.load(Ordering::Relaxed) != 0
        && 100i32.wrapping_add(unsafe { (*prevfp).rcv_count }) < rcount;
    if equal && behind {
        // SAFETY: both chains belong to this list, and the port is linked.
        unsafe { reorder_queue(port_chain(prevfp, sent), chain) };
    }
}

/// `net_filter()` of `device/net_io.c`: run `kmsg` through the interface's
/// filters and queue a copy per matching receive port.
///
/// # Safety
///
/// `kmsg` must be a live message holding the interface pointer in its remote
/// port and a receive message header; `send_list` must be an empty queue the
/// caller owns; no interface lock may be held.
pub(crate) unsafe fn filter(kmsg: Kmsg, send_list: *mut IpcKmsgQueue) {
    // SAFETY: the caller promises the receive message.
    let count = unsafe { (*net_kmsg(kmsg)).packet_type.number() as c_int };
    // SAFETY: the sender stored the interface pointer in the remote port.
    let ifp = unsafe { kmsg.remote_port() as *mut IfNet };
    // SAFETY: the caller promises an empty queue.
    unsafe { (*send_list).base = ptr::null_mut() };

    // SAFETY: the receive message is live.
    let sent = unsafe { (*net_kmsg(kmsg)).sent != 0 };
    let list = unsafe { port_list(ifp, sent) };

    let mut dead_infp: *mut QueueChain = ptr::null_mut();
    let mut dead_entp: *mut QueueChain = ptr::null_mut();

    // SAFETY: the caller promises both interface locks free.
    unsafe {
        (*ifp).if_rcv_port_list_lock.lock();
        (*ifp).if_snd_port_list_lock.lock();
    }

    // SAFETY: the list head is initialized and its links are containers.
    let mut infp = unsafe { (*list).next.cast::<NetRcvPort>() };
    while infp.cast::<QueueChain>() != list {
        let chain = unsafe { port_chain(infp, sent) };
        // The body can unlink `infp`, so the next port is read first.
        let nextfp = unsafe { (*chain).next.cast::<NetRcvPort>() };

        let mut entp: *mut NetHashEntry = ptr::null_mut();
        let mut hash_headp: *mut *mut NetHashEntry = ptr::null_mut();
        let (ret_count, dest) =
            if unsafe { (*infp).filter[0] & NETF_TYPE_MASK } == NETF_BPF {
                let wirelen = count
                    .wrapping_sub(size_of::<PacketHeader>() as c_int)
                    as c_uint;
                // SAFETY: the caller promises the message and the interface.
                let ret = unsafe {
                    do_filter(
                        infp,
                        (*net_kmsg(kmsg))
                            .packet
                            .as_ptr()
                            .add(size_of::<PacketHeader>()),
                        wirelen,
                        (*net_kmsg(kmsg)).header.as_ptr().cast::<u8>(),
                        (*ifp).if_header_size as c_int as c_uint,
                        &mut hash_headp,
                        &mut entp,
                    )
                };
                let count = if ret != 0 {
                    ret as c_uint + size_of::<PacketHeader>() as c_uint
                } else {
                    0
                };
                let dest = if entp.is_null() {
                    unsafe { (*infp).rcv_port }
                } else {
                    // SAFETY: a match instruction selected this live entry.
                    unsafe { (*entp).rcv_port }
                };
                (count, dest)
            } else {
                // SAFETY: the caller promises the message and the port.
                let hit = unsafe {
                    net_do_filter(
                        infp,
                        (*net_kmsg(kmsg)).packet.as_ptr(),
                        count as c_uint,
                        (*net_kmsg(kmsg)).header.as_ptr().cast::<u8>(),
                    )
                };
                let count = if hit { count as c_uint } else { 0 };
                // SAFETY: the caller promises a live port.
                (count, unsafe { (*infp).rcv_port })
            };

        if ret_count != 0 {
            // SAFETY: the filter selected `dest`, a port the filter owns a
            // right to.
            let dest = unsafe { ipc_port::copy_send(dest) };
            if IpcPort::valid(dest).is_none() {
                if entp.is_null() {
                    if unsafe { (*infp).filter[0] } & NETF_IN != 0 {
                        // SAFETY: the port is linked into the receive list.
                        unsafe {
                            queue_remove_generic(
                                addr_of_mut!((*ifp).if_rcv_port_list).cast(),
                                infp.cast(),
                                offset_of!(NetRcvPort, input),
                            );
                        }
                    }
                    if unsafe { (*infp).filter[0] } & NETF_OUT != 0 {
                        // SAFETY: the port is linked into the send list.
                        unsafe {
                            queue_remove_generic(
                                addr_of_mut!((*ifp).if_snd_port_list).cast(),
                                infp.cast(),
                                offset_of!(NetRcvPort, output),
                            );
                        }
                    }
                    // SAFETY: the port is off both lists and reuses `input`
                    // as the dead list's link.
                    unsafe { (*infp).input.next = dead_infp };
                    dead_infp = infp.cast();
                } else {
                    // SAFETY: the entry is linked into a live hash bucket.
                    unsafe {
                        hash_ent_remove(
                            ifp,
                            port_hash_header(infp),
                            false,
                            hash_headp,
                            entp,
                            &mut dead_entp,
                        );
                    }
                }
                infp = nextfp;
                continue;
            }

            let new_kmsg;
            if unsafe { (*send_list).base.is_null() } {
                new_kmsg = kmsg;
            } else {
                // SAFETY: the receive path may allocate a copy.
                let Some(allocated) = (unsafe { kmsg_get() }) else {
                    if let Some(dest) = IpcPort::valid(dest) {
                        // SAFETY: `dest` holds the copy `copy_send()` made.
                        unsafe { ipc_port::release_send(dest) };
                    }
                    break;
                };
                new_kmsg = allocated;
                // SAFETY: both messages are live, and the copied range is
                // the packet bytes and the header the source owns.
                unsafe {
                    ptr::copy_nonoverlapping(
                        (*net_kmsg(kmsg)).packet.as_ptr(),
                        (*net_kmsg(new_kmsg)).packet.as_mut_ptr(),
                        ret_count as usize,
                    );
                    ptr::copy_nonoverlapping(
                        (*net_kmsg(kmsg)).header.as_ptr(),
                        (*net_kmsg(new_kmsg)).header.as_mut_ptr(),
                        NET_HDW_HDR_MAX,
                    );
                }
            }
            // SAFETY: the message is live and this call owns it.
            unsafe {
                (*net_kmsg(new_kmsg)).packet_type.set_number(ret_count);
                new_kmsg.set_remote_port(dest.addr());
            }
            // SAFETY: the list holds messages this call owns.
            unsafe { ipc_kmsg::enqueue(send_list, new_kmsg) };

            // SAFETY: the port is live.
            let rcount = unsafe { (*infp).rcv_count.wrapping_add(1) };
            // SAFETY: as above.
            unsafe { (*infp).rcv_count = rcount };
            if unsafe { (*infp).priority } >= NET_HI_PRI {
                // The C examined both chains; a port is only linked into the
                // lists its filter flags name.
                // SAFETY: the port is live and held by the list locks.
                unsafe { reorder_prio(ifp, infp, true, rcount) };
                // SAFETY: as above.
                unsafe { reorder_prio(ifp, infp, false, rcount) };
                break;
            }
        }

        infp = nextfp;
    }

    // SAFETY: this call holds both interface list locks.
    unsafe {
        (*ifp).if_snd_port_list_lock.unlock();
        (*ifp).if_rcv_port_list_lock.unlock();
    }

    if !dead_infp.is_null() {
        // SAFETY: the list holds the ports this call unlinked.
        unsafe { free_dead_infp(dead_infp) };
    }
    if !dead_entp.is_null() {
        // SAFETY: the list holds the entries this call unlinked.
        unsafe { free_dead_entp(dead_entp) };
    }

    if unsafe { (*send_list).base.is_null() } {
        // SAFETY: no receiver took the message, so this call recycles it.
        unsafe { kmsg_put(kmsg.as_ptr()) };
    }
}

/// The C's `net_packet()` interface, with the kmsg still a raw pointer until
/// the checks are done.
///
/// # Safety
///
/// `ifp` must be a live interface and `kmsg` a live network message at
/// splimp, with the header the driver filled.
pub(crate) unsafe fn packet(
    ifp: *mut IfNet,
    kmsg: *mut c_void,
    count: c_uint,
    priority: bool,
) {
    let Some(kmsg) = NonNull::new(kmsg) else {
        return;
    };
    // SAFETY: the caller promises the live message.
    let kmsg = unsafe { Kmsg::from_raw(kmsg.as_ptr()) };

    // SAFETY: the caller promises the live interface and message.
    unsafe {
        kmsg.set_remote_port(ifp.addr());
        (*net_kmsg(kmsg)).packet_type.set_number(count);
    }

    NET_QUEUE_LOCK.lock();
    if priority {
        // SAFETY: the queue lock serializes the high queue.
        unsafe { ipc_kmsg::enqueue(queue_high(), kmsg) };
        // SAFETY: the queue lock serializes the size and maximum.
        unsafe {
            *NET_QUEUE_HIGH_SIZE.0.get() += 1;
            let size = *NET_QUEUE_HIGH_SIZE.0.get();
            let max = &mut *NET_QUEUE_HIGH_MAX.0.get();
            if size > *max {
                *max = size;
            }
        }
    } else {
        // SAFETY: the queue lock serializes the low queue.
        unsafe { ipc_kmsg::enqueue(queue_low(), kmsg) };
        let size = NET_QUEUE_LOW_SIZE
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        // SAFETY: the queue lock serializes the maximum.
        let max = unsafe { &mut *NET_QUEUE_LOW_MAX.0.get() };
        if size > *max {
            *max = size;
        }
    }
    // SAFETY: the queue lock serializes the flag.
    let awake = unsafe { *NET_THREAD_AWAKE.0.get() };
    NET_QUEUE_LOCK.unlock();

    if !awake {
        let s = unsafe { glue::splsched() };
        ast_on(cpu_number(), AST_NETWORK);
        let _ = unsafe { glue::splx(s) };
    }
}

/// `ethernet_priority()` of `device/net_io.c`: whether the packet is not a
/// six-byte broadcast address.
///
/// # Safety
///
/// `kmsg` must be a live network message.
pub(crate) unsafe fn ethernet_priority(kmsg: Kmsg) -> bool {
    // SAFETY: the caller promises the live message, whose header is 64
    // bytes.
    let addr = unsafe { (*net_kmsg(kmsg)).header.as_ptr().cast::<u8>() };
    // SAFETY: the loop stays inside the header's first six bytes.
    let broadcast = (0..6).all(|i| unsafe { *addr.add(i) } == 0xff);
    !broadcast
}

/// The C's `port == rcv_port || !IP_VALID(port) || !ip_active(port)` test of
/// `net_set_filter()`.
///
/// # Safety
///
/// `port` must be `IP_NULL`, `IP_DEAD` or a live port; a live one is locked
/// by this call.
unsafe fn entry_needs_removal(
    port: *mut c_void,
    rcv_port: *mut c_void,
) -> bool {
    if port == rcv_port {
        return true;
    }
    match IpcPort::valid(port) {
        None => true,
        // SAFETY: `valid` established a live port.
        Some(port) => !unsafe { port.is_active() },
    }
}

/// `net_set_filter()` of `device/net_io.c`.
///
/// # Safety
///
/// `ifp` must be a live interface; `rcv_port` a naked send right the caller
/// hands over on success; `filter` readable for `filter_count` `filter_t`
/// words.  No interface lock may be held.
pub(crate) unsafe fn set_filter(
    ifp: *mut IfNet,
    rcv_port: *mut c_void,
    priority: c_int,
    filter: *mut u16,
    filter_count: c_uint,
) -> IoResult {
    if filter_count == 0 || filter.is_null() {
        return Err(DeviceError::InvalidOperation);
    }
    // SAFETY: the caller promises `filter_count` readable words.
    let filter =
        unsafe { slice::from_raw_parts(filter, filter_count as usize) };
    let flags = filter[0];
    if flags & (NETF_IN | NETF_OUT) == 0 {
        return Err(DeviceError::InvalidOperation);
    }

    let filter_bytes = filter_count as usize * size_of::<u16>();
    let match_index = match flags & NETF_TYPE_MASK {
        NETF_BPF => {
            // SAFETY: the filter is readable for `filter_bytes` bytes at the
            // two-byte alignment the C casts accept.
            let validated = unsafe {
                validate(
                    filter.as_ptr().cast::<BpfInsn>(),
                    filter_bytes / size_of::<BpfInsn>(),
                )
            };
            match validated {
                Validated::Invalid => {
                    return Err(DeviceError::InvalidOperation);
                }
                Validated::Plain => None,
                Validated::Match(index) => Some(index),
            }
        }
        0 => {
            if !parse_filter(filter) {
                return Err(DeviceError::InvalidOperation);
            }
            None
        }
        _ => return Err(DeviceError::InvalidOperation),
    };

    let mut my_infp: *mut NetRcvPort = ptr::null_mut();
    let mut hash_entp: *mut NetHashEntry = ptr::null_mut();
    let mut is_new_infp = match_index.is_none();
    if match_index.is_none() {
        // SAFETY: `init()` built the cache.
        let Some(obj) = (unsafe { (*rcv_cache()).alloc() }) else {
            return Err(DeviceError::NoMemory);
        };
        my_infp = obj.as_ptr().cast();
        // SAFETY: the fresh port is this call's.
        unsafe { (*my_infp).rcv_port = rcv_port };
    } else {
        // SAFETY: `init()` built the cache.
        let Some(obj) = (unsafe { (*hash_entry_cache()).alloc() }) else {
            return Err(DeviceError::NoMemory);
        };
        hash_entp = obj.as_ptr().cast();
    }

    let n_keys = match match_index {
        None => 0usize,
        Some(index) => {
            // SAFETY: `validate` proved the match instruction is inside the
            // filter.
            let insn = unsafe {
                filter
                    .as_ptr()
                    .cast::<BpfInsn>()
                    .add(index)
                    .read_unaligned()
            };
            usize::from(insn.jt)
        }
    };

    let mut dead_infp: *mut QueueChain = ptr::null_mut();
    let mut dead_entp: *mut QueueChain = ptr::null_mut();

    // The list heads are read once, before the closure, so that its body
    // touches no field of `ifp`.
    // SAFETY: the caller promises a live interface.
    let rcv_head = unsafe { addr_of_mut!((*ifp).if_rcv_port_list) };
    // SAFETY: as above.
    let snd_head = unsafe { addr_of_mut!((*ifp).if_snd_port_list) };

    let mut check_filter_list = |list: *mut QueueChain, sent: bool| {
        // SAFETY: the list head is initialized and its links are containers.
        let mut infp = unsafe { (*list).next.cast::<NetRcvPort>() };
        while infp.cast::<QueueChain>() != list {
            let chain = unsafe { port_chain(infp, sent) };
            let nextfp = unsafe { (*chain).next.cast::<NetRcvPort>() };
            // SAFETY: every element of the list is a live receive port.
            if unsafe { (*infp).rcv_port }.is_null() {
                let count_words = unsafe {
                    (*infp).filter_end.addr() - addr_of!((*infp).filter).addr()
                } / size_of::<u16>();
                if match_index.is_some()
                    && unsafe { (*infp).priority } == priority
                    && my_infp.is_null()
                    && count_words == filter_count as usize
                    && unsafe {
                        eq(
                            (*infp).filter.as_ptr().cast::<BpfInsn>(),
                            filter.as_ptr().cast::<BpfInsn>(),
                            filter_bytes / size_of::<BpfInsn>(),
                        )
                    }
                {
                    my_infp = infp;
                }
                'buckets: for i in 0..NET_HASH_SIZE as usize {
                    // SAFETY: the port shares a hash header's storage.
                    let head = unsafe {
                        addr_of_mut!((*port_hash_header(infp)).table[i])
                    };
                    if unsafe { (*head).is_null() } {
                        continue;
                    }
                    let mut entp = unsafe { *head };
                    loop {
                        // SAFETY: the entry is linked into the bucket.
                        let nextentp = unsafe {
                            (*entp).chain.next.cast::<NetHashEntry>()
                        };
                        if unsafe {
                            entry_needs_removal((*entp).rcv_port, rcv_port)
                        } {
                            let used = !my_infp.is_null() && my_infp == infp;
                            // SAFETY: the header and entry are live, and
                            // this call holds both interface locks.
                            let removed = unsafe {
                                hash_ent_remove(
                                    ifp,
                                    port_hash_header(infp),
                                    used,
                                    head,
                                    entp,
                                    &mut dead_entp,
                                )
                            };
                            if removed {
                                break 'buckets;
                            }
                        }
                        entp = nextentp;
                        // SAFETY: `head` points into the live table.
                        if unsafe { (*head).is_null() || entp == *head } {
                            break;
                        }
                    }
                }
            } else if unsafe {
                entry_needs_removal((*infp).rcv_port, rcv_port)
            } {
                if unsafe { (*infp).filter[0] } & NETF_IN != 0 {
                    // SAFETY: the port is linked into the receive list.
                    unsafe {
                        queue_remove_generic(
                            rcv_head.cast(),
                            infp.cast(),
                            offset_of!(NetRcvPort, input),
                        );
                    }
                }
                if unsafe { (*infp).filter[0] } & NETF_OUT != 0 {
                    // SAFETY: the port is linked into the send list.
                    unsafe {
                        queue_remove_generic(
                            snd_head.cast(),
                            infp.cast(),
                            offset_of!(NetRcvPort, output),
                        );
                    }
                }
                // SAFETY: the port is off both lists and reuses `input`.
                unsafe { (*infp).input.next = dead_infp };
                dead_infp = infp.cast();
            }
            infp = nextfp;
        }
    };

    let in_ = flags & NETF_IN != 0;
    let out = flags & NETF_OUT != 0;

    // SAFETY: the caller promises no interface lock held.
    unsafe {
        (*ifp).if_rcv_port_list_lock.lock();
        (*ifp).if_snd_port_list_lock.lock();
    }
    if in_ {
        check_filter_list(rcv_head, true);
    }
    if out {
        check_filter_list(snd_head, false);
    }

    let mut failed = false;
    if my_infp.is_null() {
        NET_HASH_HEADER_LOCK.lock();
        let mut free_slot = N_NET_HASH;
        for i in 0..N_NET_HASH {
            // SAFETY: the header slot is static storage.
            if unsafe { (*hash_header_slot(i)).n_keys } == 0 {
                free_slot = i;
                break;
            }
        }
        if free_slot == N_NET_HASH {
            NET_HASH_HEADER_LOCK.unlock();
            // SAFETY: this call took both interface locks.
            unsafe {
                (*ifp).if_snd_port_list_lock.unlock();
                (*ifp).if_rcv_port_list_lock.unlock();
            }
            if let Some(port) = IpcPort::valid(rcv_port) {
                // SAFETY: `port` is a live send right this call owns.
                unsafe { ipc_port::release_send(port) };
            }
            if !hash_entp.is_null() {
                // SAFETY: the entry is a live allocation of the cache.
                unsafe {
                    (*hash_entry_cache())
                        .free(NonNull::new_unchecked(hash_entp.cast()))
                };
                hash_entp = ptr::null_mut();
            }
            failed = true;
        } else {
            let hhp = hash_header_slot(free_slot);
            // SAFETY: the slot is free and the lock keeps it so.
            unsafe { (*hhp).n_keys = n_keys as c_int };
            NET_HASH_HEADER_LOCK.unlock();
            // SAFETY: this call owns the slot until it is linked.
            unsafe {
                (*hhp).ref_count = 0;
                for i in 0..NET_HASH_SIZE as usize {
                    (*hhp).table[i] = ptr::null_mut();
                }
            }
            my_infp = hhp.cast();
            // SAFETY: as above.
            unsafe { (*my_infp).rcv_port = ptr::null_mut() };
            is_new_infp = true;
        }
    }

    if !failed {
        if is_new_infp {
            // SAFETY: the port is this call's fresh allocation.
            unsafe {
                (*my_infp).priority = priority;
                (*my_infp).rcv_count = 0;
                ptr::copy_nonoverlapping(
                    filter.as_ptr(),
                    (*my_infp).filter.as_mut_ptr(),
                    filter_count as usize,
                );
                (*my_infp).filter_end =
                    (*my_infp).filter.as_mut_ptr().add(filter_count as usize);
            }
            let qlimit = if match_index.is_none() {
                // SAFETY: the caller's port right is live when valid.
                unsafe { add_q_info(rcv_port) }
            } else {
                0
            };
            // SAFETY: the port is this call's fresh allocation.
            unsafe { (*my_infp).rcv_qlimit = qlimit };
            if in_ {
                // SAFETY: the receive list is initialized and held.
                unsafe {
                    queue_enter_tail(
                        addr_of_mut!((*ifp).if_rcv_port_list).cast(),
                        my_infp.cast(),
                        offset_of!(NetRcvPort, input),
                    );
                }
            }
            if out {
                // SAFETY: the send list is initialized and held.
                unsafe {
                    queue_enter_tail(
                        addr_of_mut!((*ifp).if_snd_port_list).cast(),
                        my_infp.cast(),
                        offset_of!(NetRcvPort, output),
                    );
                }
            }
        }

        if let Some(index) = match_index {
            // SAFETY: the entry is this call's fresh allocation.
            unsafe { (*hash_entp).rcv_port = rcv_port };
            let mut keys = [0u32; N_NET_HASH_KEYS];
            for i in 0..n_keys {
                // SAFETY: `validate` proved the key instructions follow the
                // match instruction inside the filter.
                let insn = unsafe {
                    filter
                        .as_ptr()
                        .cast::<BpfInsn>()
                        .add(index + 1 + i)
                        .read_unaligned()
                };
                let key = insn.k as c_uint;
                if let Some(slot) = keys.get_mut(i) {
                    *slot = key;
                }
                // SAFETY: the entry holds `N_NET_HASH_KEYS` key words.
                if let Some(slot) = unsafe { (*hash_entp).keys.get_mut(i) } {
                    *slot = key;
                }
            }
            let header = my_infp.cast::<NetHashHeader>();
            // SAFETY: `hash()` stays below the table length.
            let bucket = hash(&keys[..n_keys]) as usize;
            // SAFETY: the header is the live group this call holds.
            let p = unsafe { addr_of_mut!((*header).table[bucket]) };
            // SAFETY: the bucket is empty or holds a chain of live entries.
            unsafe {
                if (*p).is_null() {
                    queue_init(
                        addr_of_mut!((*hash_entp).chain).cast::<QueueEntry>(),
                    );
                    *p = hash_entp;
                } else {
                    enqueue_tail(
                        addr_of_mut!((*(*p)).chain).cast::<QueueEntry>(),
                        addr_of_mut!((*hash_entp).chain).cast::<QueueEntry>(),
                    );
                }
                (*header).ref_count += 1;
            }
            // SAFETY: the caller's port right is live when valid.
            let qlimit = unsafe { add_q_info(rcv_port) };
            // SAFETY: the entry is this call's.
            unsafe { (*hash_entp).rcv_qlimit = qlimit };
        }

        // SAFETY: this call holds both interface locks.
        unsafe {
            (*ifp).if_snd_port_list_lock.unlock();
            (*ifp).if_rcv_port_list_lock.unlock();
        }
    }

    if !dead_infp.is_null() {
        // SAFETY: the list holds the ports this call unlinked.
        unsafe { free_dead_infp(dead_infp) };
    }
    if !dead_entp.is_null() {
        // SAFETY: the list holds the entries this call unlinked.
        unsafe { free_dead_entp(dead_entp) };
    }

    if failed {
        return Err(DeviceError::NoMemory);
    }
    Ok(DeviceSuccess::Success)
}

/// `hash_ent_remove()` of `device/net_io.c`.
///
/// # Safety
///
/// `hp` and `entp` must be live filter structures, `head` the bucket `entp`
/// is linked into, and `dead_p` a list head this call owns.
pub(crate) unsafe fn hash_ent_remove(
    ifp: *mut IfNet,
    hp: *mut NetHashHeader,
    used: bool,
    head: *mut *mut NetHashEntry,
    entp: *mut NetHashEntry,
    dead_p: *mut *mut QueueChain,
) -> bool {
    // SAFETY: the caller promises the live header.
    unsafe { (*hp).ref_count -= 1 };

    // SAFETY: the caller promises the live bucket and entry.
    if unsafe { *head } == entp {
        // SAFETY: as above.
        if unsafe { (*entp).chain.next } == entp.cast::<QueueChain>() {
            // SAFETY: as above, and the dead list's head is writable.
            unsafe {
                *head = ptr::null_mut();
                (*entp).chain.next = *dead_p;
                *dead_p = entp.cast();
            }
            // SAFETY: the caller promises a live header.
            if unsafe { (*hp).ref_count == 0 } && !used {
                let port = hp.cast::<NetRcvPort>();
                // SAFETY: the header shares the port's storage and is live.
                unsafe {
                    if (*port).filter[0] & NETF_IN != 0 {
                        queue_remove_generic(
                            addr_of_mut!((*ifp).if_rcv_port_list).cast(),
                            hp.cast(),
                            offset_of!(NetRcvPort, input),
                        );
                    }
                    if (*port).filter[0] & NETF_OUT != 0 {
                        queue_remove_generic(
                            addr_of_mut!((*ifp).if_snd_port_list).cast(),
                            hp.cast(),
                            offset_of!(NetRcvPort, output),
                        );
                    }
                    (*hp).n_keys = 0;
                }
                return true;
            }
            return false;
        }
        // SAFETY: the entry is linked into this bucket.
        unsafe { *head = (*entp).chain.next.cast() };
    }

    // SAFETY: the entry is linked into the bucket.
    unsafe {
        crate::kern::queue::remqueue(
            (*head).cast::<QueueEntry>(),
            entp.cast::<QueueEntry>(),
        );
        (*entp).chain.next = *dead_p;
        *dead_p = entp.cast();
    }
    false
}

/// `net_add_q_info()` of `device/net_io.c`.
///
/// # Safety
///
/// `rcv_port` must be `IP_NULL`, `IP_DEAD` or a live port.
pub(crate) unsafe fn add_q_info(rcv_port: *mut c_void) -> c_int {
    let mut qlimit = 0u32;
    if let Some(port) = IpcPort::valid(rcv_port) {
        // SAFETY: `valid` established a live port.
        unsafe { port.lock() };
        // SAFETY: the port is live and locked.
        if unsafe { port.is_active() } {
            // SAFETY: as above.
            qlimit = unsafe { port.qlimit() };
        }
        // SAFETY: as above.
        unsafe { port.unlock() };
    }

    NET_KMSG_TOTAL_LOCK.lock();
    NET_QUEUE_FREE_MIN.fetch_add(1, Ordering::Relaxed);
    NET_KMSG_MAX.fetch_add(qlimit.wrapping_add(1) as c_int, Ordering::Relaxed);
    NET_KMSG_TOTAL_LOCK.unlock();

    qlimit as c_int
}

/// `net_del_q_info()` of `device/net_io.c`.
///
/// # Safety
///
/// `qlimit` must be the value `add_q_info()` returned for this filter.
unsafe fn del_q_info(qlimit: c_int) {
    NET_KMSG_TOTAL_LOCK.lock();
    NET_QUEUE_FREE_MIN.fetch_sub(1, Ordering::Relaxed);
    NET_KMSG_MAX.fetch_sub(qlimit.wrapping_add(1), Ordering::Relaxed);
    NET_KMSG_TOTAL_LOCK.unlock();
}

/// `net_free_dead_infp()` of `device/net_io.c`.
///
/// # Safety
///
/// `dead` must head a list of receive ports this call unlinked, and no lock
/// may be held.
pub(crate) unsafe fn free_dead_infp(dead: *mut QueueChain) {
    let mut infp = dead.cast::<NetRcvPort>();
    while !infp.is_null() {
        // SAFETY: the dead list links live ports through `input`.
        let nextfp = unsafe { (*infp).input.next.cast::<NetRcvPort>() };
        // SAFETY: the port is live.
        if let Some(port) = IpcPort::valid(unsafe { (*infp).rcv_port }) {
            // SAFETY: the filter owns one send right to the port.
            unsafe { ipc_port::release_send(port) };
        }
        // SAFETY: the port is live.
        let qlimit = unsafe { (*infp).rcv_qlimit };
        // SAFETY: the caller holds no lock.
        unsafe { del_q_info(qlimit) };
        // SAFETY: the port is a live allocation of the cache.
        unsafe {
            (*rcv_cache()).free(NonNull::new_unchecked(infp.cast::<u8>()));
        }
        infp = nextfp;
    }
}

/// `net_free_dead_entp()` of `device/net_io.c`.
///
/// # Safety
///
/// `dead` must head a list of hash entries this call unlinked, and no lock
/// may be held.
pub(crate) unsafe fn free_dead_entp(dead: *mut QueueChain) {
    let mut entp = dead.cast::<NetHashEntry>();
    while !entp.is_null() {
        // SAFETY: the dead list links live entries through `chain`.
        let nextentp = unsafe { (*entp).chain.next.cast::<NetHashEntry>() };
        // SAFETY: the entry is live.
        if let Some(port) = IpcPort::valid(unsafe { (*entp).rcv_port }) {
            // SAFETY: the filter owns one send right to the port.
            unsafe { ipc_port::release_send(port) };
        }
        // SAFETY: the entry is live.
        let qlimit = unsafe { (*entp).rcv_qlimit };
        // SAFETY: the caller holds no lock.
        unsafe { del_q_info(qlimit) };
        // SAFETY: the entry is a live allocation of the cache.
        unsafe {
            (*hash_entry_cache())
                .free(NonNull::new_unchecked(entp.cast::<u8>()));
        }
        entp = nextentp;
    }
}

/// `net_getstat()` of `device/net_io.c`.
///
/// # Safety
///
/// `ifp` must be a live interface, `status` writable for `*count` words, and
/// `count` readable and writable.
pub(crate) unsafe fn getstat(
    ifp: *mut IfNet,
    flavor: c_int,
    status: *mut c_int,
    count: *mut c_uint,
) -> IoResult {
    match flavor {
        NET_STATUS => {
            if unsafe { *count } < NET_STATUS_COUNT {
                return Err(DeviceError::InvalidOperation);
            }
            let ns = status.cast::<NetStatus>();
            // SAFETY: the caller promises a live interface and a writable
            // status buffer of at least `NET_STATUS_COUNT` words.
            unsafe {
                (*ns).min_packet_size = c_int::from((*ifp).if_header_size);
                (*ns).max_packet_size = c_int::from((*ifp).if_header_size)
                    + c_int::from((*ifp).if_mtu);
                (*ns).header_format = c_int::from((*ifp).if_header_format);
                (*ns).header_size = c_int::from((*ifp).if_header_size);
                (*ns).address_size = c_int::from((*ifp).if_address_size);
                (*ns).flags = c_int::from((*ifp).if_flags);
                (*ns).mapped_size = 0;
                *count = NET_STATUS_COUNT;
            }
            Ok(DeviceSuccess::Success)
        }
        NET_ADDRESS => {
            let Ok(byte_count) =
                usize::try_from(unsafe { (*ifp).if_address_size })
            else {
                return Err(DeviceError::InvalidOperation);
            };
            let int_count = byte_count.div_ceil(size_of::<c_int>());
            if unsafe { *count } < int_count as c_uint {
                // SAFETY: the format takes two `int` arguments, as the C's
                // arguments are.
                unsafe {
                    glue::printf(
                        c"net_getstat: count: %d, addr_int_count: %d\n"
                            .as_ptr(),
                        *count as c_int,
                        int_count as c_int,
                    )
                };
                return Err(DeviceError::InvalidOperation);
            }
            let bytes = status.cast::<u8>();
            // SAFETY: the caller promises `byte_count` readable address
            // bytes, and the status buffer is writable for the padded words.
            unsafe {
                ptr::copy_nonoverlapping(
                    (*ifp).if_address.cast::<u8>(),
                    bytes,
                    byte_count,
                );
                let total = int_count * size_of::<c_int>();
                if byte_count < total {
                    ptr::write_bytes(
                        bytes.add(byte_count),
                        0,
                        total - byte_count,
                    );
                }
            }
            for i in 0..int_count {
                // SAFETY: the buffer holds `int_count` readable words.
                let word = unsafe { *status.add(i) } as u32;
                // SAFETY: as above; the store replaces the word with its
                // network-order image.
                unsafe { *status.add(i) = htonl(word) as c_int };
            }
            // SAFETY: the caller promises a writable count.
            unsafe { *count = int_count as c_uint };
            Ok(DeviceSuccess::Success)
        }
        _ => Err(DeviceError::InvalidOperation),
    }
}

/// `net_write()` of `device/net_io.c`.
///
/// # Safety
///
/// `ifp` must be a live interface, `ior` a live request the caller owns, and
/// `start` the driver's start routine.
pub(crate) unsafe fn write(
    ifp: *mut IfNet,
    start: Option<unsafe extern "C" fn(c_short) -> c_int>,
    ior: *mut IoReq,
) -> Result<DeviceSuccess, WriteError> {
    let flags = c_int::from(unsafe { (*ifp).if_flags });
    if flags & (IFF_UP | IFF_RUNNING) != (IFF_UP | IFF_RUNNING) {
        return Err(WriteError::Device(DeviceError::DeviceDown));
    }

    let count = unsafe { (*ior).count };
    let header = c_int::from(unsafe { (*ifp).if_header_size });
    let mtu = c_int::from(unsafe { (*ifp).if_mtu });
    if count < c_long::from(header) || c_long::from(header + mtu) < count {
        return Err(WriteError::Device(DeviceError::InvalidSize));
    }

    let mut wait: c_int = 0;
    // SAFETY: the caller promises the live request, and `wait` is writable.
    let rc = unsafe { ds_routines::device_write_get(ior, &mut wait) };
    if rc != 0 {
        return Err(WriteError::Kern(rc));
    }
    if wait != 0 {
        // SAFETY: `Panic` does not return; the file and function tags are
        // the C `panic()` call's.
        unsafe {
            glue::Panic(
                c"device/net_io.c".as_ptr(),
                line!() as c_int,
                c"net_write".as_ptr(),
                c"net_write: VM continuation".as_ptr(),
            )
        }
    }

    let s = unsafe { glue::splimp() };
    // SAFETY: the caller promises the live interface and request.
    unsafe {
        let ifq = addr_of_mut!((*ifp).if_snd);
        (*ifq).ifq_lock.lock();
        enqueue_tail(
            addr_of_mut!((*ifq).ifq_head).cast::<QueueEntry>(),
            ior.cast::<QueueEntry>(),
        );
        (*ifq).ifq_len += 1;
        (*ifq).ifq_lock.unlock();

        if let Some(start) = start {
            start((*ifp).if_unit);
        }
    }
    let _ = unsafe { glue::splx(s) };

    Ok(DeviceSuccess::IoQueued)
}

/// What `net_write()` reports when it cannot queue the request: a `D_*` code
/// or the `kern_return_t` `device_write_get()` returned, which the C passed
/// through untouched.
pub(crate) enum WriteError {
    Device(DeviceError),
    Kern(c_int),
}

/// `net_io_init()` of `device/net_io.c`.
pub(crate) unsafe fn init() {
    // SAFETY: the caches are static storage no thread can see yet.
    unsafe {
        (*rcv_cache()).init(
            b"net_rcv_port",
            size_of::<NetRcvPort>(),
            0,
            None,
            CacheInitFlags::from_bits(0),
        );
    }
    // SAFETY: as above.
    unsafe {
        (*hash_entry_cache()).init(
            b"net_hash_entry",
            size_of::<NetHashEntry>(),
            0,
            None,
            CacheInitFlags::from_bits(0),
        );
    }

    let size = ikm_plus_overhead(size_of::<NetRcvMsg>());
    NET_KMSG_SIZE
        .store(crate::vm::vm_map::round_page(size), Ordering::Relaxed);

    NET_KMSG_TOTAL_LOCK.init();
    if NET_KMSG_MAX.load(Ordering::Relaxed) == 0 {
        NET_KMSG_MAX.store(
            NET_QUEUE_FREE_MIN.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }

    NET_QUEUE_FREE_LOCK.init();
    // SAFETY: this call owns the queues before threads start.
    unsafe { (*queue_free()).base = ptr::null_mut() };

    NET_QUEUE_LOCK.init();
    // SAFETY: as above.
    unsafe {
        (*queue_high()).base = ptr::null_mut();
        (*queue_low()).base = ptr::null_mut();
    }

    NET_HASH_HEADER_LOCK.init();
}
