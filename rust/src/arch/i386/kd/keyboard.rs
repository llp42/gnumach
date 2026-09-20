// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The keyboard half of the kd driver: the scan-code interrupt, the
//! modifier state machine, the magic key sequences, the keyboard
//! controller commands and the key map.
//!
//! Everything here runs at `SPLKD`; `kdintr()` is entered from the
//! keyboard IRQ (vector 1) and `kdcnmaygetc()` polls the same engine
//! when the debugger has interrupts off.

use super::keymap::KEY_MAP;
use super::*;
use crate::arch::i386::kd_event::kd_enqsc;
use crate::arch::i386::kd_mouse;
use crate::glue;
use core::ffi::{c_int, c_uint};

/// The `which_button[]` table of `kd_kbd_magic()`: index to event type
/// (`MOUSE_LEFT`, `MOUSE_MIDDLE`, `MOUSE_RIGHT` of <device/input.h>).
const WHICH_BUTTON: [u16; 4] = [0, 1, 2, 3];

// The scancodes the magic-key "mouse" uses.
const K_F1SC: c_int = 0x3b;
const K_F2SC: c_int = 0x3c;
const K_F3SC: c_int = 0x3d;
const K_KP_HOME: c_int = 0x47;
const K_UPSC: c_int = 0x48;
const K_KP_PGUP: c_int = 0x49;
const K_LEFTSC: c_int = 0x4b;
const K_RIGHTSC: c_int = 0x4d;
const K_KP_END: c_int = 0x4f;
const K_DOWNSC: c_int = 0x50;
const K_KP_PGDN: c_int = 0x51;

/// Read the exported `kd_state`.
fn state_bits() -> c_int {
    // SAFETY: a plain integer written at SPLKD.
    unsafe { KD_STATE }
}

/// Write the exported `kd_state`.
fn set_state_bits(value: c_int) {
    // SAFETY: as above.
    unsafe { KD_STATE = value };
}

/// The current keyboard mode.
fn mode() -> c_int {
    kb_mode()
}

/// `do_modifier()`: the new state for a modifier key.
pub(crate) fn modifier(state_in: c_int, c: u8, up: bool) -> c_int {
    let mut st = state_in;
    match c {
        K_ALTSC => {
            if up {
                st &= !KS_ALTED;
            } else {
                st |= KS_ALTED;
            }
            state().kd_extended = false;
        }
        K_CLCKSC | K_CTLSC => {
            if up {
                st &= !KS_CTLED;
            } else {
                st |= KS_CTLED;
            }
            state().kd_extended = false;
        }
        K_NLCKSC => {
            if !up {
                st ^= KS_NLKED;
            }
        }
        K_LSHSC | K_RSHSC => {
            if up {
                st &= !KS_SHIFTED;
            } else {
                st |= KS_SHIFTED;
            }
            state().kd_extended = false;
        }
        _ => {}
    }
    st
}

/// `kdstate2idx()`: the key_map column for a modifier state.
pub(crate) fn state2idx(state_in: c_uint, extended: bool) -> usize {
    let st = state_in as c_int;
    let mut state_idx = NORM_STATE;
    if !extended && st != KS_NORMAL {
        if st & (KS_SHIFTED | KS_ALTED) == (KS_SHIFTED | KS_ALTED) {
            state_idx = SHIFT_ALT;
        } else if st & KS_CTLED != 0 {
            state_idx = CTRL_STATE;
        } else if st & KS_SHIFTED != 0 {
            state_idx = SHIFT_STATE;
        } else if st & KS_ALTED != 0 {
            state_idx = ALT_STATE;
        }
    }
    charidx(state_idx)
}

/// Wait for the input buffer and send a byte to the keyboard.
pub(crate) fn senddata(ch: u8) {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {}
    unsafe { glue::pio_outb(K_RDWR, ch) };
    state().last_sent = ch;
}

/// Wait for the input buffer and send a command to the keyboard.
pub(crate) fn sendcmd(ch: u8) {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {}
    unsafe { glue::pio_outb(K_CMD, ch) };
}

/// Wait for a data byte from the keyboard.
pub(crate) fn getdata() -> u8 {
    while unsafe { glue::pio_inb(K_STATUS) } & K_OBUF_FUL == 0 {}
    unsafe { glue::pio_inb(K_RDWR) }
}

/// Complete a pending keyboard command.
pub(crate) fn handle_ack() {
    match state().kd_ack {
        Ack::SetLeds => {
            set_leds2();
            state().kd_ack = Ack::Data;
        }
        Ack::Data => state().kd_ack = Ack::NotWaiting,
        Ack::NotWaiting => {
            // SAFETY: a literal format with no arguments.
            unsafe {
                glue::printf(c"unexpected ACK from keyboard\n".as_ptr())
            };
        }
    }
}

/// Resend a missed keyboard command or data byte.
pub(crate) fn resend() {
    if state().kd_ack == Ack::NotWaiting {
        // SAFETY: a literal format with no arguments.
        unsafe { glue::printf(c"unexpected RESEND from keyboard\n".as_ptr()) };
    } else {
        senddata(state().last_sent);
    }
}

