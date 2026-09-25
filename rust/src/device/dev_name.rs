// SPDX-License-Identifier: CMU-Mach
// Derived from device/dev_name.c and i386/i386at/conf.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The device-name lookup of `device/dev_name.c` and the AT device tables of
//! `i386/i386at/conf.c`, declared in <device/dev_hdr.h> and <device/conf.h>.

use crate::arch::i386::com_ffi::{
    comclose, comgetstat, comopen, comportdeath, comread, comsetstat, comwrite,
};
use crate::arch::i386::kd::tty::{
    kdclose, kdgetstat, kdmmap, kdopen, kdportdeath, kdread, kdsetstat,
    kdwrite,
};
use crate::arch::i386::kd_event::{
    kbdclose, kbdgetstat, kbdopen, kbdread, kbdsetstat,
};
use crate::arch::i386::kd_mouse::{
    mouseclose, mousegetstat, mouseopen, mouseread,
};
use crate::arch::i386::mbinfo::mbinforead;
use crate::arch::i386::mem::memmmap;
use crate::arch::i386::model_dep_ffi::timemmap;
use crate::device::dev_name_ffi::{
    nodev_async_in, nodev_info, nomap, nulldev_close, nulldev_getstat,
    nulldev_open, nulldev_portdeath, nulldev_read, nulldev_reset,
    nulldev_setstat, nulldev_write,
};
use crate::device::ds_routines::DevOps;
use crate::device::intr_ffi::irqgetstat;
use crate::device::kmsg_ffi::{kmsgclose, kmsggetstat, kmsgopen, kmsgread};
use crate::kern::mach_clock_ffi::{timeclose, timeopen};
use crate::utils::cell::SyncCell;
use crate::utils::string::strcmp;
use core::cell::UnsafeCell;
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

/// The entries of `dev_name_list[]` of `i386/i386at/conf.c`.
const DEV_NAME_COUNT: usize = 10;

/// The entries of `dev_indirect_list[]` of `i386/i386at/conf.c`.
const DEV_INDIRECT_COUNT: usize = 1;

/// `dev_name_list[]` of `i386/i386at/conf.c`: the major-device table
/// [`lookup`] searches.  Slot 0 is the console placeholder `cninit()` fills
/// through [`set_indirection`].
static DEV_NAME_LIST: SyncCell<[DevOps; DEV_NAME_COUNT]> =
    SyncCell(UnsafeCell::new([
        DevOps {
            d_name: c"cn".as_ptr().cast_mut(),
            d_open: Some(nulldev_open),
            d_close: Some(nulldev_close),
            d_read: Some(nulldev_read),
            d_write: Some(nulldev_write),
            d_getstat: Some(nulldev_getstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"kd".as_ptr().cast_mut(),
            d_open: Some(kdopen),
            d_close: Some(kdclose),
            d_read: Some(kdread),
            d_write: Some(kdwrite),
            d_getstat: Some(kdgetstat),
            d_setstat: Some(kdsetstat),
            d_mmap: Some(kdmmap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(kdportdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"time".as_ptr().cast_mut(),
            d_open: Some(timeopen),
            d_close: Some(timeclose),
            d_read: Some(nulldev_read),
            d_write: Some(nulldev_write),
            d_getstat: Some(nulldev_getstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(timemmap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"com".as_ptr().cast_mut(),
            d_open: Some(comopen),
            d_close: Some(comclose),
            d_read: Some(comread),
            d_write: Some(comwrite),
            d_getstat: Some(comgetstat),
            d_setstat: Some(comsetstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(comportdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"mouse".as_ptr().cast_mut(),
            d_open: Some(mouseopen),
            d_close: Some(mouseclose),
            d_read: Some(mouseread),
            d_write: Some(nulldev_write),
            d_getstat: Some(mousegetstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"kbd".as_ptr().cast_mut(),
            d_open: Some(kbdopen),
            d_close: Some(kbdclose),
            d_read: Some(kbdread),
            d_write: Some(nulldev_write),
            d_getstat: Some(kbdgetstat),
            d_setstat: Some(kbdsetstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"mem".as_ptr().cast_mut(),
            d_open: Some(nulldev_open),
            d_close: Some(nulldev_close),
            d_read: Some(nulldev_read),
            d_write: Some(nulldev_write),
            d_getstat: Some(nulldev_getstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(memmmap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"kmsg".as_ptr().cast_mut(),
            d_open: Some(kmsgopen),
            d_close: Some(kmsgclose),
            d_read: Some(kmsgread),
            d_write: Some(nulldev_write),
            d_getstat: Some(kmsggetstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"irq".as_ptr().cast_mut(),
            d_open: Some(nulldev_open),
            d_close: Some(nulldev_close),
            d_read: Some(nulldev_read),
            d_write: Some(nulldev_write),
            d_getstat: Some(irqgetstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
        DevOps {
            d_name: c"mbinfo".as_ptr().cast_mut(),
            d_open: Some(nulldev_open),
            d_close: Some(nulldev_close),
            d_read: Some(mbinforead),
            d_write: Some(nulldev_write),
            d_getstat: Some(nulldev_getstat),
            d_setstat: Some(nulldev_setstat),
            d_mmap: Some(nomap),
            d_async_in: Some(nodev_async_in),
            d_reset: Some(nulldev_reset),
            d_port_death: Some(nulldev_portdeath),
            d_subdev: 0,
            d_dev_info: Some(nodev_info),
        },
    ]));

/// `dev_indirect_list[]` of `i386/i386at/conf.c`: the indirect-device table
/// [`lookup`] falls back to.  `cninit()` rewrites the console entry's
/// operations through [`set_indirection`].
static DEV_INDIRECT_LIST: SyncCell<[DevIndirect; DEV_INDIRECT_COUNT]> =
    SyncCell(UnsafeCell::new([DevIndirect {
        d_name: c"console".as_ptr().cast_mut(),
        d_ops: ptr::addr_of!(DEV_NAME_LIST.0).cast_mut().cast::<DevOps>(),
        d_unit: 0,
    }]));

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

    // SAFETY: the table is boot-initialized and only read from here.
    let first = DEV_NAME_LIST.0.get().cast::<DevOps>();
    for i in 0..DEV_NAME_COUNT {
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

    // SAFETY: the table is boot-initialized and only read from here.
    let indirect = DEV_INDIRECT_LIST.0.get().cast::<DevIndirect>();
    for i in 0..DEV_INDIRECT_COUNT {
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
    // SAFETY: the table is boot-initialized and only written here.
    let list = DEV_INDIRECT_LIST.0.get().cast::<DevIndirect>();
    for i in 0..DEV_INDIRECT_COUNT {
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
