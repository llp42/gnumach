// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_name.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device-name lookup of `device/dev_name.c`, declared in
//! <device/dev_hdr.h> and <device/conf.h>.

use crate::device::ds_routines::DevOps;
use crate::glue;
use crate::utils::string::strcmp;
use core::ffi::{c_char, c_int};
use core::mem::size_of;
use core::ptr::{self, NonNull};
use core::slice;

/// `struct dev_indirect` of <device/conf.h>: the operation vector and unit one
/// indirect name stands for.
#[repr(C)]
pub struct DevIndirect {
    pub d_name: *mut c_char,
    pub d_ops: *mut DevOps,
    pub d_unit: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<DevIndirect>() == 24);
    assert!(core::mem::align_of::<DevIndirect>() == 8);
    assert!(core::mem::offset_of!(DevIndirect, d_name) == 0);
    assert!(core::mem::offset_of!(DevIndirect, d_ops) == 8);
    assert!(core::mem::offset_of!(DevIndirect, d_unit) == 16);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<DevIndirect>() == 12);
    assert!(core::mem::align_of::<DevIndirect>() == 4);
    assert!(core::mem::offset_of!(DevIndirect, d_name) == 0);
    assert!(core::mem::offset_of!(DevIndirect, d_ops) == 4);
    assert!(core::mem::offset_of!(DevIndirect, d_unit) == 8);
};

/// The C `c >= '0' && c <= '9'`.
fn is_digit(c: c_char) -> bool {
    b'0' as c_char <= c && c <= b'9' as c_char
}

/// `name_equal()` of <device/dev_hdr.h>: whether `target` begins with the
/// `len` bytes at `src` and ends there.
///
/// # Safety
///
/// `src` must be readable for `len` bytes when `len` is positive (nothing is
/// read when it is not), and `target` must be readable through the first byte
/// that differs from `src`, or through `target[len]` when the first `len`
/// bytes all match.
pub(crate) unsafe fn name_equal(
    src: *const c_char,
    len: c_int,
    target: *const c_char,
) -> bool {
    // The C's pre-decrement skips its loop for any len <= 0 and then asks only
    // whether target is empty; the clamp keeps that answer.
    let len = len.max(0) as usize;
    // SAFETY: the caller promises `len` readable bytes at `src` when positive.
    let src: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(src.cast::<u8>(), len) }
    };
    let target = target.cast::<u8>();
    for (i, &want) in src.iter().enumerate() {
        // SAFETY: the caller promises `target` readable through the first byte
        // that differs from `src`, and no byte before `i` has differed yet.
        if unsafe { *target.add(i) } != want {
            return false;
        }
    }
    // SAFETY: every byte of the prefix matched, and the caller promises
    // `target[len]` readable for that case; it is the terminator the C tests.
    (unsafe { *target.add(len) }) == 0
}

/// The unit arithmetic `dev_name_lookup()` applies once a name matched.
///
/// # Safety
///
/// `cp` must point into the NUL-terminated name, at the first byte after the
/// unit digits, and `c` must be the byte at `cp`; `subdev` is the matched
/// entry's `d_subdev`.
unsafe fn subdev_unit(
    mut unit: c_int,
    subdev: c_int,
    mut c: c_char,
    mut cp: *const c_char,
) -> c_int {
    if subdev <= 0 {
        return unit;
    }

    unit = unit.wrapping_mul(subdev);
    let mut slice_num: c_int = 0;
    if c == b's' as c_char {
        // SAFETY: the caller's walk stays inside the NUL-terminated name.
        cp = unsafe { cp.add(1) };
        loop {
            // SAFETY: as above.
            c = unsafe { *cp };
            if c == 0 || !is_digit(c) {
                break;
            }
            slice_num = slice_num
                .wrapping_mul(10)
                .wrapping_add(c_int::from(c as u8 - b'0'));
            // SAFETY: as above.
            cp = unsafe { cp.add(1) };
        }
    }

    unit = unit.wrapping_add(slice_num << 4);
    let offset = c_int::from(c) - c_int::from(b'a' as c_char);
    if 0 <= offset && offset < subdev {
        unit = unit.wrapping_add(offset + 1);
    }
    unit
}

