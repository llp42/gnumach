// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/kd_event.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The keyboard event driver, which `i386/i386at/kd_event.c` used to
//! define.
//!
//! `/dev/kbd` is fed by `kd.c`, which calls `kd_enqsc()` for every scan
//! code when the keyboard is in Event mode; `kbdread()` drains the
//! events to the reader.  The file also carries the `X_kdb` port-I/O
//! escape used by `cnpollc()`: the caller installs a list of in/out
//! commands with `kbdsetstat()`, and `x_kdb_enter()`/`x_kdb_exit()`
//! replay them.
//!
//! The queue and the device entry points run at `SPLKD` (`spltty`),
//! like the C file's globals.  `i386/i386at/kd.c` calls `x_kdb_enter()`
//! and `x_kdb_exit()`, and `conf.c` keeps the four device entries.

use super::io_req::{
    D_INVALID_OPERATION, D_INVALID_SIZE, D_IO_QUEUED, D_NOWAIT, D_SUCCESS,
    D_WOULD_BLOCK, DEV_GET_SIZE, DEV_GET_SIZE_COUNT, DEV_GET_SIZE_DEVICE_SIZE,
    DEV_GET_SIZE_RECORD_SIZE, DevT, IoReq, KERN_SUCCESS, drain,
};
use crate::glue;
use crate::kern::queue::QueueEntry;
use crate::utils::kd_queue::{KdEvent, KdEventQueue, Scancode};
use core::cell::UnsafeCell;
use core::ffi::{c_int, c_long, c_uint};
use core::mem::{MaybeUninit, size_of};
use core::pin::Pin;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, Ordering};

/// `sizeof x_kdb_enter_str / sizeof x_kdb_enter_str[0]` in C: the most
/// port commands `x_kdb_enter_init()` accepts.
const KDB_STR_MAX: usize = 512;

// The keyboard ioctls of <device/input.h>, whose `_IOW`/`_IOR` values
// are computed there; `sizeof(int)` is four on both targets.
const KDSKBDMODE: c_uint = 0x8004_4b01;
const KDGKBDTYPE: c_uint = 0x4004_4b02;
const KDSETLEDS: c_uint = 0x8004_4b05;
const KB_ASCII: c_int = 2;
const KB_VANILLAKB: c_int = 0;

// The `X_kdb` command bits of <i386at/kd.h>.
const K_X_IN: c_uint = 0x0100_0000;
const K_X_OUT: c_uint = 0x0200_0000;
const K_X_BYTE: c_uint = 0x0001_0000;
const K_X_WORD: c_uint = 0x0002_0000;
const K_X_LONG: c_uint = 0x0004_0000;
const K_X_TYPE: c_uint = 0x0307_0000;
const K_X_PORT: c_uint = 0x0000_ffff;

// Match patterns cannot be built with `|` (that is alternation), so the
// six kinds are spelled out.
const K_X_IN_BYTE: c_uint = K_X_IN | K_X_BYTE;
const K_X_IN_WORD: c_uint = K_X_IN | K_X_WORD;
const K_X_IN_LONG: c_uint = K_X_IN | K_X_LONG;
const K_X_OUT_BYTE: c_uint = K_X_OUT | K_X_BYTE;
const K_X_OUT_WORD: c_uint = K_X_OUT | K_X_WORD;
const K_X_OUT_LONG: c_uint = K_X_OUT | K_X_LONG;

// `K_X_KDB_ENTER`/`EXIT` carry `sizeof(struct X_kdb)` in the ioctl
// length field, and that is a pointer plus an `u_int`: 8 bytes on i686,
// 16 on x86_64.
#[cfg(target_pointer_width = "32")]
const K_X_KDB_ENTER: c_uint = 0x8008_4b10;
#[cfg(target_pointer_width = "32")]
const K_X_KDB_EXIT: c_uint = 0x8008_4b11;
#[cfg(target_pointer_width = "64")]
const K_X_KDB_ENTER: c_uint = 0x8010_4b10;
#[cfg(target_pointer_width = "64")]
const K_X_KDB_EXIT: c_uint = 0x8010_4b11;