/// Start setting the LEDs.
pub(crate) fn set_leds1(val: u8) {
    if state().kd_ack != Ack::NotWaiting {
        return;
    }
    state().kd_ack = Ack::SetLeds;
    state().kd_nextled = val;
    senddata(K_CMD_LEDS);
}

/// Send the LED byte after the command ack.
pub(crate) fn set_leds2() {
    senddata(state().kd_nextled);
}

/// `cnsetleds()`: set the LEDs without interrupts.
pub(crate) fn cn_set_leds(val: u8) {
    senddata(K_CMD_LEDS);
    let _ = getdata(); // assume ACK
    senddata(val);
    let _ = getdata(); // assume ACK
}

/// `kdgetkbent()`: read a key map entry.
fn map_get(row: usize, col: usize) -> [u8; NUMOUTPUT] {
    // SAFETY: the caller checks the indexes.
    unsafe {
        [
            KEY_MAP[row][col],
            KEY_MAP[row][col + 1],
            KEY_MAP[row][col + 2],
        ]
    }
}

/// `kdsetkbent()`: write a key map entry.
fn map_set(row: usize, col: usize, value: [u8; NUMOUTPUT]) {
    // SAFETY: the caller checks the indexes.
    unsafe {
        KEY_MAP[row][col] = value[0];
        KEY_MAP[row][col + 1] = value[1];
        KEY_MAP[row][col + 2] = value[2];
    }
}

/// `mouse_button()` with the event type the magic keys use.
fn mouse_button(which: u16, direction: u8) {
    kd_mouse::mouse_button(which, direction);
}

/// `mouse_moved()` with a delta scaled by the magic scale.
fn motion(dx: c_int, dy: c_int) {
    let mm = crate::utils::kd_queue::MouseMotion {
        mm_delta_x: dx as i16,
        mm_delta_y: dy as i16,
    };
    kd_mouse::mouse_moved(mm);
}

/// `kd_kbd_magic()`: the keyboard-as-mouse sequences.
pub(crate) fn kbd_magic(scancode: c_int) -> c_int {
    if state().kd_kbd_mouse == 2 {
        // SAFETY: a literal format with one integer.
        unsafe { glue::printf(c"sc = %x\n".as_ptr(), scancode) };
    }

    match scancode {
        // f1 f2 f3, with the C switch's fallthrough: 0x3b yields 1,
        // 0x3c 2, 0x3d 3.
        K_F1SC | K_F2SC | K_F3SC => {
            let new_button = scancode - K_F1SC + 1;
            let s = state();
            let old = s.kd_kbd_magic_button;
            if old != 0 && new_button != old {
                // down without up
                mouse_button(WHICH_BUTTON[old as usize], 1);
            }
            if old == new_button {
                mouse_button(WHICH_BUTTON[new_button as usize], 1);
                s.kd_kbd_magic_button = 0;
            } else {
                mouse_button(WHICH_BUTTON[new_button as usize], 0);
                s.kd_kbd_magic_button = new_button;
            }
        }
        // right left up down
        K_RIGHTSC => motion(state().kd_kbd_magic_scale, 0),
        K_LEFTSC => motion(-state().kd_kbd_magic_scale, 0),
        K_UPSC => motion(0, state().kd_kbd_magic_scale),
        K_DOWNSC => motion(0, -state().kd_kbd_magic_scale),
        // home pageup end pagedown
        K_KP_HOME => motion(
            -2 * state().kd_kbd_magic_scale,
            2 * state().kd_kbd_magic_scale,
        ),
        K_KP_PGUP => motion(
            2 * state().kd_kbd_magic_scale,
            2 * state().kd_kbd_magic_scale,
        ),
        K_KP_END => motion(
            -2 * state().kd_kbd_magic_scale,
            -2 * state().kd_kbd_magic_scale,
        ),
        K_KP_PGDN => motion(
            2 * state().kd_kbd_magic_scale,
            -2 * state().kd_kbd_magic_scale,
        ),
        _ => return 0,
    }
    1
}

/// `kdcheckmagic()`: the magic key sequences.
fn checkmagic(scancode: u8) -> bool {
    if scancode == K_SLCKSC {
        // Scroll lock: toggle the keyboard-as-mouse hack.
        let s = state();
        s.kd_kbd_mouse = c_int::from(s.kd_kbd_mouse == 0);
        s.kd_kbd_magic_button = 0;
        return true;
    }
    let up = scancode & K_UP != 0;
    let scancode = if up { scancode & !K_UP } else { scancode };
    let st = state();
    st.magic_state = modifier(st.magic_state, scancode, up);

    if st.magic_state & (KS_CTLED | KS_ALTED) == (KS_CTLED | KS_ALTED)
        && scancode == K_DELSC
        && unsafe { glue::rebootflag } != 0
    {
        // SAFETY: the caller asked for a reboot with ctl-alt-del.
        unsafe { super::kdreboot() };
    }
    false
}

