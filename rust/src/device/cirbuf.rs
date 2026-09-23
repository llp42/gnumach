// SPDX-License-Identifier: CMU-Mach
// Derived from device/cirbuf.c:
//   Copyright (c) 1992,1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The circular character buffers of `device/cirbuf.c`, the implementation of
//! <device/cirbuf.h>.

use crate::arch::types::VmSize;
use crate::glue;
use core::ffi::{c_char, c_int, c_short};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;
use core::slice;

/// `struct cirbuf` of <device/cirbuf.h>, field for field.
///
/// # Invariants
///
/// The four pointers are all null, or they all name one live allocation of
/// `c_end - c_start` bytes from [`Cirbuf::alloc`]: `c_start` at its base,
/// `c_end` one past its last byte, and `c_cf` and `c_cl` inside it.
#[repr(C)]
pub struct Cirbuf {
    c_start: *mut c_char,
    c_end: *mut c_char,
    c_cf: *mut c_char,
    c_cl: *mut c_char,
    c_cc: c_short,
    c_hog: c_short,
}

const _: () = {
    assert!(offset_of!(Cirbuf, c_start) == 0);
    assert!(offset_of!(Cirbuf, c_end) == size_of::<*mut c_char>());
    assert!(offset_of!(Cirbuf, c_cf) == 2 * size_of::<*mut c_char>());
    assert!(offset_of!(Cirbuf, c_cl) == 3 * size_of::<*mut c_char>());
    assert!(offset_of!(Cirbuf, c_cc) == 4 * size_of::<*mut c_char>());
    assert!(offset_of!(Cirbuf, c_hog) == 4 * size_of::<*mut c_char>() + 2);
};
const _: () = assert!(align_of::<Cirbuf>() == align_of::<*mut c_char>());

#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<Cirbuf>() == 20);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Cirbuf>() == 40);

impl Cirbuf {
    /// An unallocated, empty buffer.
    pub const fn new() -> Self {
        Self {
            c_start: ptr::null_mut(),
            c_end: ptr::null_mut(),
            c_cf: ptr::null_mut(),
            c_cl: ptr::null_mut(),
            c_cc: 0,
            c_hog: 0,
        }
    }

    /// The number of characters in the buffer, the C `c_cc`.
    pub fn count(&self) -> c_short {
        self.c_cc
    }

    /// The allocation's length, or zero when the buffer is unallocated.
    fn extent(&self) -> usize {
        self.c_end.addr().wrapping_sub(self.c_start.addr())
    }

    /// Put one byte at the write pointer.
    pub fn put(&mut self, value: u8) -> bool {
        if self.extent() < 2 {
            return false;
        }
        let write = self.c_cl;
        let next = write.wrapping_add(1);
        let next = if ptr::eq(next, self.c_end) {
            self.c_start
        } else {
            next
        };
        if ptr::eq(next, self.c_cf) {
            return false;
        }
        // SAFETY: the extent check above puts the buffer on a live allocation,
        // whose invariant places `write` inside it, and `write` is the byte
        // the write pointer names.
        unsafe { write.cast::<u8>().write(value) };
        self.c_cl = next;
        self.c_cc = self.c_cc.wrapping_add(1);
        true
    }

    /// Take one byte from the read pointer.
    pub fn get(&mut self) -> Option<u8> {
        if ptr::eq(self.c_cf, self.c_cl) {
            return None;
        }
        // SAFETY: the buffer invariant puts `c_cf` inside the allocation, and
        // `c_cf` and `c_cl` differ, so a character waits there.
        let value = unsafe { self.c_cf.cast::<u8>().read() };
        let next = self.c_cf.wrapping_add(1);
        self.c_cf = if ptr::eq(next, self.c_end) {
            self.c_start
        } else {
            next
        };
        self.c_cc = self.c_cc.wrapping_sub(1);
        Some(value)
    }

