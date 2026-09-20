// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The keyboard/VGA console driver, which `i386/i386at/kd.c` defined.
//!
//! `/dev/console` is a tty whose output is drawn by `esc.rs` through
//! the display table in `display.rs`, and whose input comes from
//! `keyboard.rs` through `kdintr()` (or `kdcnmaygetc()` when the
//! debugger polls).  `console.rs` holds the console entry points and
//! `kd_event.rs`'s line discipline feed.
//!
//! The tty device entry points (`kdopen()`, `kdclose()`, `kdread()`,
//! `kdwrite()`, `kdgetstat()`, `kdsetstat()`, `kdmmap()`,
//! `kdportdeath()`, `kdstart()`, `kdstop()`) still live in C with
//! `kd_tty` while the tty layer has no Rust layout; the C file reaches
//! this module through three shims (`kd_tty_rint()`, `kd_tty_init()`,
//! `kd_phystokv()`, `kd_rebootflag()`).

pub mod console;
pub mod display;
pub mod esc;
pub mod keyboard;
pub mod keymap;
pub mod tty;

use crate::glue;
use crate::utils::delay::delay;
use core::ffi::{c_int, c_short, c_uint};

/// `NUMKEYS` in <i386at/kd.h>.
pub(crate) const NUMKEYS: usize = 89;
/// `NUMOUTPUT` in <i386at/kd.h>.
pub(crate) const NUMOUTPUT: usize = 3;
/// `WIDTH_KMAP` in <i386at/kd.h>: `NUMSTATES * NUMOUTPUT`.
pub(crate) const WIDTH_KMAP: usize = 15;

/// `ONE_SPACE` in <i386at/kd.h>: bytes per displayed character.
pub(crate) const ONE_SPACE: c_short = 2;
/// `ONE_LINE` in <i386at/kd.h>: bytes per screen line.
pub(crate) const ONE_LINE: c_short = 160;
/// `ONE_PAGE` in <i386at/kd.h>: bytes per screen.
pub(crate) const ONE_PAGE: c_short = 4000;
/// `BOTTOM_LINE` in <i386at/kd.h>: first byte of the last line.
pub(crate) const BOTTOM_LINE: c_short = 3840;

/// `BEG_OF_LINE()` in <i386at/kd.h>.
pub(crate) fn beg_of_line(pos: c_short) -> c_short {
    pos - pos % ONE_LINE
}

/// `CURRENT_COLUMN()` in <i386at/kd.h>.
pub(crate) fn current_column(pos: c_short) -> c_short {
    (pos % ONE_LINE) / ONE_SPACE
}

/// `CHARIDX()` in <i386at/kd.h>: state index to key_map column.
pub(crate) fn charidx(state_idx: c_int) -> usize {
    state_idx as usize * NUMOUTPUT
}

/// `K_MAXESC` in <i386at/kd.c>: the escape sequence buffer.
pub(crate) const K_MAXESC: usize = 32;

// Keyboard controller ports, <i386at/kd.h>.
pub(crate) const K_TMR2: u16 = 0x42;
pub(crate) const K_TMRCTL: u16 = 0x43;
pub(crate) const K_RDWR: u16 = 0x60;
pub(crate) const K_PORTB: u16 = 0x61;
pub(crate) const K_STATUS: u16 = 0x64;
pub(crate) const K_CMD: u16 = 0x64;
pub(crate) const K_OBUF_FUL: u8 = 0x01;
pub(crate) const K_IBUF_FUL: u8 = 0x02;
pub(crate) const K_SPKRDATA: u8 = 0x02;
pub(crate) const K_ENABLETMR2: u8 = 0x01;
pub(crate) const K_SELTMR2: u8 = 0x80;
pub(crate) const K_RDLDTWORD: u8 = 0x30;
pub(crate) const K_TSQRWAVE: u8 = 0x06;
pub(crate) const K_TBINARY: u8 = 0x00;
pub(crate) const K_CMD_LEDS: u8 = 0xed;
pub(crate) const K_LED_NUMLK: u8 = 0x2;
pub(crate) const K_LED_CAPSLK: u8 = 0x4;
pub(crate) const KC_CMD_READ: u8 = 0x20;
pub(crate) const KC_CMD_WRITE: u8 = 0x60;
pub(crate) const K_CB_DISBLE: u8 = 0x10;
pub(crate) const K_CB_ENBLIRQ: u8 = 0x01;
pub(crate) const KBD_IRQ: c_uint = 1;

