// SPDX-License-Identifier: CMU-Mach
// Derived from device/chario.c, device/tty.h and include/device/tty_status.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
//   Copyright (c) 1993-1990 Carnegie Mellon University.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The tty line discipline of `device/chario.c`, the `struct tty` of
//! <device/tty.h> and the `struct tty_status` of <device/tty_status.h>.

use crate::arch::i386::io_req::{D_NOWAIT, IoDone, IoReq};
use crate::arch::types::{VmOffset, VmSize};
use crate::device::cirbuf::{self, Cirbuf};
use crate::device::ds_routines;
use crate::device::r#return::{DeviceError, DeviceSuccess};
use crate::glue;
use crate::ipc::IpcPort;
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock;
use crate::kern::queue::{QueueEntry, enqueue_tail};
use crate::vm::error::KERN_SUCCESS;
use crate::vm::vm_map::VmMapCopy;
use crate::vm::vm_map_ffi::vm_map_copyout;
use crate::vm::vm_user_ffi::vm_deallocate;
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::pin::Pin;
use core::ptr::{self, NonNull};
use core::slice;

/// `NSPEEDS` of <device/tty_status.h>: how many baud-rate slots the `tt*` and
/// `pdma_*` tables have.
pub(crate) const NSPEEDS: usize = 18;

/// The `B*` speed indices of <device/tty_status.h>, the rows of the `pdma_*`
/// tables.
const B0: usize = 0;
const B300: usize = 7;
const B600: usize = 8;
const B1200: usize = 9;
const B1800: usize = 10;
const B2400: usize = 11;
const B4800: usize = 12;
const B9600: usize = 13;
const EXTA: usize = 14;
const EXTB: usize = 15;
const B57600: usize = 16;
const B115200: usize = 17;

/// `TS_*` of <device/tty.h>.
pub(crate) const TS_INIT: c_int = 0x0000_0001;
pub(crate) const TS_TIMEOUT: c_int = 0x0000_0002;
pub(crate) const TS_WOPEN: c_int = 0x0000_0004;
pub(crate) const TS_ISOPEN: c_int = 0x0000_0008;
pub(crate) const TS_FLUSH: c_int = 0x0000_0010;
pub(crate) const TS_CARR_ON: c_int = 0x0000_0020;
pub(crate) const TS_BUSY: c_int = 0x0000_0040;
pub(crate) const TS_TTSTOP: c_int = 0x0000_0100;
pub(crate) const TS_HUPCLS: c_int = 0x0000_0200;
const TS_ONDELAY: c_int = 0x0000_2000;
pub(crate) const TS_MIN: c_int = 0x0000_4000;
const TS_MIN_TO: c_int = 0x0000_8000;
const TS_RTS_DOWN: c_int = 0x0002_0000;
const TS_MIN_TO_RCV: c_int = 0x0040_0000;

/// `TF_*` of <device/tty_status.h>.
pub(crate) const TF_ODDP: c_int = 0x0000_0002;
pub(crate) const TF_EVENP: c_int = 0x0000_0004;
pub(crate) const TF_LITOUT: c_int = 0x0000_0008;
const TF_MDMBUF: c_int = 0x0000_0010;
const TF_NOHANG: c_int = 0x0000_0020;
const TF_HUPCLS: c_int = 0x0000_0040;
pub(crate) const TF_ECHO: c_int = 0x0000_0080;
pub(crate) const TF_CRMOD: c_int = 0x0000_0100;
pub(crate) const TF_XTABS: c_int = 0x0000_0200;

/// `DMSET`, `DMBIS`, `DMBIC` and `DMGET` of <device/tty.h>.
pub(crate) const DMSET: c_int = 0;
pub(crate) const DMBIS: c_int = 1;
pub(crate) const DMBIC: c_int = 2;
pub(crate) const DMGET: c_int = 3;

/// `TM_*` modem signals of <device/tty_status.h>.
pub(crate) const TM_DTR: c_int = 0x0002;
pub(crate) const TM_RTS: c_int = 0x0004;
pub(crate) const TM_CTS: c_int = 0x0020;
pub(crate) const TM_CAR: c_int = 0x0040;
pub(crate) const TM_RNG: c_int = 0x0080;
pub(crate) const TM_DSR: c_int = 0x0100;
pub(crate) const TM_BRK: c_int = 0x0200;
pub(crate) const TM_HUP: c_int = 0;

/// `D_READ`, `D_WRITE` and `D_NODELAY` of <device/device_types.h>.
pub(crate) const D_READ: c_int = 0x1;
pub(crate) const D_WRITE: c_int = 0x2;
const D_NODELAY: c_uint = 0x4;

/// `IO_INBAND` of <device/io_req.h>.
const IO_INBAND: c_int = 0x0000_4000;

/// One row of the PDMA tick table: a speed's `B*` index and the baud rate its
/// timeout divides `hz` by.
const PDMA_TIMEOUT_ROWS: [(usize, c_int); 11] = [
    (B300, 30),
    (B600, 60),
    (B1200, 120),
    (B1800, 180),
    (B2400, 240),
    (B4800, 480),
    (B9600, 960),
    (EXTA, 1440),
    (EXTB, 1920),
    (B57600, 5760),
    (B115200, 11520),
];

/// The slow speeds' water marks, the integers `device/chario.c`'s floating
/// point expressions truncate to.
const PDMA_SLOW_MARK_ROWS: [(usize, c_int); 6] = [
    (B300, 24),
    (B600, 24),
    (B1200, 24),
    (B1800, 36),
    (B2400, 48),
    (B4800, 96),
];

/// The speeds that buffer half the input queue instead.
const PDMA_FAST_SPEEDS: [usize; 5] = [B9600, EXTA, EXTB, B57600, B115200];

const _: () = assert!(B115200 < NSPEEDS);