/// The driver's mutable state: the C file's file-scope globals.
struct State {
    queue: KdEventQueue,
    read_queue: MaybeUninit<QueueEntry>,
    read_queue_ready: bool,
    initialized: bool,
    x_kdb_enter_str: [c_uint; KDB_STR_MAX],
    x_kdb_exit_str: [c_uint; KDB_STR_MAX],
    x_kdb_enter_len: usize,
    x_kdb_exit_len: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            queue: KdEventQueue::new(),
            read_queue: MaybeUninit::uninit(),
            read_queue_ready: false,
            initialized: false,
            x_kdb_enter_str: [0; KDB_STR_MAX],
            x_kdb_exit_str: [0; KDB_STR_MAX],
            x_kdb_enter_len: 0,
            x_kdb_exit_len: 0,
        }
    }
}

static STATE: crate::arch::i386::kd::SyncCell<State> =
    crate::arch::i386::kd::SyncCell(UnsafeCell::new(State::new()));

/// The one state object.  Callers must hold `SPLKD`, which serializes
/// every use, and must not hold the reference across a call that could
/// re-enter the driver.
fn state() -> &'static mut State {
    // SAFETY: the driver runs at SPLKD; nothing else accesses `STATE`.
    unsafe { &mut *STATE.0.get() }
}

/// The read queue head, self-linked on first use.  Callers hold `SPLKD`.
fn read_queue(s: &mut State) -> Pin<&mut QueueEntry> {
    let p = ptr::addr_of_mut!(s.read_queue).cast::<QueueEntry>();
    if !s.read_queue_ready {
        // SAFETY: `p` points at this state's `QueueEntry` storage, and
        // this is the first use; nothing else can reach it at SPLKD.
        unsafe {
            QueueEntry::pin_in_place(NonNull::new_unchecked(p)).init_head();
        }
        s.read_queue_ready = true;
    }
    // SAFETY: as above; the storage is initialized and at a fixed
    // address.
    unsafe { QueueEntry::pin_in_place(NonNull::new_unchecked(p)) }
}

/// `printf_once("kbd: queue full\n")` in C: prints the first time a
/// full queue drops an event, then never again.
fn printf_once() {
    static PRINTED: AtomicBool = AtomicBool::new(false);
    if !PRINTED.swap(true, Ordering::Relaxed) {
        // SAFETY: a literal format string with no arguments.
        unsafe { glue::printf(c"kbd: queue full\n".as_ptr()) };
    }
}

/// Enqueue `ev` and complete any reads waiting for data.  Called at
/// `SPLKD`.
fn enqueue_event(s: &mut State, ev: &KdEvent) {
    if s.queue.is_full() {
        printf_once();
    } else {
        s.queue.push_back(*ev);
    }
    loop {
        // SAFETY: the queue is self-consistent and this runs at SPLKD.
        let entry = unsafe { read_queue(s).pop_front() };
        match entry {
            // SAFETY: each link is an `io_req` (its chain is the first
            // field), still owned by the device layer and valid for
            // `iodone()`.
            Some(entry) => unsafe { glue::iodone(entry.as_ptr().cast()) },
            None => break,
        }
    }
}

/// `kbdinit()` in C: reset the queue once, at `SPLKD`.
fn kbdinit() {
    let sp = unsafe { glue::spltty() };
    let s = state();
    if !s.initialized {
        s.queue.clear();
        s.initialized = true;
    }
    unsafe { glue::splx(sp) };
}

/// `kdb_in_out()` in C: run one `X_kdb` command, whose second word is
/// `p1`.
fn kdb_in_out(p0: c_uint, p1: c_uint) {
    let port = (p0 & K_X_PORT) as u16;
    match p0 & K_X_TYPE {
        K_X_IN_BYTE => {
            let _ = unsafe { glue::pio_inb(port) };
        }
        K_X_IN_WORD => {
            let _ = unsafe { glue::pio_inw(port) };
        }
        K_X_IN_LONG => {
            let _ = unsafe { glue::pio_inl(port) };
        }
        K_X_OUT_BYTE => unsafe { glue::pio_outb(port, p1 as u8) },
        K_X_OUT_WORD => unsafe { glue::pio_outw(port, p1 as u16) },
        K_X_OUT_LONG => unsafe { glue::pio_outl(port, p1) },
        _ => {}
    }
}