/// The keyboard IRQ handler.  `kdintr()` in C.
fn intr() {
    if state().kd_pollc != 0 {
        return; // kdb polling the keyboard
    }
    if !state().kd_initialized {
        return;
    }

    // Allow for keyboards that raise the interrupt before the character
    // reaches the buffer, but do not wait forever.
    let mut safety: c_int = 1000;
    while unsafe { glue::pio_inb(K_STATUS) } & K_OBUF_FUL == 0 {
        safety -= 1;
        if safety == 0 {
            break;
        }
    }

    // We may have seen a mouse event.
    if unsafe { glue::pio_inb(K_STATUS) } & K_AUX_OBUF_FUL == K_AUX_OBUF_FUL {
        let sc = unsafe { glue::pio_inb(K_RDWR) };
        if unsafe { kd_mouse::MOUSE_IN_USE } != 0 {
            kd_mouse::mouse_handle_byte(sc);
        } else {
            // SAFETY: a literal format with one integer.
            unsafe { glue::printf(c"M%xI".as_ptr(), sc as c_int) };
        }
        return;
    }

    let mut scancode = unsafe { glue::pio_inb(K_RDWR) };
    if scancode == K_EXTEND && mode() != KB_EVENT {
        state().kd_extended = true;
        return;
    } else if scancode == K_RESEND {
        resend();
        return;
    } else if scancode == K_ACKSC {
        handle_ack();
        return;
    } else if (state().kd_kbd_mouse != 0 && kbd_magic(scancode as c_int) != 0)
        || checkmagic(scancode)
    {
        return;
    } else if mode() == KB_EVENT {
        kd_enqsc(scancode);
        return;
    }

    let up = scancode & K_UP != 0;
    if up {
        scancode &= !K_UP;
    }
    if (scancode as usize) < NUMKEYS {
        // Look up in the map, then process.
        let mut char_idx =
            state2idx(state_bits() as c_uint, state().kd_extended);
        let mut c = unsafe { KEY_MAP[scancode as usize][char_idx] };
        if c == K_SCAN {
            char_idx += 1;
            c = unsafe { KEY_MAP[scancode as usize][char_idx] };
            let st = modifier(state_bits(), c, up);
            set_state_bits(st);
        } else if !up {
            // A regular key-down.
            let mut max = char_idx + NUMOUTPUT;
            char_idx += 1;
            if !state().kd_extended {
                if state_bits() & KS_CLKED != 0 {
                    if c.is_ascii_uppercase() {
                        c = c.wrapping_add(b'a' - b'A');
                        max = char_idx;
                    } else if c.is_ascii_lowercase() {
                        c = c.wrapping_sub(b'a' - b'A');
                        max = char_idx;
                    }
                }
                // NumLock only affects the physical keypad.
                if state_bits() & KS_NLKED != 0
                    && (K_HOMESC..=K_DELSC).contains(&scancode)
                {
                    char_idx = charidx(SHIFT_STATE);
                    c = unsafe { KEY_MAP[scancode as usize][char_idx] };
                    max = char_idx + NUMOUTPUT;
                    char_idx += 1;
                }
            }
            // Put the character (or sequence) on the input queue.
            while c != K_DONE && char_idx <= max {
                // SAFETY: the tty feeds the line discipline at SPLKD.
                super::tty::line_rint(c);
                c = unsafe { KEY_MAP[scancode as usize][char_idx] };
                char_idx += 1;
            }
            state().kd_extended = false;
        }
    }
}

/// The keyboard IRQ handler.  `kdintr()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdintr(_vec: c_int) {
    intr();
}

/// Wait for the input buffer and write the controller command register,
/// which `kd_mouse.rs` uses for its PS/2 sequences.
pub(crate) fn cmdreg_write(val: c_int) {
    sendcmd(KC_CMD_WRITE);
    senddata(val as u8);
}

/// Drain pending keyboard bytes, printing them; `kd_mouse.rs` closes a
/// PS/2 mouse with this.
pub(crate) fn mouse_drain() {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {}
    let mut i = unsafe { glue::pio_inb(K_STATUS) };
    while i & K_OBUF_FUL != 0 {
        let data = unsafe { glue::pio_inb(K_RDWR) };
        // SAFETY: a literal format with two integers.
        unsafe {
            glue::printf(
                c"kbd: S = %x D = %x\n".as_ptr(),
                i as c_int,
                data as c_int,
            )
        };
        i = unsafe { glue::pio_inb(K_STATUS) };
    }
}

/// Read a key map entry into `kb`: the `kdgetkbent()` core.
pub(crate) fn entry_get(kb: &mut KbEntry) {
    let o_pri = unsafe { glue::spltty() };
    kb.kb_value = map_get(kb.kb_index as usize, charidx(kb.kb_state as c_int));
    unsafe { glue::splx(o_pri) };
}

/// Write a key map entry from `kb`: the `kdsetkbent()` core.
pub(crate) fn entry_set(kb: &KbEntry) {
    let o_pri = unsafe { glue::spltty() };
    map_set(
        kb.kb_index as usize,
        charidx(kb.kb_state as c_int),
        kb.kb_value,
    );
    unsafe { glue::splx(o_pri) };
}