// Keyboard bytes, <i386at/kd.h>.
pub(crate) const K_ESC: u8 = 0x1b;
pub(crate) const K_LF: u8 = 0x0a;
pub(crate) const K_CR: u8 = 0x0d;
pub(crate) const K_BS: u8 = 0x08;
pub(crate) const K_HT: u8 = 0x09;
pub(crate) const K_BEL: u8 = 0x07;
pub(crate) const K_SPACE: u8 = 0x20;
pub(crate) const K_QUES: u8 = 0x3f;
pub(crate) const K_UP: u8 = 0x80;
pub(crate) const K_EXTEND: u8 = 0xe0;
pub(crate) const K_ACKSC: u8 = 0xfa;
pub(crate) const K_RESEND: u8 = 0xfe;
pub(crate) const K_SCAN: u8 = 0xfe;
pub(crate) const K_DONE: u8 = 0xff;

// Modifier scan codes.
pub(crate) const K_CTLSC: u8 = 0x1d;
pub(crate) const K_LSHSC: u8 = 0x2a;
pub(crate) const K_RSHSC: u8 = 0x36;
pub(crate) const K_ALTSC: u8 = 0x38;
pub(crate) const K_CLCKSC: u8 = 0x3a;
pub(crate) const K_NLCKSC: u8 = 0x45;
pub(crate) const K_HOMESC: u8 = 0x47;
pub(crate) const K_DELSC: u8 = 0x53;

// Modifier state bits and state indices.
pub(crate) const KS_NORMAL: c_int = 0x00;
pub(crate) const KS_NLKED: c_int = 0x02;
pub(crate) const KS_CLKED: c_int = 0x04;
pub(crate) const KS_ALTED: c_int = 0x08;
pub(crate) const KS_SHIFTED: c_int = 0x10;
pub(crate) const KS_CTLED: c_int = 0x20;
pub(crate) const NORM_STATE: c_int = 0;
pub(crate) const SHIFT_STATE: c_int = 1;
pub(crate) const CTRL_STATE: c_int = 2;
pub(crate) const ALT_STATE: c_int = 3;
pub(crate) const SHIFT_ALT: c_int = 4;

/// `kb_mode` values from <device/input.h>.
pub(crate) const KB_EVENT: c_int = 1;
pub(crate) const KB_ASCII: c_int = 2;

// Display attributes, <i386at/kd.h>.
pub(crate) const KA_NORMAL: u8 = 0x07;
pub(crate) const KA_REVERSE: u8 = 0x70;
pub(crate) const KAX_REVERSE: u8 = 0x01;
pub(crate) const KAX_UNDERLINE: u8 = 0x02;
pub(crate) const KAX_BLINK: u8 = 0x04;
pub(crate) const KAX_BOLD: u8 = 0x08;
pub(crate) const KAX_DIM: u8 = 0x10;
pub(crate) const KAX_INVISIBLE: u8 = 0x20;
pub(crate) const KAX_COL_UNDERLINE: u8 = 0x0f;
pub(crate) const KAX_COL_DIM: u8 = 0x08;

// VGA/EGA registers and memory, <i386at/kd.h>.
pub(crate) const EGA_START: usize = 0x0b8000;
pub(crate) const EGA_IDX_REG: u16 = 0x3d4;
pub(crate) const EGA_IO_REG: u16 = 0x3d5;
pub(crate) const C_START: u8 = 0x0a;
pub(crate) const C_STOP: u8 = 0x0b;
pub(crate) const C_LOW: u8 = 0x0f;
pub(crate) const C_HIGH: u8 = 0x0e;
pub(crate) const C_BITMAP_START: usize = 0xa0000;

/// `SLAMBPW` in <i386at/kd.c>.
pub(crate) const SLAMBPW: c_int = 2;

/// `CN_INTERNAL` of <device/cons.h>.
pub(crate) const CN_INTERNAL: c_short = 2;

/// `MACH_ATOI_DEFAULT` of <util/atoi.h>, the "no number" value
/// `mach_atoi()` stores.
pub(crate) const MACH_ATOI_DEFAULT: c_int = -1;

/// `color_table[]` in <i386at/kd.c>, the proper ANSI color order.
pub(crate) const COLOR_TABLE: [u8; 16] =
    [0, 4, 2, 6, 1, 5, 3, 7, 8, 12, 10, 14, 9, 13, 11, 15];

/// The `why_ack` enumeration of <i386at/kd.c>.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ack {
    NotWaiting,
    SetLeds,
    Data,
}