/// `struct tty` of <device/tty.h>, field for field.
#[repr(C)]
pub struct Tty {
    pub(crate) t_lock: SimpleLock,
    pub(crate) t_inq: Cirbuf,
    pub(crate) t_outq: Cirbuf,
    pub(crate) t_addr: Option<NonNull<c_char>>,
    pub(crate) t_dev: c_int,
    pub(crate) t_start: Option<unsafe extern "C" fn(*mut Tty)>,
    pub(crate) t_stop: Option<unsafe extern "C" fn(*mut Tty, c_int)>,
    pub(crate) t_mctl:
        Option<unsafe extern "C" fn(*mut Tty, c_int, c_int) -> c_int>,
    pub(crate) t_ispeed: u8,
    pub(crate) t_ospeed: u8,
    pub(crate) t_breakc: c_char,
    pub(crate) t_flags: c_int,
    pub(crate) t_state: c_int,
    pub(crate) t_line: c_int,
    pub(crate) t_delayed_read: QueueEntry,
    pub(crate) t_delayed_write: QueueEntry,
    pub(crate) t_delayed_open: QueueEntry,
    t_timeout: Option<NonNull<mach_clock::Timeout>>,
    pub(crate) t_getstat: Option<
        unsafe extern "C" fn(u16, c_uint, *mut c_int, *mut u32) -> c_int,
    >,
    pub(crate) t_setstat:
        Option<unsafe extern "C" fn(u16, c_uint, *mut c_int, u32) -> c_int>,
    t_tops: Option<NonNull<c_void>>,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Tty>() == 224);
    assert!(align_of::<Tty>() == align_of::<*mut c_void>());
    assert!(offset_of!(Tty, t_lock) == 0);
    assert!(offset_of!(Tty, t_inq) == 8);
    assert!(offset_of!(Tty, t_outq) == 48);
    assert!(offset_of!(Tty, t_addr) == 88);
    assert!(offset_of!(Tty, t_dev) == 96);
    assert!(offset_of!(Tty, t_start) == 104);
    assert!(offset_of!(Tty, t_stop) == 112);
    assert!(offset_of!(Tty, t_mctl) == 120);
    assert!(offset_of!(Tty, t_ispeed) == 128);
    assert!(offset_of!(Tty, t_ospeed) == 129);
    assert!(offset_of!(Tty, t_breakc) == 130);
    assert!(offset_of!(Tty, t_flags) == 132);
    assert!(offset_of!(Tty, t_state) == 136);
    assert!(offset_of!(Tty, t_line) == 140);
    assert!(offset_of!(Tty, t_delayed_read) == 144);
    assert!(offset_of!(Tty, t_delayed_write) == 160);
    assert!(offset_of!(Tty, t_delayed_open) == 176);
    assert!(offset_of!(Tty, t_timeout) == 192);
    assert!(offset_of!(Tty, t_getstat) == 200);
    assert!(offset_of!(Tty, t_setstat) == 208);
    assert!(offset_of!(Tty, t_tops) == 216);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Tty>() == 120);
    assert!(align_of::<Tty>() == align_of::<*mut c_void>());
    assert!(offset_of!(Tty, t_lock) == 0);
    assert!(offset_of!(Tty, t_inq) == 4);
    assert!(offset_of!(Tty, t_outq) == 24);
    assert!(offset_of!(Tty, t_addr) == 44);
    assert!(offset_of!(Tty, t_dev) == 48);
    assert!(offset_of!(Tty, t_start) == 52);
    assert!(offset_of!(Tty, t_stop) == 56);
    assert!(offset_of!(Tty, t_mctl) == 60);
    assert!(offset_of!(Tty, t_ispeed) == 64);
    assert!(offset_of!(Tty, t_ospeed) == 65);
    assert!(offset_of!(Tty, t_breakc) == 66);
    assert!(offset_of!(Tty, t_flags) == 68);
    assert!(offset_of!(Tty, t_state) == 72);
    assert!(offset_of!(Tty, t_line) == 76);
    assert!(offset_of!(Tty, t_delayed_read) == 80);
    assert!(offset_of!(Tty, t_delayed_write) == 88);
    assert!(offset_of!(Tty, t_delayed_open) == 96);
    assert!(offset_of!(Tty, t_timeout) == 104);
    assert!(offset_of!(Tty, t_getstat) == 108);
    assert!(offset_of!(Tty, t_setstat) == 112);
    assert!(offset_of!(Tty, t_tops) == 116);
};

impl Tty {
    /// The zero image a C `static` began with, ready for [`chars`].
    pub(crate) const fn new() -> Self {
        Self {
            t_lock: SimpleLock::new(),
            t_inq: Cirbuf::new(),
            t_outq: Cirbuf::new(),
            t_addr: None,
            t_dev: 0,
            t_start: None,
            t_stop: None,
            t_mctl: None,
            t_ispeed: 0,
            t_ospeed: 0,
            t_breakc: 0,
            t_flags: 0,
            t_state: 0,
            t_line: 0,
            t_delayed_read: QueueEntry::unlinked(),
            t_delayed_write: QueueEntry::unlinked(),
            t_delayed_open: QueueEntry::unlinked(),
            t_timeout: None,
            t_getstat: None,
            t_setstat: None,
            t_tops: None,
        }
    }
}

/// `struct tty_status` of <device/tty_status.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TtyStatus {
    pub(crate) tt_ispeed: c_int,
    pub(crate) tt_ospeed: c_int,
    pub(crate) tt_breakc: c_int,
    pub(crate) tt_flags: c_int,
}

const _: () = {
    assert!(size_of::<TtyStatus>() == 16);
    assert!(align_of::<TtyStatus>() == align_of::<c_int>());
    assert!(offset_of!(TtyStatus, tt_ispeed) == 0);
    assert!(offset_of!(TtyStatus, tt_ospeed) == 4);
    assert!(offset_of!(TtyStatus, tt_breakc) == 8);
    assert!(offset_of!(TtyStatus, tt_flags) == 12);
};

/// `struct ldisc_switch` of <device/tty.h>: the entry points one line
/// discipline provides.
#[repr(C)]
pub(crate) struct LdiscSwitch {
    pub(crate) l_read:
        Option<unsafe extern "C" fn(*mut Tty, *mut IoReq) -> c_int>,
    pub(crate) l_write:
        Option<unsafe extern "C" fn(*mut Tty, *mut IoReq) -> c_int>,
    pub(crate) l_rint: Option<unsafe extern "C" fn(c_uint, *mut Tty)>,
    pub(crate) l_modem: Option<unsafe extern "C" fn(*mut Tty, c_int) -> c_int>,
    pub(crate) l_start: Option<unsafe extern "C" fn(*mut Tty)>,
}