/// Open the keyboard.  `kbdopen()` in C.
///
/// # Safety
///
/// The device layer calls this for the keyboard device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kbdopen(
    _dev: DevT,
    _flags: c_int,
    _ior: *mut IoReq,
) -> c_int {
    let sp = unsafe { glue::spltty() };
    // SAFETY: kd.c's driver init, as in C, at spltty.
    crate::arch::i386::kd::kdinit();
    unsafe { glue::splx(sp) };
    kbdinit();
    0
}

/// Close the keyboard: back to Ascii mode, empty queue.  `kbdclose()`
/// in C.
///
/// # Safety
///
/// The device layer calls this for an open keyboard.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kbdclose(_dev: DevT, _flags: c_int) {
    let sp = unsafe { glue::spltty() };
    // The mode is kd's, now that the keyboard driver is Rust.
    crate::arch::i386::kd::set_kb_mode(KB_ASCII);
    state().queue.clear();
    unsafe { glue::splx(sp) };
}

/// Device status query.  `kbdgetstat()` in C.
///
/// # Safety
///
/// The device layer calls this with `data` able to hold the value the
/// flavor asks for and a valid `count`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kbdgetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut u32,
) -> c_int {
    if flavor == KDGKBDTYPE {
        // SAFETY: the caller promises room for one integer.
        unsafe {
            *data = KB_VANILLAKB;
            *count = 1;
        }
        D_SUCCESS
    } else if flavor == DEV_GET_SIZE {
        // SAFETY: the caller promises room for the two values.
        unsafe {
            *data.add(DEV_GET_SIZE_DEVICE_SIZE) = 0;
            *data.add(DEV_GET_SIZE_RECORD_SIZE) =
                size_of::<KdEvent>() as c_int;
            *count = DEV_GET_SIZE_COUNT;
        }
        D_SUCCESS
    } else {
        D_INVALID_OPERATION
    }
}

/// Device status set.  `kbdsetstat()` in C.
///
/// # Safety
///
/// The device layer calls this with `data` holding `count` values for
/// the flavor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kbdsetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: u32,
) -> c_int {
    if flavor == KDSKBDMODE {
        // SAFETY: one integer behind `data`, and kd owns the mode.
        crate::arch::i386::kd::set_kb_mode(unsafe { *data });
        D_SUCCESS
    } else if flavor == KDSETLEDS {
        if count != 1 {
            return D_INVALID_OPERATION;
        }
        // SAFETY: `count == 1` promises one readable value; kd
        // truncates to the `u_char` the C passed.
        let val = unsafe { *data };
        crate::arch::i386::kd::keyboard::set_leds1(val as u8);
        D_SUCCESS
    } else if flavor == K_X_KDB_ENTER {
        // SAFETY: `data` holds `count` port commands.
        unsafe { x_kdb_enter_init(data.cast(), count) }
    } else if flavor == K_X_KDB_EXIT {
        // SAFETY: as above.
        unsafe { x_kdb_exit_init(data.cast(), count) }
    } else {
        D_INVALID_OPERATION
    }
}

