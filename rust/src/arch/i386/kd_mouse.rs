// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The mouse driver, which `i386/i386at/kd_mouse.c` used to define.
//!
//! `/dev/mouse` speaks the Mouse Systems 5-byte, Microsoft and Logitech
//! 3-byte and IBM PS/2 3-byte protocols, on a serial port or the
//! keyboard controller.  The bytes are decoded into `kd_event`s and
//! queued for `mouseread()`.
//!
//! Everything runs at `SPLKD` (`spltty`): `mouseintr()` is entered from
//! the interrupt path, the device entry points bracket their queue
//! access with `spltty()`/`splx()`, and the C file's globals are the
//! `STATE` below under that serialization.
//!
//! `kdintr()` reads `MOUSE_IN_USE` and calls `mouse_handle_byte()`;
//! only the four conf.c device entries stay `extern "C"`.

use super::io_req::{
    D_ALREADY_OPEN, D_INVALID_OPERATION, D_INVALID_SIZE, D_IO_QUEUED,
    D_NOWAIT, D_SUCCESS, D_WOULD_BLOCK, DEV_GET_SIZE, DEV_GET_SIZE_COUNT,
    DEV_GET_SIZE_DEVICE_SIZE, DEV_GET_SIZE_RECORD_SIZE, DevT, IoReq,
    KERN_SUCCESS, drain,
};
use crate::glue;
use crate::kern::queue::QueueEntry;
use crate::utils::kd_queue::{KdEvent, KdEventQueue, KevType, MouseMotion};
use core::ffi::{c_int, c_long, c_uint};
use core::mem::{MaybeUninit, size_of};
use core::pin::Pin;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, Ordering};

/// `interrupt_handler_fn` of <i386/ipl.h>.
type InterruptHandler = unsafe extern "C" fn(c_int);

/// `MOUSEBUFSIZE` in <i386at/kd_mouse.h>.
const MOUSEBUFSIZE: usize = 5;

/// Button directions: `MOUSE_DOWN` is the C's 0, and the direction a
/// `mouse_button()` caller passes.
const MOUSE_UP: u8 = 1;
const MOUSE_DOWN: u8 = 0;
const MOUSE_ALL_UP: u8 = 0x7;

/// `IBM_MOUSE_IRQ` in <i386at/kd_mouse.c>.
const IBM_MOUSE_IRQ: c_int = 12;

/// Mouse protocols, from the high bits of the minor number.
const MOUSE_SYSTEM_MOUSE: c_int = 0;
const MICROSOFT_MOUSE: c_int = 1;
const IBM_MOUSE: c_int = 2;
const NO_MOUSE: c_int = 3;
const LOGITECH_TRACKMAN: c_int = 4;
const MICROSOFT_MOUSE7: c_int = 5;

/// Event types of <device/input.h>.
const MOUSE_LEFT: KevType = 1;
const MOUSE_MIDDLE: KevType = 2;
const MOUSE_RIGHT: KevType = 3;

// 8250 register offsets and bits, <i386at/i8250.h>.
const RDAT: u16 = 0;
const RIE: u16 = 1;
const RID: u16 = 2;
const RLC: u16 = 3;
const RMC: u16 = 4;
const RLS: u16 = 5;
const RDLSB: u16 = 0;
const RDMSB: u16 = 1;
const IERD: u8 = 0x01;
const IELS: u8 = 0x04;
const IDRD: u8 = 0x04;
const IDLS: u8 = 0x06;
const LC7: u8 = 0x02;
const LC8: u8 = 0x03;
const LCDLAB: u8 = 0x80;
const LSDR: u8 = 0x01;
const MCDTR: u8 = 0x01;
const MCRTS: u8 = 0x02;
const MCOUT2: u8 = 0x08;
const BCNT1200: c_int = 0x60;

// Keyboard controller ports, <i386at/kd.h>.
const K_RDWR: u16 = 0x60;
const K_STATUS: u16 = 0x64;
const K_CMD: u16 = 0x64;
const K_IBUF_FUL: u8 = 0x02;

/// Whether `/dev/mouse` is open.  `i386/i386at/kd.c` reads it directly
/// (`cnpollc`, `kdintr`), so the symbol and its `boolean_t` size stay.
pub(crate) static mut MOUSE_IN_USE: c_int = 0;