const _: () = {
    const PTR: usize = size_of::<*const c_void>();
    assert!(size_of::<LdiscSwitch>() == 5 * PTR);
    assert!(align_of::<LdiscSwitch>() == align_of::<*const c_void>());
    assert!(offset_of!(LdiscSwitch, l_read) == 0);
    assert!(offset_of!(LdiscSwitch, l_write) == PTR);
    assert!(offset_of!(LdiscSwitch, l_rint) == 2 * PTR);
    assert!(offset_of!(LdiscSwitch, l_modem) == 3 * PTR);
    assert!(offset_of!(LdiscSwitch, l_start) == 4 * PTR);
};

/// `tthiwat[]` of <device/tty.h>, read through the `TTHIWAT()` macro.
#[unsafe(export_name = "tthiwat")]
static TTHIWAT: [c_short; NSPEEDS] = [
    100, 100, 100, 100, 100, 100, 100, 200, 200, 400, 400, 400, 650, 650,
    1300, 2000, 2000, 2000,
];

/// `ttlowat[]` of <device/tty.h>, read through the `TTLOWAT()` macro and by
/// the C `com.c` driver.
#[unsafe(export_name = "ttlowat")]
pub(crate) static TTLOWAT: [c_short; NSPEEDS] = [
    30, 30, 30, 30, 30, 30, 30, 50, 50, 120, 120, 120, 125, 125, 125, 125,
    125, 125,
];

/// `tty_inq_size` of device/chario.c: the input buffer's allocation.
#[unsafe(export_name = "tty_inq_size")]
static TTY_INQ_SIZE: c_uint = 4096;

/// `tty_outq_size` of device/chario.c: the output buffer's allocation.
#[unsafe(export_name = "tty_outq_size")]
static TTY_OUTQ_SIZE: c_uint = 2048;

/// `pdma_default` of device/chario.c: pseudo-DMA on for every line.
#[unsafe(export_name = "pdma_default")]
static PDMA_DEFAULT: c_int = 1;

/// `pdma_timeouts[]` of device/chario.c: ticks per speed's receive timeout.
#[unsafe(export_name = "pdma_timeouts")]
static mut PDMA_TIMEOUTS: [c_int; NSPEEDS] = [0; NSPEEDS];

/// `pdma_water_mark[]` of device/chario.c: input-queue water marks.
#[unsafe(export_name = "pdma_water_mark")]
static mut PDMA_WATER_MARK: [c_int; NSPEEDS] = [0; NSPEEDS];

/// The error families a tty reply can carry: the device layer's `D_*` codes,
/// or a `kern_return_t` the character copy passed through unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TtyError {
    /// A `kern_return_t` from `vm_map_copyout()` or `device_read_alloc()`,
    /// returned raw because the C returned it unchanged.
    Kern(c_int),
    /// A `D_*` error of <device/device_types.h>.
    Device(DeviceError),
}

impl TtyError {
    /// The `io_return_t` the C caller sees.
    pub(crate) const fn as_io_return(self) -> c_int {
        match self {
            TtyError::Kern(code) => code,
            TtyError::Device(error) => error as c_int,
        }
    }
}

/// What a tty operation reports: the `D_SUCCESS`/`D_IO_QUEUED` codes, or a
/// [`TtyError`].
pub(crate) type TtyResult = Result<DeviceSuccess, TtyError>;

/// The `io_return_t` a [`TtyResult`] denotes.
pub(crate) fn io_return(result: TtyResult) -> c_int {
    match result {
        Ok(DeviceSuccess::Success) => 0,
        Ok(DeviceSuccess::IoQueued) => -1,
        Err(error) => error.as_io_return(),
    }
}

/// The table row `speed` names, or `None` when it is outside the tables.
fn speed_row(speed: c_int) -> Option<u8> {
    let row = u8::try_from(speed).ok()?;
    if usize::from(row) < NSPEEDS {
        Some(row)
    } else {
        None
    }
}

/// `TTHIWAT(tp)` of <device/tty.h>.
fn high_water(tp: &Tty) -> c_short {
    TTHIWAT.get(usize::from(tp.t_ospeed)).copied().unwrap_or(0)
}

/// `TTLOWAT(tp)` of <device/tty.h>.
pub(crate) fn low_water(tp: &Tty) -> c_short {
    TTLOWAT.get(usize::from(tp.t_ospeed)).copied().unwrap_or(0)
}

/// `pdma_timeouts[speed]` of device/chario.c.
fn pdma_timeout(speed: u8) -> c_int {
    let index = usize::from(speed);
    if index < NSPEEDS {
        // SAFETY: `chario_init()` is the only writer and runs at boot before
        // any tty is open; tty code reads the table under the tty lock.
        unsafe { ptr::addr_of!(PDMA_TIMEOUTS[index]).read() }
    } else {
        0
    }
}

/// `pdma_water_mark[speed]` of device/chario.c.
fn pdma_water(speed: u8) -> c_int {
    let index = usize::from(speed);
    if index < NSPEEDS {
        // SAFETY: as [`pdma_timeout()`]: the table is written once at boot.
        unsafe { ptr::addr_of!(PDMA_WATER_MARK[index]).read() }
    } else {
        0
    }
}

/// Take `tp`'s lock at `splhigh`, as the `simple_lock_irq()` macro did.
fn lock_irq(tp: &Tty) -> c_int {
    // SAFETY: `splhigh()` is the asm entry of <machine/spl.h>.
    let level = unsafe { glue::splhigh() };
    tp.t_lock.lock();
    level
}

/// Release `tp`'s lock and restore `level`, as `simple_unlock_irq()` did.
fn unlock_irq(tp: &Tty, level: c_int) {
    tp.t_lock.unlock();
    // SAFETY: `level` is the value [`lock_irq()`] returned for this tty.
    unsafe { glue::splx(level) };
}

/// `queue_delayed_reply()` of device/chario.c.
///
/// # Safety
///
/// `head` must be an initialized queue head that stays at its address, `ior`
/// must stay at its address until the queue completes it, and the caller must
/// hold the tty lock.
pub(crate) unsafe fn queue_delayed_reply(
    head: *mut QueueEntry,
    ior: &mut IoReq,
    done: IoDone,
) {
    ior.set_done(done);
    // SAFETY: the caller promises `head` is stable and `ior` stays put; the
    // request's first two fields are its queue links.
    unsafe { enqueue_tail(head, ptr::from_mut(ior).cast::<QueueEntry>()) };
}

