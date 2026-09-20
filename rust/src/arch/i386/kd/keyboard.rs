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

/// Read the exported `kd_state`.
fn state_bits() -> c_int {
    // SAFETY: a plain integer written at SPLKD.
    unsafe { kd_state }
}

/// Write the exported `kd_state`.
fn set_state_bits(value: c_int) {
    // SAFETY: as above.
    unsafe { kd_state = value };
}

/// The current keyboard mode.
fn mode() -> c_int {
    kb_mode()
}

/// `do_modifier()`: the new state for a modifier key.
fn modifier(state_in: c_int, c: u8, up: bool) -> c_int {
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
fn state2idx(state_in: c_uint, extended: bool) -> c_uint {
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
    charidx(state_idx) as c_uint
}

/// Wait for the input buffer and send a byte to the keyboard.
fn senddata(ch: u8) {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {}
    unsafe { glue::pio_outb(K_RDWR, ch) };
    state().last_sent = ch;
}

/// Wait for the input buffer and send a command to the keyboard.
fn sendcmd(ch: u8) {
    while unsafe { glue::pio_inb(K_STATUS) } & K_IBUF_FUL != 0 {}
    unsafe { glue::pio_outb(K_CMD, ch) };
}

/// Wait for a data byte from the keyboard.
fn getdata() -> u8 {
    while unsafe { glue::pio_inb(K_STATUS) } & K_OBUF_FUL == 0 {}
    unsafe { glue::pio_inb(K_RDWR) }
}

/// Complete a pending keyboard command.
fn handle_ack() {
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
fn resend() {
    if state().kd_ack == Ack::NotWaiting {
        // SAFETY: a literal format with no arguments.
        unsafe { glue::printf(c"unexpected RESEND from keyboard\n".as_ptr()) };
    } else {
        senddata(state().last_sent);
    }
}

/// The LED byte for a keyboard state.
fn leds_for_state(state_in: c_int) -> u8 {
    let mut result = 0;
    if state_in & KS_NLKED != 0 {
        result |= K_LED_NUMLK;
    }
    if state_in & KS_CLKED != 0 {
        result |= K_LED_CAPSLK;
    }
    result
}

/// Start setting the LEDs.
fn set_leds1(val: u8) {
    if state().kd_ack != Ack::NotWaiting {
        return;
    }
    state().kd_ack = Ack::SetLeds;
    state().kd_nextled = val;
    senddata(K_CMD_LEDS);
}

/// Send the LED byte after the command ack.
fn set_leds2() {
    senddata(state().kd_nextled);
}

/// `set_kd_state()`: set the state and update the LEDs.
fn set_kd_state_impl(newstate: c_int) {
    set_state_bits(newstate);
    set_leds1(leds_for_state(newstate));
}

/// `cnsetleds()`: set the LEDs without interrupts.
fn cnsetleds_impl(val: u8) {
    senddata(K_CMD_LEDS);
    let _ = getdata(); // assume ACK
    senddata(val);
    let _ = getdata(); // assume ACK
}

/// `kdgetkbent()`: read a key map entry.
fn kbent_get(row: usize, col: usize) -> [u8; NUMOUTPUT] {
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
fn kbent_set(row: usize, col: usize, value: [u8; NUMOUTPUT]) {
    // SAFETY: the caller checks the indexes.
    unsafe {
        KEY_MAP[row][col] = value[0];
        KEY_MAP[row][col + 1] = value[1];
        KEY_MAP[row][col + 2] = value[2];
    }
}

/// `mouse_button()` with the event type the magic keys use.
fn mouse_button(which: u16, direction: u8) {
    // SAFETY: called at SPLKD.
    unsafe { kd_mouse::mouse_button(which, direction) };
}

/// `mouse_moved()` with a delta scaled by the magic scale.
fn mouse_move(dx: c_int, dy: c_int) {
    let mm = crate::utils::kd_queue::MouseMotion {
        mm_delta_x: dx as i16,
        mm_delta_y: dy as i16,
    };
    // SAFETY: called at SPLKD.
    unsafe { kd_mouse::mouse_moved(mm) };
}

/// `kd_kbd_magic()`: the keyboard-as-mouse sequences.
fn kbd_magic(scancode: c_int) -> c_int {
    if state().kd_kbd_mouse == 2 {
        // SAFETY: a literal format with one integer.
        unsafe { glue::printf(c"sc = %x\n".as_ptr(), scancode) };
    }

    match scancode {
        // f1 f2 f3, with the C switch's fallthrough: 0x3b yields 1,
        // 0x3c 2, 0x3d 3.
        0x3b..=0x3d => {
            let new_button = scancode - 0x3b + 1;
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
        0x4d => mouse_move(state().kd_kbd_magic_scale, 0),
        0x4b => mouse_move(-state().kd_kbd_magic_scale, 0),
        0x48 => mouse_move(0, state().kd_kbd_magic_scale),
        0x50 => mouse_move(0, -state().kd_kbd_magic_scale),
        // home pageup end pagedown
        0x47 => mouse_move(
            -2 * state().kd_kbd_magic_scale,
            2 * state().kd_kbd_magic_scale,
        ),
        0x49 => mouse_move(
            2 * state().kd_kbd_magic_scale,
            2 * state().kd_kbd_magic_scale,
        ),
        0x4f => mouse_move(
            -2 * state().kd_kbd_magic_scale,
            -2 * state().kd_kbd_magic_scale,
        ),
        0x51 => mouse_move(
            2 * state().kd_kbd_magic_scale,
            -2 * state().kd_kbd_magic_scale,
        ),
        _ => return 0,
    }
    1
}

/// `kdcheckmagic()`: the magic key sequences.
fn checkmagic(scancode: u8) -> c_int {
    if scancode == 0x46 {
        // Scroll lock: toggle the keyboard-as-mouse hack.
        let s = state();
        s.kd_kbd_mouse = c_int::from(s.kd_kbd_mouse == 0);
        s.kd_kbd_magic_button = 0;
        return 1;
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
    0
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
    if unsafe { glue::pio_inb(K_STATUS) } & 0x20 == 0x20 {
        let sc = unsafe { glue::pio_inb(K_RDWR) };
        if unsafe { kd_mouse::mouse_in_use } != 0 {
            // SAFETY: the keyboard driver owns the device.
            unsafe { kd_mouse::mouse_handle_byte(sc) };
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
        || checkmagic(scancode) != 0
    {
        return;
    } else if mode() == KB_EVENT {
        // SAFETY: the event queue runs at SPLKD too.
        unsafe { kd_enqsc(scancode) };
        return;
    }

    let up = scancode & K_UP != 0;
    if up {
        scancode &= !K_UP;
    }
    if (scancode as usize) < NUMKEYS {
        // Look up in the map, then process.
        let mut char_idx =
            state2idx(state_bits() as c_uint, state().kd_extended) as usize;
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

/// Complete a pending keyboard command.  `kd_handle_ack()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_handle_ack() {
    handle_ack();
}

/// Resend a missed keyboard command or data byte.  `kd_resend()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_resend() {
    resend();
}

/// Change the keyboard state for a modifier key.  `do_modifier()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_modifier(
    state_in: c_int,
    c: u8,
    up: c_int,
) -> c_int {
    modifier(state_in, c, up != 0)
}

/// Check for a magic key combination.  `kdcheckmagic()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdcheckmagic(scancode: u8) -> c_int {
    checkmagic(scancode)
}

/// The key_map column for a modifier state.
/// `kdstate2idx()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdstate2idx(
    state_in: c_uint,
    extended: c_int,
) -> c_uint {
    state2idx(state_in, extended != 0)
}

/// Uppercase test.  `kd_isupper()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_isupper(c: u8) -> c_int {
    c_int::from(c.is_ascii_uppercase())
}

/// Lowercase test.  `kd_islower()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_islower(c: u8) -> c_int {
    c_int::from(c.is_ascii_lowercase())
}

/// Wait for the input buffer and send a byte.  `kd_senddata()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`; this polls the controller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_senddata(ch: u8) {
    senddata(ch);
}

/// Wait for the input buffer and send a command.  `kd_sendcmd()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`; this polls the controller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_sendcmd(ch: u8) {
    sendcmd(ch);
}