/// The driver's mutable state: the C file's file-scope globals.
///
/// `read_queue` is a `MaybeUninit` because a queue head has to link to
/// itself, which no static initializer can express; it is set up on the
/// first use, before any interrupt can reach it.
struct State {
    queue: KdEventQueue,
    read_queue: MaybeUninit<QueueEntry>,
    read_queue_ready: bool,
    lastbuttons: u8,
    mouse_baud: c_int,
    mouse_type: c_int,
    mousebufsize: c_int,
    mousebufindex: c_int,
    mouse_char_cmd: bool,
    mouse_char_wanted: bool,
    mouse_char_index: c_int,
    lastgitech: c_int,
    fourthgitech: c_int,
    middlegitech: c_int,
    mousebuf: [u8; MOUSEBUFSIZE],
    oldvect: Option<InterruptHandler>,
    oldunit: c_int,
    track_man: [c_int; 10],
    mouse_packets: c_int,
    show_mouse_byte: c_int,
}

impl State {
    const fn new() -> Self {
        Self {
            queue: KdEventQueue::new(),
            read_queue: MaybeUninit::uninit(),
            read_queue_ready: false,
            lastbuttons: 0,
            mouse_baud: BCNT1200,
            mouse_type: 0,
            mousebufsize: 0,
            mousebufindex: 0,
            mouse_char_cmd: false,
            mouse_char_wanted: false,
            mouse_char_index: 0,
            lastgitech: 0x40,
            fourthgitech: 0,
            middlegitech: 0,
            mousebuf: [0; MOUSEBUFSIZE],
            oldvect: None,
            oldunit: 0,
            track_man: [0; 10],
            mouse_packets: 0,
            show_mouse_byte: 0,
        }
    }
}

static mut STATE: State = State::new();

/// The one state object.  Callers must hold `SPLKD`, which serializes
/// every use, and must not hold the reference across a call that could
/// re-enter the driver.
fn state() -> &'static mut State {
    // SAFETY: the driver runs at SPLKD; nothing else accesses `STATE`,
    // and no reference outlives the function that took it.
    unsafe { &mut *ptr::addr_of_mut!(STATE) }
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

/// `printf_once("mouse: queue full\n")` in C: prints the first time a
/// full queue drops an event, then never again.
fn printf_once() {
    static PRINTED: AtomicBool = AtomicBool::new(false);
    if !PRINTED.swap(true, Ordering::Relaxed) {
        // SAFETY: a literal format string with no arguments.
        unsafe { glue::printf(c"mouse: queue full\n".as_ptr()) };
    }
}