/// Empty every `io_req` on `head` through `iodone()`.
///
/// # Safety
///
/// `head` must be an initialized queue head that stays at its address, every
/// entry must be an `io_req`, and nothing else may access the queue during
/// the call.
unsafe fn complete(mut head: Pin<&mut QueueEntry>) {
    loop {
        // SAFETY: the caller's contract holds on every iteration.
        let Some(entry) = (unsafe { head.as_mut().pop_front() }) else {
            return;
        };
        // SAFETY: every entry of these queues is an `io_req`, whose chain is
        // its first two fields.
        unsafe { ds_routines::iodone(entry.as_ptr().cast::<IoReq>()) };
    }
}

/// `tty_queue_completion()` of device/chario.c.
///
/// # Safety
///
/// `queue` must be an initialized queue head that stays at its address while
/// entries are linked, every entry must be an `io_req`, and nothing else may
/// access the queue during the call.
pub(crate) unsafe fn complete_queue(queue: *mut QueueEntry) {
    let Some(queue) = NonNull::new(queue) else {
        return;
    };
    // SAFETY: the caller promises a valid, stable head with `io_req` entries.
    let head = unsafe { QueueEntry::pin_in_place(queue) };
    // SAFETY: as above.
    unsafe { complete(head) };
}

/// Drain one of `ttyclose()`'s delayed queues, recording `done` on each
/// request before completing it.
///
/// # Safety
///
/// Same contract as [`complete()`], and the caller must hold the tty lock.
unsafe fn drain(head: *mut QueueEntry, done: IoDone) {
    let Some(head) = NonNull::new(head) else {
        return;
    };
    // SAFETY: the caller promises a valid, stable head with `io_req` entries.
    let mut head = unsafe { QueueEntry::pin_in_place(head) };
    loop {
        // SAFETY: the caller's contract holds on every iteration.
        let Some(entry) = (unsafe { head.as_mut().pop_front() }) else {
            return;
        };
        let ior = entry.as_ptr().cast::<IoReq>();
        // SAFETY: every entry is an `io_req`, and `iodone()` completes it
        // once.
        unsafe {
            (*ior).done = Some(done);
            ds_routines::iodone(ior);
        }
    }
}

/// The first request on `head` whose reply port is `port`, completed through
/// `done`.
///
/// # Safety
///
/// `head` must be an initialized queue of `io_req` entries the caller holds
/// the tty lock over, and nothing else may access it during the call.
unsafe fn clean_queue(
    head: &QueueEntry,
    port: *mut c_void,
    done: IoDone,
) -> bool {
    for entry in head.iter() {
        let ior = entry.as_ptr().cast::<IoReq>();
        // SAFETY: every entry of the delayed queues is an `io_req`.
        if unsafe { (*ior).reply_port } != port {
            continue;
        }
        // SAFETY: `entry` is linked into a queue that stays at its address.
        unsafe { QueueEntry::remove(QueueEntry::pin_in_place(entry)) };
        // SAFETY: the entry is an `io_req`; `iodone()` completes it once.
        unsafe {
            (*ior).done = Some(done);
            ds_routines::iodone(ior);
        }
        return true;
    }
    false
}

/// `tty_close_open_reply()` of device/chario.c.
unsafe extern "C" fn tty_close_open_reply(ior: *mut IoReq) -> c_int {
    // SAFETY: `iodone()` passes back the live request it was queued with.
    unsafe {
        (*ior).error = DeviceError::DeviceDown as c_int;
        ds_routines::ds_open_done(ior);
    }
    c_int::from(true)
}

/// `tty_close_write_reply()` of device/chario.c.
unsafe extern "C" fn tty_close_write_reply(ior: *mut IoReq) -> c_int {
    // SAFETY: `iodone()` passes back the live request it was queued with.
    unsafe {
        (*ior).residual = (*ior).count;
        (*ior).error = DeviceError::DeviceDown as c_int;
        ds_routines::ds_write_done(ior);
    }
    c_int::from(true)
}

/// `tty_close_read_reply()` of device/chario.c.
unsafe extern "C" fn tty_close_read_reply(ior: *mut IoReq) -> c_int {
    // SAFETY: `iodone()` passes back the live request it was queued with.
    unsafe {
        (*ior).residual = (*ior).count;
        (*ior).error = DeviceError::DeviceDown as c_int;
        ds_routines::ds_read_done(ior);
    }
    c_int::from(true)
}

/// Open the tty, queueing the request when the carrier is not up yet.
pub(crate) fn open(
    tp: &mut Tty,
    dev: c_int,
    mode: c_uint,
    ior: &mut IoReq,
) -> TtyResult {
    let level = lock_irq(tp);
    tp.t_dev = dev;

    if let Some(mctl) = tp.t_mctl {
        // SAFETY: the driver's modem-control routine takes its own tty; the
        // tty lock is held, as its contract requires.
        unsafe { mctl(ptr::from_mut(tp), TM_DTR, DMSET) };
    }

    if PDMA_DEFAULT != 0 {
        tp.t_state |= TS_MIN;
    }

    if tp.t_state & TS_CARR_ON == 0 {
        if mode & D_NODELAY != 0 {
            tp.t_state |= TS_ONDELAY;
        } else {
            tp.t_state |= TS_WOPEN;
            ior.dev_ptr = ptr::from_mut(tp).cast::<c_char>();
            // SAFETY: the open queue is the tty's and stays at its address;
            // the request stays live until `char_open_done()` completes it.
            unsafe {
                queue_delayed_reply(
                    ptr::from_mut(&mut tp.t_delayed_open),
                    ior,
                    char_open_done,
                )
            };
            unlock_irq(tp, level);
            return Ok(DeviceSuccess::IoQueued);
        }
    }

    tp.t_state |= TS_ISOPEN;
    if let Some(mctl) = tp.t_mctl {
        // SAFETY: as the DTR call above.
        unsafe { mctl(ptr::from_mut(tp), TM_RTS, DMBIS) };
    }

    unlock_irq(tp, level);
    Ok(DeviceSuccess::Success)
}