/// The `struct consdev` prefix of <device/cons.h> the console entry
/// points touch.
///
/// The function pointers are owned by `cons_conf.c` only so the struct
/// is laid out correctly; the driver writes `cn_dev` and `cn_pri`.
#[repr(C)]
#[allow(dead_code)]
pub struct ConsDev {
    cn_name: *mut core::ffi::c_char,
    cn_probe: Option<unsafe extern "C" fn(*mut ConsDev) -> c_int>,
    cn_init: Option<unsafe extern "C" fn(*mut ConsDev) -> c_int>,
    cn_getc: Option<unsafe extern "C" fn(u16, c_int) -> c_int>,
    cn_putc: Option<unsafe extern "C" fn(u16, c_int) -> c_int>,
    cn_dev: u16,
    cn_pri: c_short,
}

/// `struct kbentry` of <i386at/kd.h>, the key remapping ioctl payload.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbEntry {
    pub kb_state: u8,
    pub kb_index: u8,
    pub kb_value: [u8; NUMOUTPUT],
}

/// The driver's mutable state: the C file's file-scope globals.
pub(crate) struct State {
    pub(crate) kd_initialized: bool,
    pub(crate) kd_extended: bool,
    pub(crate) kd_pollc: c_int,
    pub(crate) old_kb_mode: c_int,
    pub(crate) sit_for_0: c_int,

    pub(crate) kd_attr: u8,
    pub(crate) kd_color: u8,
    pub(crate) kd_attrflags: u8,
    pub(crate) kd_curpos: c_short,
    pub(crate) kd_lines: c_short,
    pub(crate) kd_cols: c_short,
    pub(crate) vid_start: *mut u8,
    pub(crate) kd_index_reg: c_short,
    pub(crate) kd_io_reg: c_short,

    pub(crate) esc_seq: [u8; K_MAXESC],
    pub(crate) esc_spt: usize,

    pub(crate) kd_ack: Ack,
    pub(crate) last_sent: u8,
    pub(crate) kd_nextled: u8,
    pub(crate) kd_kbd_mouse: c_int,
    pub(crate) kd_kbd_magic_scale: c_int,
    pub(crate) kd_kbd_magic_button: c_int,
    pub(crate) magic_state: c_int,

    pub(crate) kd_bellstate: bool,

    pub(crate) font_start: *const u8,
    pub(crate) fb_height: c_short,
    pub(crate) char_width: c_short,
    pub(crate) char_height: c_short,
    pub(crate) chars_in_font: c_short,
    pub(crate) cursor_height: c_short,
    pub(crate) char_black: u8,
    pub(crate) char_white: u8,
    pub(crate) xstart: c_short,
    pub(crate) ystart: c_short,
    pub(crate) char_byte_width: c_short,
    pub(crate) fb_byte_width: c_short,
    pub(crate) font_byte_width: c_short,
}

impl State {
    const fn new() -> Self {
        Self {
            kd_initialized: false,
            kd_extended: false,
            kd_pollc: 0,
            old_kb_mode: 0,
            sit_for_0: 1,
            kd_attr: KA_NORMAL,
            kd_color: KA_NORMAL,
            kd_attrflags: 0,
            kd_curpos: 0,
            kd_lines: 25,
            kd_cols: 80,
            vid_start: EGA_START as *mut u8,
            kd_index_reg: EGA_IDX_REG as c_short,
            kd_io_reg: EGA_IO_REG as c_short,
            esc_seq: [0; K_MAXESC],
            esc_spt: 0,
            kd_ack: Ack::NotWaiting,
            last_sent: 0,
            kd_nextled: 0,
            kd_kbd_mouse: 0,
            kd_kbd_magic_scale: 6,
            kd_kbd_magic_button: 0,
            magic_state: KS_NORMAL,
            kd_bellstate: false,
            font_start: core::ptr::null(),
            fb_height: 0,
            char_width: 0,
            char_height: 0,
            chars_in_font: 0,
            cursor_height: 0,
            char_black: 0,
            char_white: 0xff,
            xstart: 0,
            ystart: 0,
            char_byte_width: 0,
            fb_byte_width: 0,
            font_byte_width: 0,
        }
    }
}

static mut STATE: State = State::new();

/// The one state object.  Callers must hold `SPLKD`, which serializes
/// every use, and must not hold the reference across a call that could
/// re-enter the driver.
pub(crate) fn state() -> &'static mut State {
    // SAFETY: the driver runs at SPLKD; nothing else accesses `STATE`.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

/// `kb_mode` of <i386at/kd.h>, the ascii/event switch.  `kd_event.rs`
/// sets it from `kbdsetstat()`; `kd.c` used to own it.
static mut KB_MODE: c_int = KB_ASCII;

/// The current keyboard mode.
pub(crate) fn kb_mode() -> c_int {
    // SAFETY: a plain integer, published at SPLKD.
    unsafe { KB_MODE }
}