/// Read queued events.  `kbdread()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid, read-only request whose
/// buffer `device_read_alloc()` may allocate; everything else runs at
/// `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kbdread(_dev: DevT, ior: *mut IoReq) -> c_int {
    let wanted = unsafe { (*ior).count() };
    if wanted % size_of::<KdEvent>() as c_long != 0 {
        return D_INVALID_SIZE;
    }
    // SAFETY: the request is the caller's, as the C assumed.
    let err = unsafe { glue::device_read_alloc(ior.cast(), wanted as usize) };
    if err != KERN_SUCCESS {
        return err;
    }
    let s = state();
    // SAFETY: queueing a request and the event queue share SPLKD.
    let sp = unsafe { glue::spltty() };
    if s.queue.is_empty() {
        if unsafe { (*ior).mode() } & D_NOWAIT != 0 {
            unsafe { glue::splx(sp) };
            return D_WOULD_BLOCK;
        }
        unsafe { (*ior).set_done(kbd_read_done) };
        // SAFETY: `io_req`'s chain is its first field, and it stays at
        // its address until `iodone()`.
        let entry = unsafe { (*ior).queue_entry() };
        // SAFETY: the read queue is this state's, at SPLKD.
        unsafe { read_queue(s).push_back(entry) };
        unsafe { glue::splx(sp) };
        return D_IO_QUEUED;
    }
    let count = drain(&mut s.queue, unsafe { &mut *ior });
    unsafe { glue::splx(sp) };
    unsafe { (*ior).set_residual((*ior).count() - count) };
    D_SUCCESS
}

/// Finish a read that was queued waiting for events.
/// `kbd_read_done()` in C, as a callback value.
unsafe extern "C" fn kbd_read_done(ior: *mut IoReq) -> c_int {
    let s = state();
    // SAFETY: as in `kbdread()`.
    let sp = unsafe { glue::spltty() };
    if s.queue.is_empty() {
        unsafe { (*ior).set_done(kbd_read_done) };
        // SAFETY: as in `kbdread()`.
        let entry = unsafe { (*ior).queue_entry() };
        unsafe { read_queue(s).push_back(entry) };
        unsafe { glue::splx(sp) };
        return 0;
    }
    let count = drain(&mut s.queue, unsafe { &mut *ior });
    unsafe { glue::splx(sp) };
    unsafe { (*ior).set_residual((*ior).count() - count) };
    // SAFETY: the request is complete; its data buffer is populated.
    unsafe { glue::ds_read_done(ior.cast()) };
    1
}

/// Enqueue a scancode.  `kd_enqsc()` in C; called at `SPLKD` from the
/// kd interrupt path.
pub(crate) fn kd_enqsc(sc: Scancode) {
    enqueue_event(state(), &KdEvent::scancode(sc));
}

/// Replay the `x_kdb_enter` port commands.  `x_kdb_enter()` in C.
pub(crate) fn x_kdb_enter() {
    let s = state();
    let len = s.x_kdb_enter_len;
    let mut i = 0;
    while i < len {
        kdb_in_out(s.x_kdb_enter_str[i], s.x_kdb_enter_str[i + 1]);
        i += 2;
    }
}

/// Replay the `x_kdb_exit` port commands.  `x_kdb_exit()` in C.
pub(crate) fn x_kdb_exit() {
    let s = state();
    let len = s.x_kdb_exit_len;
    let mut i = 0;
    while i < len {
        kdb_in_out(s.x_kdb_exit_str[i], s.x_kdb_exit_str[i + 1]);
        i += 2;
    }
}

/// Install the `x_kdb_enter` port commands.  `x_kdb_enter_init()` in C.
unsafe fn x_kdb_enter_init(data: *mut c_uint, count: c_uint) -> c_int {
    if count as usize > KDB_STR_MAX {
        return D_INVALID_OPERATION;
    }
    let s = state();
    // SAFETY: `count` is in bounds and the caller promises that many
    // readable integers behind `data`.
    unsafe {
        ptr::copy_nonoverlapping(
            data,
            s.x_kdb_enter_str.as_mut_ptr(),
            count as usize,
        );
    }
    s.x_kdb_enter_len = count as usize;
    D_SUCCESS
}

/// Install the `x_kdb_exit` port commands.  `x_kdb_exit_init()` in C.
unsafe fn x_kdb_exit_init(data: *mut c_uint, count: c_uint) -> c_int {
    if count as usize > KDB_STR_MAX {
        return D_INVALID_OPERATION;
    }
    let s = state();
    // SAFETY: `count` is in bounds and the caller promises that many
    // readable integers behind `data`.
    unsafe {
        ptr::copy_nonoverlapping(
            data,
            s.x_kdb_exit_str.as_mut_ptr(),
            count as usize,
        );
    }
    s.x_kdb_exit_len = count as usize;
    D_SUCCESS
}
