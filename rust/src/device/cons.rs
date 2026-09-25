// SPDX-License-Identifier: CMU-Mach
// Derived from device/cons.c and device/cons.h:
//   Copyright (c) 1988-1994, The University of Utah and
//   the Computer Systems Laboratory (CSL).  All rights reserved.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The console package, which `device/cons.c` used to define and
//! <device/cons.h> declares.
//!
//! The console table itself, `constab`, belongs to the machine's
//! `i386/i386at/cons_conf.c` and is declared in `glue`; the device-name
//! lookup it is wired through is [`crate::device::dev_name`].

use crate::arch::i386::kd::ConsDev;
use crate::device::dev_name;
use crate::device::kmsg;
use crate::glue;
use core::ffi::{c_char, c_int, c_short};
use core::ptr;

/// `CN_DEAD` of <device/cons.h>: a console that does not exist.
const CN_DEAD: c_short = 0;

/// `CONSBUFSIZE` of <device/cons.h>.
const CONSBUFSIZE: usize = 1024;

/// `cn_inited` of `device/cons.c`.
static mut CN_INITED: bool = false;

/// `cn_tab` of `device/cons.c`: the chosen console.
static mut CN_TAB: *mut ConsDev = ptr::null_mut();

/// `romgetc` of `device/cons.c`: the boot ROM's character input, if any.
#[unsafe(export_name = "romgetc")]
pub static mut ROMGETC: Option<unsafe extern "C" fn(c_char) -> c_int> = None;

/// `romputc` of `device/cons.c`: the boot ROM's character output, if any.
#[unsafe(export_name = "romputc")]
pub static mut ROMPUTC: Option<unsafe extern "C" fn(c_char)> = None;

/// `consbuf` of `device/cons.c`: the output held until a console is chosen.
static mut CONSBUF: [c_char; CONSBUFSIZE] = [0; CONSBUFSIZE];

/// `consbp` of `device/cons.c`.
static mut CONSBP: *mut c_char = ptr::addr_of_mut!(CONSBUF).cast::<c_char>();

/// `consbufused` of `device/cons.c`.
static mut CONSBUFUSED: bool = false;

/// `cninit()` of `device/cons.c`: find and initialize the console.
///
/// # Safety
///
/// Called once, during the boot, after the device tables and `constab` exist
/// and before any console user runs.
pub(crate) unsafe fn init() {
    // SAFETY: the flag is this module's and the boot is single-threaded.
    if unsafe { CN_INITED } {
        return;
    }

    let constab = ptr::addr_of_mut!(glue::constab);
    let mut cp = constab;
    let mut chosen: *mut ConsDev = ptr::null_mut();
    // SAFETY: `constab` is the C table, terminated by an entry whose
    // `cn_probe` is null, which `Option` reads as `None`.
    while let Some(probe) = unsafe { (*cp).cn_probe } {
        // SAFETY: the probe entry is inside `constab` and the probe routine
        // is the entry's own.
        unsafe { probe(cp) };
        // SAFETY: `cp` is the probed entry.
        if unsafe { (*cp).cn_pri } > CN_DEAD
            && (chosen.is_null()
                || unsafe { (*cp).cn_pri } > unsafe { (*chosen).cn_pri })
        {
            chosen = cp;
            // SAFETY: `cn_tab` is this module's, and the C named the best
            // entry as soon as it saw one.
            unsafe { CN_TAB = cp };
        }
        // SAFETY: the probe entry is inside `constab`, whose terminator ends
        // the walk.
        cp = unsafe { cp.add(1) };
    }

    if chosen.is_null() {
        // SAFETY: `Panic` does not return; the file, function and message are
        // the C `panic()` macro's, and the line is this Rust file's.
        unsafe {
            glue::Panic(
                c"device/cons.c".as_ptr(),
                line!() as c_int,
                c"cninit".as_ptr(),
                c"can't find a console device".as_ptr(),
            )
        }
    }

    // SAFETY: `chosen` is a probed table entry with its `cn_init` filled by
    // the probe.
    if let Some(initialize) = unsafe { (*chosen).cn_init } {
        // SAFETY: as above.
        unsafe { initialize(chosen) };
    }

    // SAFETY: the console's name is a NUL-terminated string in the table.
    let Some((ops, _unit)) = (unsafe { dev_name::lookup((*chosen).cn_name) })
    else {
        // SAFETY: `Panic` does not return; the tags are the C's own.
        unsafe {
            glue::Panic(
                c"device/cons.c".as_ptr(),
                line!() as c_int,
                c"cninit".as_ptr(),
                c"cninit: dev_name_lookup failed".as_ptr(),
            )
        }
    };
    // `minor()` of <sys/types.h>: the low byte of the device number.
    // SAFETY: `chosen` is a live table entry.
    let minor = c_int::from(unsafe { (*chosen).cn_dev } & 0xff);
    // SAFETY: the indirect table is the C's, and `ops` is a live entry point
    // table.
    unsafe {
        dev_name::set_indirection(c"console".as_ptr(), ops.as_ptr(), minor)
    };

    // SAFETY: the console is initialized, so the pending buffer can be
    // flushed through it.
    unsafe { flush_pending() };
    // SAFETY: this boot step is the flag's only writer.
    unsafe { CN_INITED = true };
}

