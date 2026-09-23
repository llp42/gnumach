// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/lock.h and x86_64/x86_64/lock.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The bit operations of the machine lock header, which held them as inline
//! asm macros in `i386/i386/lock.h` until that file was deleted.

use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

/// The atomic word and bit mask that `bit` names in the bitmap at `l`.
///
/// # Safety
///
/// `l` must point at the base of a live bitmap whose 32-bit word at `bit / 32`
/// is aligned, readable and writable, and `bit` must not be negative.
unsafe fn bit_word<'a>(bit: c_int, l: *mut c_void) -> (&'a AtomicU32, u32) {
    // The C asm fed `(int)(bit)` to `btl`/`btsl`/`btrl`, whose memory operand
    // is addressed as `l + bit/32` with the selected bit at `bit % 32`.
    let bit = bit as u32;
    let mask = 1 << (bit % 32);
    // `bit / 32` is bounded by the length of the bitmap, so widening it to
    // `usize` cannot lose anything on either kernel.
    let offset = (bit / 32) as usize;
    // SAFETY: the caller promises a live, aligned bitmap with room for the
    // word, so the pointer is valid for the atomic's accesses, and `AtomicU32`
    // has `u32`'s size and alignment.
    let word = unsafe { AtomicU32::from_ptr(l.cast::<u32>().add(offset)) };
    (word, mask)
}

/// Acquire the bit lock on bit `bit` of the bitmap at `l`.
///
/// # Safety
///
/// `l` must point at the base of a live bitmap whose 32-bit word at `bit / 32`
/// is aligned, readable and writable, and `bit` must not be negative.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bit_lock(bit: c_int, l: *mut c_void) {
    // SAFETY: the caller promises a live bitmap.
    let (word, mask) = unsafe { bit_word(bit, l) };
    loop {
        // The waiting load is the C's unlocked `btl`: only the locked `btsl`
        // below decides whether the lock was taken, so the load is `Relaxed`,
        // as `SimpleLock::lock()` reads its word.
        while word.load(Ordering::Relaxed) & mask != 0 {
            core::hint::spin_loop();
        }
        // The C's `lock btsl` is a full barrier, so the attempt is `SeqCst`;
        // the old word it returns says whether the bit was already set, and
        // then the whole attempt repeats.
        if word.fetch_or(mask, Ordering::SeqCst) & mask == 0 {
            break;
        }
    }
}

/// Release the bit lock on bit `bit` of the bitmap at `l`.
///
/// # Safety
///
/// `l` must point at the base of a live bitmap whose 32-bit word at `bit / 32`
/// is aligned, readable and writable, and `bit` must not be negative.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bit_unlock(bit: c_int, l: *mut c_void) {
    // SAFETY: the caller promises a live bitmap.
    let (word, mask) = unsafe { bit_word(bit, l) };
    // The C's `lock btrl` is a full barrier, so the clear is `SeqCst`.
    word.fetch_and(!mask, Ordering::SeqCst);
}

/// Set bit `bit` of the bitmap at `l`.
///
/// # Safety
///
/// `l` must point at the base of a live bitmap whose 32-bit word at `bit / 32`
/// is aligned, readable and writable, and `bit` must not be negative.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i_bit_set(bit: c_int, l: *mut c_void) {
    // SAFETY: the caller promises a live bitmap.
    let (word, mask) = unsafe { bit_word(bit, l) };
    // The C's `lock btsl` is a full barrier, so the set is `SeqCst`.
    word.fetch_or(mask, Ordering::SeqCst);
}

/// Clear bit `bit` of the bitmap at `l`.
///
/// # Safety
///
/// `l` must point at the base of a live bitmap whose 32-bit word at `bit / 32`
/// is aligned, readable and writable, and `bit` must not be negative.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i_bit_clear(bit: c_int, l: *mut c_void) {
    // SAFETY: the caller promises a live bitmap.
    let (word, mask) = unsafe { bit_word(bit, l) };
    // The C's `lock btrl` is a full barrier, so the clear is `SeqCst`.
    word.fetch_and(!mask, Ordering::SeqCst);
}