/// `char_open_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live open request `open()` queued on a tty.
pub(crate) unsafe extern "C" fn char_open_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    let ior = unsafe { &mut *ior };
    // SAFETY: `open()` set `dev_ptr` to the tty that stays live.
    let tp = unsafe { &mut *ior.dev_ptr.cast::<Tty>() };
    let level = lock_irq(tp);

    if tp.t_state & TS_ISOPEN == 0 {
        // SAFETY: the open queue is the tty's and stays at its address.
        unsafe {
            queue_delayed_reply(
                ptr::from_mut(&mut tp.t_delayed_open),
                ior,
                char_open_done,
            )
        };
        unlock_irq(tp, level);
        return c_int::from(false);
    }

    tp.t_state |= TS_ISOPEN;
    tp.t_state &= !TS_WOPEN;
    if let Some(mctl) = tp.t_mctl {
        // SAFETY: the driver's modem-control routine takes its own tty; the
        // tty lock is held.
        unsafe { mctl(ptr::from_mut(tp), TM_RTS, DMBIS) };
    }

    unlock_irq(tp, level);

    ior.error = 0;
    // SAFETY: `ds_open_done()`'s contract; `device_open()` built the request.
    unsafe { ds_routines::ds_open_done(ptr::from_mut(ior)) };
    c_int::from(true)
}

/// Write the request's data to the tty's output queue.
pub(crate) fn write(tp: &mut Tty, ior: &mut IoReq) -> TtyResult {
    let count = ior.count;
    if count == 0 {
        return Ok(DeviceSuccess::Success);
    }

    let inband = ior.op & IO_INBAND != 0;
    let mut addr: VmOffset = 0;
    let mut data = ior.data;
    if !inband {
        // SAFETY: the request's data is a live `vm_map_copy`, `device_io_map`
        // is the boot map `ds_routines` owns, and `addr` is writable.
        let kr = unsafe {
            vm_map_copyout(
                ds_routines::device_io_map,
                &mut addr,
                data.cast::<VmMapCopy>(),
            )
        };
        if kr != KERN_SUCCESS {
            return Err(TtyError::Kern(kr));
        }
        // SAFETY: `vm_map_copyout()` mapped the copy's bytes at `addr` on
        // success.
        data = ptr::with_exposed_provenance_mut(addr);
    }

    let level = lock_irq(tp);

    let result =
        if tp.t_state & TS_CARR_ON == 0 && tp.t_state & TS_ONDELAY == 0 {
            Err(TtyError::Device(DeviceError::IoError))
        } else if tp.t_state & TS_CARR_ON == 0 && ior.mode & D_NOWAIT != 0 {
            Err(TtyError::Device(DeviceError::WouldBlock))
        } else {
            write_output(tp, ior, data, count)
        };

    unlock_irq(tp, level);

    if !inband {
        // SAFETY: the copyout above mapped `count` bytes at `addr` in
        // `device_io_map`, and the request still owns them.
        unsafe {
            vm_deallocate(ds_routines::device_io_map, addr, count as VmSize)
        };
    }

    result
}

/// The output path of `write()`, the tty lock held and the carrier checked.
fn write_output(
    tp: &mut Tty,
    ior: &mut IoReq,
    data: *mut c_char,
    count: c_long,
) -> TtyResult {
    // A negative `io_count` is outside the device layer's contract and the
    // ported `b_to_q` treated it as empty; `unwrap_or(0)` keeps that.
    let len = usize::try_from(count).unwrap_or(0);
    // SAFETY: the caller guarantees `len` readable bytes at `data`: the
    // inband buffer or the copyout mapping.
    let input = unsafe { slice::from_raw_parts(data.cast::<u8>(), len) };
    let entered = tp.t_outq.write(input);
    // The ported `b_to_q()` clamped a negative count to zero, and the
    // residual the C stored came from the clamped value.
    ior.residual = len as c_long - entered as c_long;

    tp.t_state &= !TS_TTSTOP;
    start(tp);

    if tp.t_outq.count() > high_water(tp) || tp.t_state & TS_CARR_ON == 0 {
        ior.dev_ptr = ptr::from_mut(tp).cast::<c_char>();
        // SAFETY: the write queue is the tty's and stays at its address; the
        // request stays live until `char_write_done()` completes it.
        unsafe {
            queue_delayed_reply(
                ptr::from_mut(&mut tp.t_delayed_write),
                ior,
                char_write_done,
            )
        };
        return Ok(DeviceSuccess::IoQueued);
    }
    Ok(DeviceSuccess::Success)
}

/// `char_write_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live write request `write()` queued on a tty.
pub(crate) unsafe extern "C" fn char_write_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    let ior = unsafe { &mut *ior };
    // SAFETY: `write_output()` set `dev_ptr` to the tty that stays live.
    let tp = unsafe { &mut *ior.dev_ptr.cast::<Tty>() };
    let level = lock_irq(tp);

    if tp.t_outq.count() > high_water(tp) || tp.t_state & TS_CARR_ON == 0 {
        // SAFETY: the write queue is the tty's and stays at its address.
        unsafe {
            queue_delayed_reply(
                ptr::from_mut(&mut tp.t_delayed_write),
                ior,
                char_write_done,
            )
        };
        unlock_irq(tp, level);
        return c_int::from(false);
    }

    unlock_irq(tp, level);

    if let Some(port) = IpcPort::valid(ior.reply_port) {
        // The C narrowed the long difference to the reply's `int` count.
        let bytes = (ior.total - ior.residual) as c_int;
        if ior.op & IO_INBAND != 0 {
            // SAFETY: the generated reply stub takes the live reply port.
            unsafe {
                glue::ds_device_write_reply_inband(
                    port.as_ptr(),
                    ior.reply_port_type,
                    ior.error,
                    bytes,
                )
            };
        } else {
            // SAFETY: as the inband stub; the reply port is live and the
            // request owns its reply.
            unsafe {
                glue::ds_device_write_reply(
                    port.as_ptr(),
                    ior.reply_port_type,
                    ior.error,
                    bytes,
                )
            };
        }
    }
    // SAFETY: `device_reference()`/`device_deallocate()`'s contract; the
    // request owns its reference on the device.
    unsafe { crate::device::dev_lookup::deallocate(ior.device.cast()) };
    c_int::from(true)
}