/// The `consbufused` flush at the end of `cninit()`.
///
/// # Safety
///
/// The console must be initialized; the buffer is this module's and nothing
/// else flushes it concurrently.
unsafe fn flush_pending() {
    // SAFETY: the flag and pointer are this module's.
    if !unsafe { CONSBUFUSED } {
        return;
    }
    let base = ptr::addr_of_mut!(CONSBUF).cast::<c_char>();
    let start = unsafe { CONSBP };
    let mut cbp = start;
    loop {
        // SAFETY: `cbp` walks the 1024-byte buffer, which is initialized.
        let byte = unsafe { *cbp };
        if byte != 0 {
            // SAFETY: as the caller; the console is initialized, so the C
            // recursed into `cnputc()` here.
            unsafe { putc(byte) };
        }
        // SAFETY: the walk stays inside the buffer, as the C's did.
        cbp = unsafe { cbp.add(1) };
        if cbp == unsafe { base.add(CONSBUFSIZE) } {
            cbp = base;
        }
        if cbp == start {
            break;
        }
    }
    // SAFETY: this routine is the flag's only writer.
    unsafe { CONSBUFUSED = false };
}

/// `cngetc()` and `cnmaygetc()` of `device/cons.c`.
///
/// # Safety
///
/// The console and ROM tables are the machine's; `wait` selects the blocking
/// or polling read, exactly as the C passed `1` or `0`.
pub(crate) unsafe fn getc(wait: c_int) -> c_int {
    // SAFETY: `CN_TAB` is the chosen entry, or null before `cninit()`.
    let tab = unsafe { CN_TAB };
    if !tab.is_null() {
        // SAFETY: a chosen console has `cn_getc` filled by its probe.
        if let Some(cn_getc) = unsafe { (*tab).cn_getc } {
            // SAFETY: as above; the device number and wait are plain values.
            return unsafe { cn_getc((*tab).cn_dev, wait) };
        }
    }
    // SAFETY: the ROM pointer is null until a boot ROM installs one.
    if let Some(romgetc) = unsafe { ROMGETC } {
        // SAFETY: as above; the C passed the literal `1` or `0` as the `char`
        // argument.
        return unsafe { romgetc(wait as c_char) };
    }
    0
}

/// `cnputc()` of `device/cons.c`.
///
/// # Safety
///
/// The console and ROM tables are the machine's, and the pending buffer is
/// this module's.
pub(crate) unsafe fn putc(c: c_char) {
    if c == 0 {
        return;
    }

    kmsg::putchar(c_int::from(c));

    // SAFETY: `CN_TAB` is the chosen entry, or null before `cninit()`.
    let tab = unsafe { CN_TAB };
    if !tab.is_null() {
        // SAFETY: a chosen console has `cn_putc` filled by its probe.
        if let Some(cn_putc) = unsafe { (*tab).cn_putc } {
            // SAFETY: as above; the device number and character are plain
            // values.
            unsafe {
                cn_putc((*tab).cn_dev, c_int::from(c));
                if c == b'\n' as c_char {
                    cn_putc((*tab).cn_dev, c_int::from(b'\r'));
                }
            }
        }
    // SAFETY: the ROM pointer is null until a boot ROM installs one.
    } else if let Some(romputc) = unsafe { ROMPUTC } {
        // SAFETY: as above; the character is a plain value.
        unsafe {
            romputc(c);
            if c == b'\n' as c_char {
                romputc(b'\r' as c_char);
            }
        }
    } else {
        // SAFETY: the buffer is this module's and no console exists yet, so
        // the C buffered the byte.
        unsafe { buffer_char(c) };
    }
}

/// Hold `c` in `consbuf` until a console is chosen, as the `CONSBUFSIZE > 0`
/// arm of `cnputc()` did.
///
/// # Safety
///
/// Nothing else may write the buffer concurrently: the C ran this before any
/// console existed.
unsafe fn buffer_char(c: c_char) {
    let base = ptr::addr_of_mut!(CONSBUF).cast::<c_char>();
    // SAFETY: the flag, pointer and buffer are this module's.
    if !unsafe { CONSBUFUSED } {
        // SAFETY: as above; the C cleared the buffer with `memset()`.
        unsafe {
            CONSBP = base;
            CONSBUFUSED = true;
            ptr::write_bytes(base, 0, CONSBUFSIZE);
        }
    }
    // SAFETY: `CONSBP` always addresses the 1024-byte buffer.
    unsafe {
        let slot = CONSBP;
        slot.write(c);
        CONSBP = slot.add(1);
        if CONSBP >= base.add(CONSBUFSIZE) {
            CONSBP = base;
        }
    }
}