/// Set the keyboard mode: `kbd_set_mode()` was the C shim for this.
pub(crate) fn set_kb_mode(mode: c_int) {
    // SAFETY: as above.
    unsafe { KB_MODE = mode };
}

/// `kd_state` of <i386at/kd.h>: needed by the C `kdgetstat()` until the
/// tty slice moves it.
#[unsafe(no_mangle)]
pub static mut kd_state: c_int = KS_NORMAL;

/// `kd_bitmap_start` of <i386at/kd.c>, needed by the C `kdmmap()` until
/// the tty slice moves it.
#[unsafe(no_mangle)]
pub static mut kd_bitmap_start: usize = C_BITMAP_START;

/// Initialize the driver.  `kdinit()` in C; interrupts are assumed
/// disabled, and the call is idempotent.
///
/// # Safety
///
/// The caller must hold the interrupt level the C used (`spltty`), and
/// the keyboard controller must not be in use.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdinit() {
    if state().kd_initialized {
        return;
    }
    {
        let s = state();
        s.esc_spt = 0;
        s.kd_attr = KA_NORMAL;
        s.kd_attrflags = 0;
        s.kd_color = KA_NORMAL;
    }
    // Board-specific initialization, then the controller.
    unsafe { display::kd_xga_init() };

    // Get rid of any garbage in the output buffer.
    if unsafe { glue::pio_inb(K_STATUS) } & K_OBUF_FUL != 0 {
        let _ = unsafe { glue::pio_inb(K_RDWR) };
    }

    unsafe {
        keyboard::kd_sendcmd(KC_CMD_READ);
        let mut k_comm = keyboard::kd_getdata();
        k_comm &= !K_CB_DISBLE;
        k_comm |= K_CB_ENBLIRQ;
        keyboard::kd_sendcmd(KC_CMD_WRITE);
        keyboard::kd_senddata(k_comm);
        glue::irq_unmask(KBD_IRQ);
    }
    state().kd_initialized = true;

    // Clear the LEDs after enabling the controller: this keeps
    // NUM-LOCK from being set on the NEC Versa.
    unsafe {
        kd_state = KS_NORMAL;
        keyboard::cnsetleds(KS_NORMAL as u8);
    }

    // Allocate the input buffer.
    tty::ttychars_init();
}

/// `cnpollc()` in C: switch the console between polled and interrupted.
///
/// # Safety
///
/// Called by the debugger's console layer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnpollc(on: c_int) {
    if unsafe { crate::arch::i386::kd_mouse::mouse_in_use } != 0 {
        if on != 0 {
            // Switch into X.
            let s = state();
            s.old_kb_mode = kb_mode();
            set_kb_mode(KB_ASCII);
            unsafe { crate::arch::i386::kd_event::X_kdb_enter() };
            s.kd_pollc += 1;
        } else {
            let s = state();
            s.kd_pollc -= 1;
            // Switch out of X.
            unsafe { crate::arch::i386::kd_event::X_kdb_exit() };
            set_kb_mode(s.old_kb_mode);
        }
    } else if on != 0 {
        state().kd_pollc += 1;
    } else {
        state().kd_pollc -= 1;
    }
}

/// `kdreboot()` in C.
///
/// # Safety
///
/// Called on a magic key sequence or a reboot request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdreboot() {
    unsafe { (display::kd_dreset)() };
    unsafe { keyboard::kd_sendcmd(0xfe) };
    delay(1000000);
    unsafe { glue::cpu_shutdown() };
}

/// `kd_belloff()` in C.
///
/// # Safety
///
/// Called from the timeout table; `_param` is unused.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_belloff(_param: *mut core::ffi::c_void) {
    let status =
        unsafe { glue::pio_inb(K_PORTB) } & !(K_SPKRDATA | K_ENABLETMR2);
    unsafe { glue::pio_outb(K_PORTB, status) };
    state().kd_bellstate = false;
}

/// `kd_bellon()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_bellon() {
    // Program timer 2.
    unsafe {
        glue::pio_outb(
            K_TMRCTL,
            K_SELTMR2 | K_RDLDTWORD | K_TSQRWAVE | K_TBINARY,
        );
        glue::pio_outb(K_TMR2, (1500 & 0xff) as u8);
        glue::pio_outb(K_TMR2, (1500 >> 8) as u8);
    }
    // Start the speaker.
    let status = unsafe { glue::pio_inb(K_PORTB) } | K_ENABLETMR2 | K_SPKRDATA;
    unsafe { glue::pio_outb(K_PORTB, status) };
}