/// Read the tty's input queue into the request's data buffer.
pub(crate) fn read(tp: &mut Tty, ior: &mut IoReq) -> TtyResult {
    // The C narrowed `io_count` to `vm_size_t`; the device layer sizes the
    // count, so the conversion cannot lose anything that matters.
    let size = ior.count as VmSize;
    // SAFETY: `device_read_alloc()`'s contract; the request is live.
    let kr =
        unsafe { ds_routines::device_read_alloc(ptr::from_mut(ior), size) };
    if kr != KERN_SUCCESS {
        return Err(TtyError::Kern(kr));
    }

    let level = lock_irq(tp);

    if tp.t_state & TS_CARR_ON == 0 && tp.t_state & TS_ONDELAY == 0 {
        unlock_irq(tp, level);
        return Err(TtyError::Device(DeviceError::IoError));
    }
    if tp.t_state & TS_CARR_ON == 0 && ior.mode & D_NOWAIT != 0 {
        unlock_irq(tp, level);
        return Err(TtyError::Device(DeviceError::WouldBlock));
    }

    if tp.t_inq.count() <= 0 || tp.t_state & TS_CARR_ON == 0 {
        ior.dev_ptr = ptr::from_mut(tp).cast::<c_char>();
        // SAFETY: the read queue is the tty's and stays at its address; the
        // request stays live until `char_read_done()` completes it.
        unsafe {
            queue_delayed_reply(
                ptr::from_mut(&mut tp.t_delayed_read),
                ior,
                char_read_done,
            )
        };
        unlock_irq(tp, level);
        return Ok(DeviceSuccess::IoQueued);
    }

    let out = if ior.count <= 0 {
        None
    } else {
        // SAFETY: `device_read_alloc()` allocated `io_count` writable bytes at
        // `io_data` above.
        Some(unsafe {
            slice::from_raw_parts_mut(
                ior.data.cast::<u8>(),
                ior.count as usize,
            )
        })
    };
    let copied = match out {
        Some(out) => tp.t_inq.read(out),
        None => 0,
    };
    ior.residual = ior.count - copied as c_long;

    if tp.t_state & TS_RTS_DOWN != 0 {
        if let Some(mctl) = tp.t_mctl {
            // SAFETY: the driver's modem-control routine takes its own tty;
            // the tty lock is held.
            unsafe { mctl(ptr::from_mut(tp), TM_RTS, DMBIS) };
        }
        tp.t_state &= !TS_RTS_DOWN;
    }

    unlock_irq(tp, level);
    Ok(DeviceSuccess::Success)
}

/// `char_read_done()` of device/chario.c.
///
/// # Safety
///
/// `ior` is the live read request `read()` queued on a tty.
pub(crate) unsafe extern "C" fn char_read_done(ior: *mut IoReq) -> c_int {
    // SAFETY: the caller promises the live request.
    let ior = unsafe { &mut *ior };
    // SAFETY: `read()` set `dev_ptr` to the tty that stays live.
    let tp = unsafe { &mut *ior.dev_ptr.cast::<Tty>() };
    let level = lock_irq(tp);

    if tp.t_inq.count() <= 0 || tp.t_state & TS_CARR_ON == 0 {
        // SAFETY: the read queue is the tty's and stays at its address.
        unsafe {
            queue_delayed_reply(
                ptr::from_mut(&mut tp.t_delayed_read),
                ior,
                char_read_done,
            )
        };
        unlock_irq(tp, level);
        return c_int::from(false);
    }

    let out = if ior.count <= 0 {
        None
    } else {
        // SAFETY: `device_read_alloc()` allocated `io_count` writable bytes at
        // `io_data` when the request was queued.
        Some(unsafe {
            slice::from_raw_parts_mut(
                ior.data.cast::<u8>(),
                ior.count as usize,
            )
        })
    };
    let copied = match out {
        Some(out) => tp.t_inq.read(out),
        None => 0,
    };
    ior.residual = ior.count - copied as c_long;

    if tp.t_state & TS_RTS_DOWN != 0 {
        if let Some(mctl) = tp.t_mctl {
            // SAFETY: the driver's modem-control routine takes its own tty;
            // the tty lock is held.
            unsafe { mctl(ptr::from_mut(tp), TM_RTS, DMBIS) };
        }
        tp.t_state &= !TS_RTS_DOWN;
    }

    unlock_irq(tp, level);

    // SAFETY: `ds_read_done()`'s contract; the queued request is live.
    unsafe { ds_routines::ds_read_done(ior) };
    c_int::from(true)
}

/// `ttyclose()` of device/chario.c: complete the delayed replies and hang up.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn close(tp: &mut Tty) {
    // SAFETY: the delayed queues are the tty's, initialized before any
    // request is queued, and the caller holds the tty lock.
    unsafe {
        drain(ptr::from_mut(&mut tp.t_delayed_read), tty_close_read_reply);
        drain(
            ptr::from_mut(&mut tp.t_delayed_write),
            tty_close_write_reply,
        );
        drain(ptr::from_mut(&mut tp.t_delayed_open), tty_close_open_reply);
    }

    if let Some(mctl) = tp.t_mctl {
        // SAFETY: the driver's modem-control routine takes its own tty; the
        // tty lock is held.
        unsafe { mctl(ptr::from_mut(tp), TM_BRK | TM_RTS, DMBIC) };
        if tp.t_state & (TS_HUPCLS | TS_WOPEN) != 0
            || tp.t_state & TS_ISOPEN == 0
        {
            // SAFETY: as above.
            unsafe { mctl(ptr::from_mut(tp), TM_HUP, DMSET) };
        }
    }

    tp.t_state &= TS_MIN | TS_CARR_ON;
}

/// `tty_portdeath()` of device/chario.c: complete the requests whose reply
/// port died.
pub(crate) fn port_death(tp: &mut Tty, port: *mut c_void) -> bool {
    let level = lock_irq(tp);

    // The queues may never have been initialized, as the C comment says; a
    // zeroed queue head has null links.
    let result = if tp.t_delayed_read.is_initialized() {
        // SAFETY: the three queues are initialized and the tty lock is held.
        unsafe {
            clean_queue(&tp.t_delayed_read, port, tty_close_read_reply)
                || clean_queue(
                    &tp.t_delayed_write,
                    port,
                    tty_close_write_reply,
                )
                || clean_queue(&tp.t_delayed_open, port, tty_close_open_reply)
        }
    } else {
        false
    };

    unlock_irq(tp, level);
    result
}

/// The `TTY_STATUS` read of `tty_get_status()`.
pub(crate) fn status(tp: &Tty) -> TtyStatus {
    let level = lock_irq(tp);
    let mut status = TtyStatus {
        tt_ispeed: c_int::from(tp.t_ispeed),
        tt_ospeed: c_int::from(tp.t_ospeed),
        tt_breakc: c_int::from(tp.t_breakc),
        tt_flags: tp.t_flags,
    };
    if tp.t_state & TS_HUPCLS != 0 {
        status.tt_flags |= TF_HUPCLS;
    }
    unlock_irq(tp, level);
    status
}