/// Enqueue `ev` and complete any reads waiting for data.  Called at
/// `SPLKD`.
fn enqueue(s: &mut State, ev: &KdEvent) {
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

/// Enqueue a mouse-motion event.  `mouse_moved()` in C.
fn motion_event(s: &mut State, moved: MouseMotion) {
    enqueue(s, &KdEvent::motion(moved));
}

/// Enqueue a button event.  `mouse_button()` in C.
fn button_event(s: &mut State, which: KevType, direction: u8) {
    enqueue(s, &KdEvent::button(which, direction == MOUSE_UP));
}

/// `init_mouse_hw()` in C: program the serial port.
fn init_mouse_hw(s: &State, unit: c_int, mode: u8) {
    let base_addr = unsafe { glue::com_base_addr(unit) } as u16;
    // SAFETY: `base_addr` is the unit's 8250 base; the register offsets
    // come from <i386at/i8250.h>.
    unsafe {
        glue::pio_outb(base_addr + RIE, 0);
        glue::pio_outb(base_addr + RLC, LCDLAB);
        glue::pio_outb(base_addr + RDLSB, (s.mouse_baud & 0xff) as u8);
        glue::pio_outb(base_addr + RDMSB, ((s.mouse_baud >> 8) & 0xff) as u8);
        glue::pio_outb(base_addr + RLC, mode);
        glue::pio_outb(base_addr + RMC, MCDTR | MCRTS | MCOUT2);
        glue::pio_outb(base_addr + RIE, IERD | IELS);
    }
}

/// `serial_mouse_open()` in C: take over the unit's interrupt vector.
fn serial_open(s: &mut State, dev: DevT) {
    let unit = (dev & 7) as c_int;
    let mouse_pic = unsafe { glue::com_irq(unit) };
    let sp = unsafe { glue::splhi() };
    s.oldvect = unsafe { glue::irq_get_handler(mouse_pic) };
    // SAFETY: the handler has the C `interrupt_handler_fn` signature.
    unsafe { glue::irq_set_handler(mouse_pic, Some(mouseintr)) };
    s.oldunit = unsafe { glue::irq_get_unit(mouse_pic) };
    unsafe { glue::irq_set_unit(mouse_pic, unit) };
    unsafe { glue::splx(sp) };
}

/// `kd_mouse_open()` in C: route the IRQ to the keyboard driver.
fn kd_open(s: &mut State, mouse_pic: c_int) {
    let sp = unsafe { glue::splhi() };
    s.oldvect = unsafe { glue::irq_get_handler(mouse_pic) };
    unsafe {
        glue::irq_set_handler(
            mouse_pic,
            Some(crate::arch::i386::kd::keyboard::kdintr),
        )
    };
    unsafe { glue::irq_unmask(mouse_pic as c_uint) };
    unsafe { glue::splx(sp) };
}

/// `serial_mouse_close()` in C.
fn serial_close(s: &mut State, dev: DevT) {
    let sp = unsafe { glue::splhi() };
    let unit = (dev & 7) as c_int;
    let mouse_pic = unsafe { glue::com_irq(unit) };
    let base_addr = unsafe { glue::com_base_addr(unit) } as u16;
    // SAFETY: as in `init_mouse_hw()`, and the old vector/unit were
    // saved by the matching open.
    unsafe {
        glue::pio_outb(base_addr + RIE, 0);
        glue::pio_outb(base_addr + RMC, 0);
        glue::irq_set_handler(mouse_pic, s.oldvect);
        glue::irq_set_unit(mouse_pic, s.oldunit);
        glue::splx(sp);
    }
}

/// `kd_mouse_close()` in C.
fn kd_close(s: &mut State, mouse_pic: c_int) {
    let sp = unsafe { glue::splhi() };
    // SAFETY: the vector was saved by the matching open.
    unsafe {
        glue::irq_mask(mouse_pic as c_uint);
        glue::irq_set_handler(mouse_pic, s.oldvect);
        glue::splx(sp);
    }
}

/// `kd_mouse_write()` in C: send a byte to the PS/2 mouse.
fn write_char(ch: u8) {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {
        core::hint::spin_loop();
    }
    unsafe { glue::pio_outb(K_CMD, 0xd4) };
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {
        core::hint::spin_loop();
    }
    unsafe { glue::pio_outb(K_RDWR, ch) };
}

/// `kd_mouse_read()` in C: wait for a byte the interrupt path delivers.
fn read_char(s: &mut State) -> c_int {
    if s.mouse_char_index >= s.mousebufsize {
        return -1;
    }
    while s.mousebufindex <= s.mouse_char_index {
        s.mouse_char_wanted = true;
        // SAFETY: the wait channel is the driver's own buffer, and the
        // handler wakes this exact address.
        unsafe { glue::assert_wait(ptr::addr_of_mut!(s.mousebuf).cast(), 0) };
        // SAFETY: no thread state to hand over; the caller resumes
        // after the wakeup.
        unsafe { glue::thread_block(None) };
    }
    let ch = s.mousebuf[s.mouse_char_index as usize];
    s.mouse_char_index += 1;
    ch as c_int
}

/// `kd_mouse_read_reset()` in C.
fn read_reset(s: &mut State) {
    s.mousebufindex = 0;
    s.mouse_char_index = 0;
}

/// `ibm_ps2_mouse_open()` in C.
fn ps2_open(s: &mut State, _dev: DevT) {
    let sp = unsafe { glue::spltty() };
    s.lastbuttons = 0;
    s.mouse_char_cmd = true;
    crate::arch::i386::kd::keyboard::sendcmd(0xa8);
    crate::arch::i386::kd::keyboard::cmdreg_write(0x47);
    read_reset(s);
    write_char(0xff);
    if read_char(s) != 0xfa {
        unsafe { glue::splx(sp) };
        return;
    }
    let _ = read_char(s);
    let _ = read_char(s);
    read_reset(s);
    write_char(0xea);
    if read_char(s) != 0xfa {
        unsafe { glue::splx(sp) };
        return;
    }
    read_reset(s);
    write_char(0xf4);
    if read_char(s) != 0xfa {
        unsafe { glue::splx(sp) };
        return;
    }
    read_reset(s);
    s.mouse_char_cmd = false;
    unsafe { glue::splx(sp) };
}

/// `ibm_ps2_mouse_close()` in C.
fn ps2_close(s: &mut State, _dev: DevT) {
    let sp = unsafe { glue::spltty() };
    s.mouse_char_cmd = true;
    read_reset(s);
    write_char(0xff);
    if read_char(s) == 0xfa {
        let _ = read_char(s);
        let _ = read_char(s);
    }
    crate::arch::i386::kd::keyboard::sendcmd(0xa7);
    crate::arch::i386::kd::keyboard::cmdreg_write(0x65);
    unsafe { glue::splx(sp) };
}

/// `mouse_packet_mouse_system_mouse()` in C.
fn packet_mouse_system(s: &mut State, buf: &[u8; MOUSEBUFSIZE]) {
    let buttons = buf[0] & 0x7;
    let buttonchanges = buttons ^ s.lastbuttons;
    let moved = MouseMotion {
        mm_delta_x: (buf[1] as i8 as i16).wrapping_add(buf[3] as i8 as i16),
        mm_delta_y: (buf[2] as i8 as i16).wrapping_add(buf[4] as i8 as i16),
    };
    if moved.mm_delta_x != 0 || moved.mm_delta_y != 0 {
        motion_event(s, moved);
    }
    if buttonchanges != 0 {
        s.lastbuttons = buttons;
        if buttonchanges & 1 != 0 {
            button_event(s, MOUSE_RIGHT, buttons & 1);
        }
        if buttonchanges & 2 != 0 {
            button_event(s, MOUSE_MIDDLE, (buttons & 2) >> 1);
        }
        if buttonchanges & 4 != 0 {
            button_event(s, MOUSE_LEFT, (buttons & 4) >> 2);
        }
    }
}

/// `mouse_packet_microsoft_mouse()` in C.
fn packet_microsoft(s: &mut State, buf: &[u8; MOUSEBUFSIZE]) {
    let mut buttons = (buf[0] & 0x30) >> 4;
    buttons |= s.middlegitech as u8;
    buttons = !buttons & 0x07;
    let buttonchanges = buttons ^ s.lastbuttons;
    let mut dx = ((buf[0] & 0x03) as i16) << 6 | (buf[1] & 0x3f) as i16;
    let mut dy = ((buf[0] & 0x0c) as i16) << 4 | (buf[2] & 0x3f) as i16;
    if dx & 0x80 != 0 {
        dx -= 0x100;
    }
    if dy & 0x80 != 0 {
        dy -= 0x100;
    }
    let moved = MouseMotion {
        mm_delta_x: dx,
        mm_delta_y: -dy,
    };
    if moved.mm_delta_x != 0 || moved.mm_delta_y != 0 {
        motion_event(s, moved);
    }
    if buttonchanges != 0 {
        s.lastbuttons = buttons;
        if buttonchanges & 1 != 0 {
            let dir = if buttons & 1 != 0 {
                MOUSE_UP
            } else {
                MOUSE_DOWN
            };
            button_event(s, MOUSE_RIGHT, dir);
        }
        if buttonchanges & 2 != 0 {
            let dir = if buttons & 2 != 0 {
                MOUSE_UP
            } else {
                MOUSE_DOWN
            };
            button_event(s, MOUSE_LEFT, dir);
        }
        if buttonchanges & 4 != 0 {
            let dir = if buttons & 4 != 0 {
                MOUSE_UP
            } else {
                MOUSE_DOWN
            };
            button_event(s, MOUSE_MIDDLE, dir);
        }
    }
}

/// `mouse_packet_ibm_ps2_mouse()` in C.
fn packet_ibm_ps2(s: &mut State, buf: &[u8; MOUSEBUFSIZE]) {
    let buttons = buf[0] & 0x7;
    let buttonchanges = buttons ^ s.lastbuttons;
    let moved = MouseMotion {
        mm_delta_x: if buf[0] & 0x10 != 0 {
            (0xffffff00u32 | buf[1] as u32) as i16
        } else {
            buf[1] as i16
        },
        mm_delta_y: if buf[0] & 0x20 != 0 {
            (0xffffff00u32 | buf[2] as u32) as i16
        } else {
            buf[2] as i16
        },
    };
    if s.mouse_packets != 0 {
        // SAFETY: a literal format with three integers.
        unsafe {
            glue::printf(
                c"(%x:%x:%x)".as_ptr(),
                buf[0] as c_int,
                buf[1] as c_int,
                buf[2] as c_int,
            );
        }
        return;
    }
    if moved.mm_delta_x != 0 || moved.mm_delta_y != 0 {
        motion_event(s, moved);
    }
    if buttonchanges != 0 {
        s.lastbuttons = buttons;
        if buttonchanges & 1 != 0 {
            button_event(s, MOUSE_LEFT, u8::from(buttons & 1 == 0));
        }
        if buttonchanges & 2 != 0 {
            button_event(s, MOUSE_RIGHT, u8::from(buttons & 2 == 0));
        }
        if buttonchanges & 4 != 0 {
            button_event(s, MOUSE_MIDDLE, u8::from(buttons & 4 == 0));
        }
    }
}

/// `mouse_handle_byte()` in C: accumulate bytes until a packet is
/// complete, then decode it.  Called at `SPLKD`.
fn handle_byte(s: &mut State, ch: u8) {
    if s.show_mouse_byte != 0 {
        // SAFETY: a literal format with two integers.
        unsafe { glue::printf(c"%x(%c) ".as_ptr(), ch as c_int, ch as c_int) };
    }
    if s.mouse_char_cmd {
        if s.mousebufindex < s.mousebufsize {
            s.mousebuf[s.mousebufindex as usize] = ch;
            s.mousebufindex += 1;
        }
        if s.mouse_char_wanted {
            s.mouse_char_wanted = false;
            // SAFETY: the channel is the buffer `read_char()` waits on.
            unsafe { glue::wakeup(ptr::addr_of!(s.mousebuf) as usize) };
        }
        return;
    }
    if s.mousebufindex == 0 {
        match s.mouse_type {
            MICROSOFT_MOUSE7 => {
                if ch & 0x40 != 0x40 {
                    return;
                }
            }
            MICROSOFT_MOUSE => {
                if ch & 0xc0 != 0xc0 {
                    return;
                }
            }
            MOUSE_SYSTEM_MOUSE => {
                if ch & 0xf8 != 0x80 {
                    return;
                }
            }
            LOGITECH_TRACKMAN => {
                if s.fourthgitech == 1 {
                    s.fourthgitech = 0;
                    s.middlegitech = if ch & 0xf0 != 0 { 0x4 } else { 0x0 };
                    let buf = s.mousebuf;
                    packet_microsoft(s, &buf);
                    return;
                } else if ch & 0xc0 != 0x40 {
                    return;
                }
            }
            IBM_MOUSE => {}
            _ => {}
        }
    }
    s.mousebuf[s.mousebufindex as usize] = ch;
    s.mousebufindex += 1;
    if s.mousebufindex < s.mousebufsize {
        return;
    }
    s.mousebufindex = 0;
    let buf = s.mousebuf;
    match s.mouse_type {
        MICROSOFT_MOUSE7 | MICROSOFT_MOUSE => packet_microsoft(s, &buf),
        MOUSE_SYSTEM_MOUSE => packet_mouse_system(s, &buf),
        LOGITECH_TRACKMAN => {
            if buf[1] != 0 || buf[2] != 0 || buf[0] != s.lastgitech as u8 {
                packet_microsoft(s, &buf);
                s.lastgitech = (buf[0] & 0xf0) as c_int;
            } else {
                s.fourthgitech = 1;
            }
        }
        IBM_MOUSE => packet_ibm_ps2(s, &buf),
        _ => {}
    }
}

/// Open the mouse.  `mouseopen()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid, open request; everything
/// else runs at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mouseopen(
    dev: DevT,
    _flags: c_int,
    _ior: *mut IoReq,
) -> c_int {
    if unsafe { MOUSE_IN_USE } != 0 {
        return D_ALREADY_OPEN;
    }
    unsafe { MOUSE_IN_USE = 1 };
    let s = state();
    s.queue.clear();
    s.lastbuttons = MOUSE_ALL_UP;
    s.mouse_type = (((dev & 0xff) & 0xf8) >> 3) as c_int;
    match s.mouse_type {
        MICROSOFT_MOUSE7 => {
            s.mousebufsize = 3;
            serial_open(s, dev);
            init_mouse_hw(s, (dev & 7) as c_int, LC7);
        }
        MICROSOFT_MOUSE => {
            s.mousebufsize = 3;
            serial_open(s, dev);
            init_mouse_hw(s, (dev & 7) as c_int, LC8);
        }
        MOUSE_SYSTEM_MOUSE => {
            s.mousebufsize = 5;
            serial_open(s, dev);
            init_mouse_hw(s, (dev & 7) as c_int, LC8);
        }
        LOGITECH_TRACKMAN => {
            s.mousebufsize = 3;
            serial_open(s, dev);
            init_mouse_hw(s, (dev & 7) as c_int, LC7);
            s.track_man[0] = unsafe { glue::comgetc((dev & 7) as c_int) };
            s.track_man[1] = unsafe { glue::comgetc((dev & 7) as c_int) };
            if s.track_man[0] != 0x4d && s.track_man[1] != 0x33 {
                // SAFETY: a literal format with no arguments.
                unsafe { glue::printf(c"LOGITECH_TRACKMAN: NOT M3".as_ptr()) };
            }
        }
        IBM_MOUSE => {
            s.mousebufsize = 3;
            kd_open(s, IBM_MOUSE_IRQ);
            ps2_open(s, dev);
        }
        NO_MOUSE => {}
        _ => {}
    }
    s.mousebufindex = 0;
    0
}