/// `dev_name_lookup()` of `device/dev_name.c`.
///
/// # Safety
///
/// `name` must be a NUL-terminated string readable by the caller.
pub(crate) unsafe fn lookup(
    name: *const c_char,
) -> Option<(NonNull<DevOps>, c_int)> {
    let mut cp = name;
    let mut len: c_int = 0;
    loop {
        // SAFETY: the caller promises the NUL-terminated name.
        let c = unsafe { *cp };
        if c == 0 || is_digit(c) {
            break;
        }
        len += 1;
        // SAFETY: the walk stops at the terminator.
        cp = unsafe { cp.add(1) };
    }

    let mut unit: c_int = 0;
    // SAFETY: the walk stopped at the terminator or the first digit.
    let mut c = unsafe { *cp };
    if c != 0 {
        while is_digit(c) {
            // The C multiplied an `int` and offset by the digit.
            unit = unit
                .wrapping_mul(10)
                .wrapping_add(c_int::from(c as u8 - b'0'));
            // SAFETY: the walk stops at the terminator.
            cp = unsafe { cp.add(1) };
            c = unsafe { *cp };
        }
    }

    // SAFETY: `dev_name_list` is the C table of `dev_name_count` entries.
    let first = ptr::addr_of_mut!(glue::dev_name_list);
    let count = unsafe { glue::dev_name_count }.max(0) as usize;
    for i in 0..count {
        // SAFETY: `i` is inside the table.
        let dev = unsafe { first.add(i) };
        // SAFETY: the entry's name is a NUL-terminated string.
        if unsafe { name_equal(name, len, (*dev).d_name) } {
            // SAFETY: `dev` is a live table entry, never null.
            let dev = unsafe { NonNull::new_unchecked(dev) };
            // SAFETY: the name and the entry's fields are live.
            let unit =
                unsafe { subdev_unit(unit, (*dev.as_ptr()).d_subdev, c, cp) };
            return Some((dev, unit));
        }
    }

    // SAFETY: `dev_indirect_list` is the C table of `dev_indirect_count`
    // entries.
    let indirect = ptr::addr_of_mut!(glue::dev_indirect_list);
    let count = unsafe { glue::dev_indirect_count }.max(0) as usize;
    for i in 0..count {
        // SAFETY: `i` is inside the table.
        let di = unsafe { indirect.add(i) };
        // SAFETY: the entry's name is a NUL-terminated string.
        if unsafe { name_equal(name, len, (*di).d_name) } {
            // SAFETY: the entry's operation vector is live.
            let ops = unsafe { NonNull::new((*di).d_ops)? };
            return Some((ops, unsafe { (*di).d_unit }));
        }
    }

    None
}

/// `dev_set_indirection()` of `device/dev_name.c`.
///
/// # Safety
///
/// `name` must be a NUL-terminated string, and `ops` must be a live entry
/// point table.
pub(crate) unsafe fn set_indirection(
    name: *const c_char,
    ops: *mut DevOps,
    unit: c_int,
) {
    // SAFETY: `dev_indirect_list` is the C table of `dev_indirect_count`
    // entries.
    let list = ptr::addr_of_mut!(glue::dev_indirect_list);
    let count = unsafe { glue::dev_indirect_count }.max(0) as usize;
    for i in 0..count {
        // SAFETY: `i` is inside the table.
        let di = unsafe { list.add(i) };
        // SAFETY: both names are NUL-terminated strings.
        if unsafe { strcmp((*di).d_name, name) } == 0 {
            // SAFETY: the lock-free update is the C's own; the table is
            // initialized at boot.
            unsafe {
                (*di).d_ops = ops;
                (*di).d_unit = unit;
            }
            break;
        }
    }
}