    /// Move up to `out.len()` bytes out of the buffer.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut moved = 0;
        while moved < out.len() && !ptr::eq(self.c_cf, self.c_cl) {
            let end = self.c_end.addr();
            let read = self.c_cf.addr();
            let run = if self.c_cl.addr() < read {
                end - read
            } else {
                self.c_cl.addr() - read
            };
            let count = run.min(out.len() - moved);
            // SAFETY: `[c_cf, c_cf + count)` is a readable run inside the
            // allocation, cut short at the write pointer or the buffer end
            // above.
            unsafe {
                ptr::copy_nonoverlapping(
                    self.c_cf.cast::<u8>(),
                    out.as_mut_ptr().add(moved),
                    count,
                );
            }
            moved += count;
            self.c_cf = self.c_cf.wrapping_add(count);
            if ptr::eq(self.c_cf, self.c_end) {
                self.c_cf = self.c_start;
            }
            // The C subtracts the int `i` from the short `c_cc`; the run never
            // exceeds the buffer and the callers size it in the thousands, so
            // the conversion cannot lose anything.
            self.c_cc = self.c_cc.wrapping_sub(count as c_short);
        }
        moved
    }

    /// Move up to `input.len()` bytes into the buffer.
    pub fn write(&mut self, input: &[u8]) -> usize {
        if self.extent() < 2 {
            return 0;
        }
        let mut entered = 0;
        while entered < input.len() {
            let limit = if ptr::eq(self.c_cf, self.c_start) {
                self.c_end.wrapping_sub(1)
            } else {
                self.c_cf.wrapping_sub(1)
            };
            if ptr::eq(self.c_cl, limit) {
                break;
            }
            let end = self.c_end.addr();
            let write = self.c_cl.addr();
            let run = if write < limit.addr() {
                limit.addr() - write
            } else {
                end - write
            };
            let count = run.min(input.len() - entered);
            // SAFETY: the extent check above puts the buffer on a live
            // allocation.
            unsafe {
                ptr::copy_nonoverlapping(
                    input.as_ptr().add(entered),
                    self.c_cl.cast::<u8>(),
                    count,
                );
            }
            entered += count;
            self.c_cl = self.c_cl.wrapping_add(count);
            if ptr::eq(self.c_cl, self.c_end) {
                self.c_cl = self.c_start;
            }
            // As in `read`, the run is far smaller than `c_short::MAX`.
            self.c_cc = self.c_cc.wrapping_add(count as c_short);
        }
        entered
    }

    /// Discard up to `count` bytes from the read end of the buffer.
    pub fn flush(&mut self, count: usize) {
        let mut left = count;
        while left != 0 && !ptr::eq(self.c_cf, self.c_cl) {
            let end = self.c_end.addr();
            let read = self.c_cf.addr();
            let run = if self.c_cl.addr() < read {
                end - read
            } else {
                self.c_cl.addr() - read
            };
            let dropped = run.min(left);
            left -= dropped;
            self.c_cf = self.c_cf.wrapping_add(dropped);
            if ptr::eq(self.c_cf, self.c_end) {
                self.c_cf = self.c_start;
            }
            self.c_cc = self.c_cc.wrapping_sub(dropped as c_short);
        }
    }

    /// Reset the buffer to empty at the start of its allocation.
    pub fn clear(&mut self) {
        self.c_cf = self.c_start;
        self.c_cl = self.c_start;
        self.c_cc = 0;
    }

    /// Allocate `size` bytes and reset the buffer to empty at their base.
    ///
    /// # Safety
    ///
    /// `kalloc_init()` must have run.
    pub unsafe fn alloc(&mut self, size: VmSize) {
        // SAFETY: the caller promises the allocator is up.
        let address = unsafe { glue::kalloc(size) };
        let start = ptr::with_exposed_provenance_mut::<c_char>(address);
        self.c_start = start;
        self.c_end = start.wrapping_add(size);
        self.c_cf = start;
        self.c_cl = start;
        self.c_cc = 0;
        // The C stores `buf_size - 1` in a short, truncating; the tty callers
        // pass 4096 and 2048, so the value survives whole.
        self.c_hog = size.wrapping_sub(1) as c_short;
    }

    /// Release the allocation [`Cirbuf::alloc`] took.
    ///
    /// # Safety
    ///
    /// The buffer must hold a live allocation from [`Cirbuf::alloc`] that
    /// nothing else references, and it must not be used again before another
    /// one.
    pub unsafe fn free(&mut self) {
        let size = self.c_end.addr().wrapping_sub(self.c_start.addr());
        // SAFETY: the caller promises a live allocation of `size` bytes based
        // at `c_start`.
        unsafe { glue::kfree(self.c_start.addr(), size) };
    }
}