/// Wait for a data byte from the keyboard.  `kd_getdata()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`; this polls the controller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_getdata() -> u8 {
    getdata()
}

/// Write the keyboard controller command register.
/// `kd_cmdreg_write()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`; this polls the controller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_cmdreg_write(val: c_int) {
    sendcmd(KC_CMD_WRITE);
    senddata(val as u8);
}

/// Drain pending keyboard bytes, printing them.  `kd_mouse_drain()` in
/// C.
///
/// # Safety
///
/// The caller must hold `SPLKD`; this polls the controller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_mouse_drain() {
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

/// Set `kd_state` and update the LEDs.  `set_kd_state()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_kd_state(newstate: c_int) {
    set_kd_state_impl(newstate);
}

/// LED byte for a keyboard state.  `state2leds()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn state2leds(state_in: c_int) -> u8 {
    leds_for_state(state_in)
}

/// Start setting the LEDs.  `kd_setleds1()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_setleds1(val: u8) {
    set_leds1(val);
}

/// Send the LED byte after the command ack.  `kd_setleds2()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_setleds2() {
    set_leds2();
}

/// Like `kd_setleds[12]`, but not interrupt-based.  `cnsetleds()` in C.
///
/// # Safety
///
/// The caller must hold `SPLKD` and the controller must answer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnsetleds(val: u8) {
    cnsetleds_impl(val);
}

/// Keyboard-as-mouse sequences.  `kd_kbd_magic()` in C.
///
/// # Safety
///
/// Entered from the interrupt path at `SPLKD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kd_kbd_magic(scancode: c_int) -> c_int {
    kbd_magic(scancode)
}

/// Read a key map entry.  `kdgetkbent()` in C.
///
/// # Safety
///
/// `kbent` must point at a valid entry with `kb_index`/`kb_state` in
/// range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdgetkbent(kbent: *mut KbEntry) -> c_int {
    let o_pri = unsafe { glue::spltty() };
    // SAFETY: the caller promises a valid entry.
    let kb = unsafe { &mut *kbent };
    let value = kbent_get(kb.kb_index as usize, charidx(kb.kb_state as c_int));
    kb.kb_value = value;
    unsafe { glue::splx(o_pri) };
    0
}

/// Write a key map entry.  `kdsetkbent()` in C.
///
/// # Safety
///
/// `kbent` must point at a valid entry with `kb_index`/`kb_state` in
/// range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kdsetkbent(
    kbent: *mut KbEntry,
    _flags: c_int,
) -> c_int {
    let o_pri = unsafe { glue::spltty() };
    // SAFETY: the caller promises a valid entry.
    let kb = unsafe { &*kbent };
    kbent_set(
        kb.kb_index as usize,
        charidx(kb.kb_state as c_int),
        kb.kb_value,
    );
    unsafe { glue::splx(o_pri) };
    0
}