/// Close the mouse.  `mouseclose()` in C.
///
/// # Safety
///
/// The device layer calls this for an open mouse.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mouseclose(dev: DevT, _flags: c_int) {
    let s = state();
    match s.mouse_type {
        MICROSOFT_MOUSE | MICROSOFT_MOUSE7 | MOUSE_SYSTEM_MOUSE
        | LOGITECH_TRACKMAN => serial_close(s, dev),
        IBM_MOUSE => {
            ps2_close(s, dev);
            kd_close(s, IBM_MOUSE_IRQ);
            // The C waited here for the mouse to settle.
            let mut i: c_int = 20000;
            while i != 0 {
                i -= 1;
                core::hint::black_box(i);
            }
            crate::arch::i386::kd::keyboard::mouse_drain();
        }
        _ => {}
    }
    s.queue.clear();
    unsafe { MOUSE_IN_USE = 0 };
}

/// Read queued events.  `mouseread()` in C.
///
/// # Safety
///
/// The device layer calls this with a valid, read-only request whose
/// buffer `device_read_alloc()` may allocate; everything else runs at
/// `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mouseread(_dev: DevT, ior: *mut IoReq) -> c_int {
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
        unsafe { (*ior).set_done(mouse_read_done) };
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
/// `mouse_read_done()` in C, as a callback value.
unsafe extern "C" fn mouse_read_done(ior: *mut IoReq) -> c_int {
    let s = state();
    // SAFETY: as in `mouseread()`.
    let sp = unsafe { glue::spltty() };
    if s.queue.is_empty() {
        unsafe { (*ior).set_done(mouse_read_done) };
        // SAFETY: as in `mouseread()`.
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

/// Device size query.  `mousegetstat()` in C.
///
/// # Safety
///
/// The device layer calls this with `data` able to hold the two
/// `DEV_GET_SIZE_*` values and a valid `count`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mousegetstat(
    _dev: DevT,
    flavor: c_uint,
    data: *mut c_int,
    count: *mut u32,
) -> c_int {
    if flavor == DEV_GET_SIZE {
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

/// The unit's interrupt handler.  `mouseintr()` in C, as a callback
/// value.
unsafe extern "C" fn mouseintr(unit: c_int) {
    let base_addr = unsafe { glue::com_base_addr(unit) } as u16;
    // SAFETY: the port block is the unit's.
    let id = unsafe { glue::pio_inb(base_addr + RID) };
    let ls = unsafe { glue::pio_inb(base_addr + RLS) };
    if id == IDLS {
        if ls & LSDR != 0 {
            let _ = unsafe { glue::pio_inb(base_addr + RDAT) };
        }
        return;
    }
    if id & IDRD != 0 {
        let ch = unsafe { glue::pio_inb(base_addr + RDAT) };
        handle_byte(state(), ch);
    }
}

/// Accumulate one mouse byte.  `mouse_handle_byte()` in C; called at
/// `SPLKD` from the kd interrupt path.
pub(crate) fn mouse_handle_byte(ch: u8) {
    handle_byte(state(), ch);
}

/// Enqueue a mouse-motion event.  `mouse_moved()` in C.
pub(crate) fn mouse_moved(where_: MouseMotion) {
    motion_event(state(), where_);
}

/// Enqueue a button event.  `mouse_button()` in C.
pub(crate) fn mouse_button(which: KevType, direction: u8) {
    button_event(state(), which, direction);
}