/// The `TTY_STATUS` write of `tty_set_status()`.
pub(crate) fn apply_status(
    tp: &mut Tty,
    status: &TtyStatus,
) -> Result<(), DeviceError> {
    let Some(ispeed) = speed_row(status.tt_ispeed) else {
        return Err(DeviceError::InvalidOperation);
    };
    let Some(ospeed) = speed_row(status.tt_ospeed) else {
        return Err(DeviceError::InvalidOperation);
    };

    let level = lock_irq(tp);
    tp.t_ispeed = ispeed;
    tp.t_ospeed = ospeed;
    // The C stored the int in a char, truncating; this keeps that.
    tp.t_breakc = status.tt_breakc as c_char;
    tp.t_flags = status.tt_flags & !TF_HUPCLS;
    if status.tt_flags & TF_HUPCLS != 0 {
        tp.t_state |= TS_HUPCLS;
    }
    unlock_irq(tp, level);
    Ok(())
}

/// The `TTY_FLUSH` case of `tty_set_status()`.
pub(crate) fn set_flush(tp: &mut Tty, flags: c_int) {
    let level = lock_irq(tp);
    flush(tp, flags);
    unlock_irq(tp, level);
}

/// The `TTY_STOP` case of `tty_set_status()`.
pub(crate) fn stop_output(tp: &mut Tty) {
    let level = lock_irq(tp);
    if tp.t_state & TS_TTSTOP == 0 {
        tp.t_state |= TS_TTSTOP;
        if let Some(stop) = tp.t_stop {
            // SAFETY: the driver's stop routine takes its own tty; the tty
            // lock is held.
            unsafe { stop(ptr::from_mut(tp), 0) };
        }
    }
    unlock_irq(tp, level);
}

/// The `TTY_START` case of `tty_set_status()`.
pub(crate) fn start_output(tp: &mut Tty) {
    let level = lock_irq(tp);
    if tp.t_state & TS_TTSTOP != 0 {
        tp.t_state &= !TS_TTSTOP;
        start(tp);
    }
    unlock_irq(tp, level);
}

/// `ttychars()` of device/chario.c.
pub(crate) fn chars(tp: &mut Tty) {
    // The C tested `t_flags`, not `t_state`; `TS_INIT` is 1, the same value
    // as `TF_TANDEM`, so the test is kept exactly.
    if tp.t_flags & TS_INIT == 0 {
        // SAFETY: the queues are the tty's, stay at their addresses, and are
        // not yet linked.
        unsafe {
            QueueEntry::init_head(pinned_head(&mut tp.t_delayed_open));
            QueueEntry::init_head(pinned_head(&mut tp.t_delayed_read));
            QueueEntry::init_head(pinned_head(&mut tp.t_delayed_write));
        }

        // SAFETY: `cb_alloc()`'s contract; `kalloc_init()` has run by the
        // time a tty is first opened, and the buffers are the tty's own.
        // The sizes are the C `unsigned int` constants, which fit any
        // pointer width.
        unsafe {
            cirbuf::cb_alloc(
                ptr::from_mut(&mut tp.t_inq),
                TTY_INQ_SIZE as usize,
            );
        }

        if tp.t_mctl.is_some() && tp.t_inq.hog() > 30 {
            tp.t_inq.lower_hog(30);
        }

        // SAFETY: as the input allocation above.
        unsafe {
            cirbuf::cb_alloc(
                ptr::from_mut(&mut tp.t_outq),
                TTY_OUTQ_SIZE as usize,
            );
        }

        tp.t_state |= TS_INIT;
    }

    tp.t_breakc = 0;
}

/// The tty's queue head `entry`, pinned because linked entries point at it.
fn pinned_head(entry: &mut QueueEntry) -> Pin<&mut QueueEntry> {
    // SAFETY: the tty owns the queue for the kernel's lifetime, so the head
    // never moves while it is linked.
    unsafe { Pin::new_unchecked(entry) }
}

/// `tty_flush()` of device/chario.c.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn flush(tp: &mut Tty, rw: c_int) {
    if rw & D_READ != 0 {
        tp.t_inq.clear();
        // SAFETY: the read queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_read)) };
    }
    if rw & D_WRITE != 0 {
        tp.t_state &= !TS_TTSTOP;
        if let Some(stop) = tp.t_stop {
            // SAFETY: the driver's stop routine takes its own tty; the tty
            // lock is held.
            unsafe { stop(ptr::from_mut(tp), rw) };
        }
        tp.t_outq.clear();
        // SAFETY: the write queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_write)) };
    }
}

/// `ttrstrt()` of device/chario.c: restart output after a delay timeout.
pub(crate) fn restart(tp: &mut Tty) {
    let level = lock_irq(tp);
    tp.t_state &= !TS_TIMEOUT;
    start(tp);
    unlock_irq(tp, level);
}

/// `ttstart()` and `tty_output()` of device/chario.c, whose bodies are the
/// same.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn start(tp: &mut Tty) {
    if tp.t_state & (TS_TIMEOUT | TS_TTSTOP | TS_BUSY) != 0 {
        return;
    }
    if let Some(driver_start) = tp.t_start {
        // SAFETY: the driver's start routine takes its own tty; the tty lock
        // is held.
        unsafe { driver_start(ptr::from_mut(tp)) };
    }
    if tp.t_outq.count() <= low_water(tp) {
        // SAFETY: the write queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_write)) };
    }
}

/// `ttypush()` of device/chario.c: the PDMA receive timeout callback.
unsafe extern "C" fn ttypush(param: *mut c_void) {
    // SAFETY: `timeout()` gets back the tty `input()` armed it with, which
    // stays live.
    let tp = unsafe { &mut *param.cast::<Tty>() };
    let level = lock_irq(tp);
    let state = tp.t_state;

    if state & TS_MIN_TO != 0 {
        if state & TS_MIN_TO_RCV != 0 {
            tp.t_state = state & !TS_MIN_TO_RCV;
            // SAFETY: `timeout()`'s contract; the tty lock is held and the
            // callback runs on the master CPU.
            tp.t_timeout = NonNull::new(unsafe {
                mach_clock::timeout(
                    Some(ttypush),
                    param,
                    pdma_timeout(tp.t_ispeed),
                )
            });
        } else {
            tp.t_state = state & !TS_MIN_TO;
            if tp.t_inq.count() != 0 {
                // SAFETY: the read queue is the tty's and stays at its
                // address.
                unsafe {
                    complete_queue(ptr::from_mut(&mut tp.t_delayed_read))
                };
            }
        }
    } else {
        tp.t_state = state & !TS_MIN_TO_RCV;
    }

    unlock_irq(tp, level);
}