impl Default for Cirbuf {
    fn default() -> Self {
        Self::new()
    }
}

/// `putc()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putc(c: c_int, cb: *mut Cirbuf) -> c_int {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    // The C stores the int through a `char *`, so only its low byte survives;
    // `put` reports true when the byte was entered.
    if cb.put(c as u8) { 0 } else { 1 }
}

/// `getc()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getc(cb: *mut Cirbuf) -> c_int {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    match cb.get() {
        Some(value) => c_int::from(value),
        None => -1,
    }
}

/// `q_to_b()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`]; `cp` must be writable for
/// `count` bytes and must not overlap the buffer's allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn q_to_b(
    cb: *mut Cirbuf,
    cp: *mut c_char,
    count: c_int,
) -> c_int {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    // A negative count is outside the supported contract: the C would take it
    // as a length and corrupt memory.
    let count = count.max(0) as usize;
    // SAFETY: the caller promises `count` writable bytes at `cp`.
    let out: &mut [u8] = if count == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(cp.cast::<u8>(), count) }
    };
    // The move stays within `count`, which fits a `c_int`.
    cb.read(out) as c_int
}

/// `b_to_q()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`]; `cp` must be readable for
/// `count` bytes and must not overlap the buffer's allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn b_to_q(
    cp: *mut c_char,
    count: c_int,
    cb: *mut Cirbuf,
) -> c_int {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    // As in `q_to_b`, a negative count is outside the supported contract: the
    // C would take it as a length and corrupt memory.
    let count = count.max(0) as usize;
    // SAFETY: the caller promises `count` readable bytes at `cp`.
    let input: &[u8] = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(cp.cast::<u8>(), count) }
    };
    // `write` enters at most `count` bytes, so the subtraction cannot
    // underflow and the C value fits a `c_int`.
    (count - cb.write(input)) as c_int
}

/// `ndflush()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ndflush(cb: *mut Cirbuf, count: c_int) {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    cb.flush(count.max(0) as usize);
}

/// `cb_clear()` in C.
///
/// # Safety
///
/// `cb` must point at a live [`Cirbuf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cb_clear(cb: *mut Cirbuf) {
    // SAFETY: the caller promises a live buffer.
    let cb = unsafe { &mut *cb };
    cb.clear();
}

/// `cb_alloc()` in C.
///
/// # Safety
///
/// `cb` must point at a live, unallocated [`Cirbuf`], and `kalloc_init()` must
/// have run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cb_alloc(cb: *mut Cirbuf, buf_size: VmSize) {
    // SAFETY: the caller promises a live, unallocated buffer and a running
    // allocator.
    let cb = unsafe { &mut *cb };
    // SAFETY: as above.
    unsafe { cb.alloc(buf_size) };
}

/// `cb_free()` in C.
///
/// # Safety
///
/// `cb` must point at a live, allocated [`Cirbuf`] whose store nothing else
/// references, and the buffer must not be used again before another
/// `cb_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cb_free(cb: *mut Cirbuf) {
    // SAFETY: the caller promises a live, allocated buffer.
    let cb = unsafe { &mut *cb };
    // SAFETY: as above.
    unsafe { cb.free() };
}