/// Put one input character on the tty's input queue.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn input(tp: &mut Tty, c: c_uint) {
    if tp.t_inq.count() >= tp.t_inq.hog() {
        if let Some(mctl) = tp.t_mctl {
            // SAFETY: the driver's modem-control routine takes its own tty;
            // the tty lock is held.
            unsafe { mctl(ptr::from_mut(tp), TM_RTS, DMBIC) };
            tp.t_state |= TS_RTS_DOWN;
        }
        // SAFETY: the read queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_read)) };
        return;
    }

    let _ = tp.t_inq.put((c & 0xff) as u8);

    if tp.t_state & TS_MIN == 0
        || c_int::from(tp.t_inq.count()) > pdma_water(tp.t_ispeed)
    {
        if tp.t_state & TS_MIN_TO != 0 {
            tp.t_state &= !(TS_MIN_TO | TS_MIN_TO_RCV);
            // SAFETY: `reset_timeout()`'s contract; `t_timeout` is the live
            // timeout `input()` armed.
            unsafe {
                mach_clock::reset_timeout(
                    tp.t_timeout.map_or(ptr::null_mut(), |p| p.as_ptr()),
                )
            };
        }
        // SAFETY: the read queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_read)) };
    } else {
        let ptime = pdma_timeout(tp.t_ispeed);
        if ptime > 0 {
            if tp.t_state & TS_MIN_TO == 0 {
                tp.t_state |= TS_MIN_TO;
                // SAFETY: `timeout()`'s contract; the tty lock is held and
                // the callback runs on the master CPU.
                tp.t_timeout = NonNull::new(unsafe {
                    mach_clock::timeout(
                        Some(ttypush),
                        ptr::from_mut(tp).cast::<c_void>(),
                        ptime,
                    )
                });
            } else {
                tp.t_state |= TS_MIN_TO_RCV;
            }
        }
    }
}

/// Put many input characters on the tty's input queue.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn input_many(tp: &mut Tty, chars: &[u8]) {
    if tp.t_inq.count() < tp.t_inq.hog() {
        let _ = tp.t_inq.write(chars);
    }
    // SAFETY: the read queue is the tty's and stays at its address.
    unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_read)) };
}

/// Handle a carrier transition, the `ttymodem()` of device/chario.c.
///
/// # Safety
///
/// The caller must hold the tty lock, as the C contract requires.
pub(crate) fn modem(tp: &mut Tty, carrier_up: bool) -> bool {
    if tp.t_state & TS_WOPEN == 0 && tp.t_flags & TF_MDMBUF != 0 {
        if carrier_up {
            tp.t_state &= !TS_TTSTOP;
            start(tp);
        } else if tp.t_state & TS_TTSTOP == 0 {
            tp.t_state |= TS_TTSTOP;
            if let Some(stop) = tp.t_stop {
                // SAFETY: the driver's stop routine takes its own tty; the
                // tty lock is held.
                unsafe { stop(ptr::from_mut(tp), 0) };
            }
        }
    } else if carrier_up {
        tp.t_state |= TS_CARR_ON;
        // SAFETY: the open queue is the tty's and stays at its address.
        unsafe { complete_queue(ptr::from_mut(&mut tp.t_delayed_open)) };
    } else {
        tp.t_state &= !TS_CARR_ON;
        if tp.t_state & TS_ISOPEN != 0 && tp.t_flags & TF_NOHANG == 0 {
            flush(tp, D_READ | D_WRITE);
            return false;
        }
    }
    true
}

/// Handle a ClearToSend transition, the `tty_cts()` of device/chario.c.
///
/// # Safety
///
/// The caller must hold the tty lock and be on the master CPU, as the C
/// contract requires.
pub(crate) fn cts(tp: &mut Tty, cts_up: bool) {
    if tp.t_state & TS_ISOPEN != 0 {
        if cts_up {
            tp.t_state &= !(TS_TTSTOP | TS_BUSY);
            start(tp);
        } else {
            tp.t_state |= TS_TTSTOP | TS_BUSY;
            if let Some(stop) = tp.t_stop {
                // SAFETY: the driver's stop routine takes its own tty; the
                // tty lock is held.
                unsafe { stop(ptr::from_mut(tp), D_WRITE) };
            }
        }
    }
}

/// `chario_init()` of device/chario.c: the PDMA tables.
pub(crate) fn chario_init() {
    for speed in B0..B300 {
        // SAFETY: `speed` is below `B300 < NSPEEDS`; the one writer runs at
        // boot before any tty is open.
        unsafe {
            ptr::addr_of_mut!(PDMA_TIMEOUTS[speed]).write(0);
            ptr::addr_of_mut!(PDMA_WATER_MARK[speed]).write(0);
        }
    }

    // The live clock rate the probe set before `device_service_create()`.
    let hz = mach_clock::hz;

    for (speed, baud) in PDMA_TIMEOUT_ROWS {
        // SAFETY: every speed is below `NSPEEDS`, the length of the table.
        unsafe {
            ptr::addr_of_mut!(PDMA_TIMEOUTS[speed]).write(hz / baud + 2)
        };
    }

    for (speed, mark) in PDMA_SLOW_MARK_ROWS {
        // SAFETY: every speed is below `NSPEEDS`, the length of the table.
        unsafe { ptr::addr_of_mut!(PDMA_WATER_MARK[speed]).write(mark) };
    }

    let half_queue = half_input_queue();
    for speed in PDMA_FAST_SPEEDS {
        // SAFETY: every speed is below `NSPEEDS`, the length of the table.
        unsafe { ptr::addr_of_mut!(PDMA_WATER_MARK[speed]).write(half_queue) };
    }
}

/// Half `tty_inq_size`, the water mark device/chario.c gives the fast lines.
fn half_input_queue() -> c_int {
    // The C value is 4096, so half is far inside `c_int`; the C narrowed the
    // same unsigned half implicitly when it stored it in the `int` table.
    (TTY_INQ_SIZE / 2) as c_int
}
