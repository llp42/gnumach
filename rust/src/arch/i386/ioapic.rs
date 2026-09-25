// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/ioapic.c:
//   Copyright (C) 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The IOAPIC configuration and the interrupt vectors, which
//! `i386/i386at/ioapic.c` used to define and `i386/i386/apic.h` and
//! `i386/i386at/idt.h` declare.

use crate::arch::i386::apic;
use crate::arch::i386::kd::keyboard::kdintr;
use crate::arch::i386::percpu::cpu_number;
use crate::arch::i386::pio::Port;
use crate::config::{NCPUS, NINTR};
use crate::glue;
use crate::kern::mach_clock::{self, Timeout};
use crate::kern::queue::QueueEntry;
use crate::spin::Mutex;
use core::arch::asm;
use core::ffi::{c_char, c_int, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

/// `interrupt_handler_fn` of <i386/ipl.h>: one `ivect` entry, or [`None`]
/// where C leaves the vector unset.
pub type InterruptHandler = Option<unsafe extern "C" fn(c_int)>;

/// `struct irqinfo` of <i386/apic.h>: one line's programmed vector and
/// trigger mode.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IrqInfo {
    pub trigger: u8,
    pub vector: u8,
}

const _: () = {
    assert!(size_of::<IrqInfo>() == 2);
    assert!(align_of::<IrqInfo>() == 1);
    assert!(offset_of!(IrqInfo, trigger) == 0);
    assert!(offset_of!(IrqInfo, vector) == 1);
};

/// `struct ioapic_route_entry` of <i386/apic.h>: one redirection entry.
///
/// GCC packs the C bitfields least-significant first on x86, and Rust cannot
/// express them, so each accessor below masks the same bit of the raw words.
#[derive(Clone, Copy)]
struct RouteEntry {
    lo: u32,
    hi: u32,
}

/// The `vector` bitfield's shift, and the low word's other fields.
const VECTOR_MASK: u32 = 0xff;
const DELVMODE_SHIFT: u32 = 8;
const DESTMODE_SHIFT: u32 = 11;
const POLARITY_SHIFT: u32 = 13;
const TRIGGER_SHIFT: u32 = 15;
const MASK_SHIFT: u32 = 16;
/// The `dest` bitfield's shift in the high word.
const DEST_SHIFT: u32 = 24;

impl RouteEntry {
    const ZERO: Self = Self { lo: 0, hi: 0 };

    fn vector(&self) -> u8 {
        // The mask leaves eight bits, so the narrowing is exact.
        (self.lo & VECTOR_MASK) as u8
    }

    fn set_vector(&mut self, value: u32) {
        self.set_low(0, VECTOR_MASK, value);
    }

    fn trigger(&self) -> u32 {
        (self.lo >> TRIGGER_SHIFT) & 1
    }

    fn set_trigger(&mut self, value: u32) {
        self.set_low(TRIGGER_SHIFT, 1, value);
    }

    fn polarity(&self) -> u32 {
        (self.lo >> POLARITY_SHIFT) & 1
    }

    fn set_polarity(&mut self, value: u32) {
        self.set_low(POLARITY_SHIFT, 1, value);
    }

    fn set_delvmode(&mut self, value: u32) {
        self.set_low(DELVMODE_SHIFT, 0x7, value);
    }

    fn set_destmode(&mut self, value: u32) {
        self.set_low(DESTMODE_SHIFT, 1, value);
    }

    fn set_mask(&mut self, value: u32) {
        self.set_low(MASK_SHIFT, 1, value);
    }

    fn set_dest(&mut self, value: u32) {
        self.hi =
            (self.hi & !(0xff << DEST_SHIFT)) | ((value & 0xff) << DEST_SHIFT);
    }

    fn set_low(&mut self, shift: u32, mask: u32, value: u32) {
        self.lo = (self.lo & !(mask << shift)) | ((value & mask) << shift);
    }
}

/// `IOAPIC_INT_BASE` of <i386at/idt.h>: the first vector the IOAPIC raises.
const IOAPIC_INT_BASE: u32 = 0x30;

/// `LAPIC_TIMER_PERIODIC` of <i386/apic.h>: reload the LAPIC timer.
const LAPIC_TIMER_PERIODIC: u32 = 0x20000;
/// `LAPIC_TIMER_DIVIDE_2` of <i386/apic.h>.
const LAPIC_TIMER_DIVIDE_2: u32 = 0;

/// `IOAPIC_FIXED` of <i386/apic.h>: the fixed delivery mode.
const IOAPIC_FIXED: u32 = 0;
/// `IOAPIC_PHYSICAL` of <i386/apic.h>: physical destination mode.
const IOAPIC_PHYSICAL: u32 = 0;
/// `IOAPIC_ACTIVE_HIGH` of <i386/apic.h>.
const IOAPIC_ACTIVE_HIGH: u32 = 0;
/// `IOAPIC_ACTIVE_LOW` of <i386/apic.h>.
const IOAPIC_ACTIVE_LOW: u32 = 1;
/// `IOAPIC_EDGE_TRIGGERED` of <i386/apic.h>.
const IOAPIC_EDGE_TRIGGERED: u32 = 0;
/// `IOAPIC_LEVEL_TRIGGERED` of <i386/apic.h>.
const IOAPIC_LEVEL_TRIGGERED: u32 = 1;
/// `IOAPIC_MASK_ENABLED` of <i386/apic.h>.
const IOAPIC_MASK_ENABLED: u32 = 0;
/// `IOAPIC_MASK_DISABLED` of <i386/apic.h>.
const IOAPIC_MASK_DISABLED: u32 = 1;

/// `APIC_IRQ_OVERRIDE_POLARITY_MASK` of <i386/apic.h>.
const APIC_IRQ_OVERRIDE_POLARITY_MASK: u16 = 1;
/// `APIC_IRQ_OVERRIDE_ACTIVE_LOW` of <i386/apic.h>.
const APIC_IRQ_OVERRIDE_ACTIVE_LOW: u16 = 2;
/// `APIC_IRQ_OVERRIDE_TRIGGER_MASK` of <i386/apic.h>.
const APIC_IRQ_OVERRIDE_TRIGGER_MASK: u16 = 4;
/// `APIC_IRQ_OVERRIDE_LEVEL_TRIGGERED` of <i386/apic.h>.
const APIC_IRQ_OVERRIDE_LEVEL_TRIGGERED: u16 = 8;

/// `ACPI_PICMODE_APIC` of <device/irq_status.h>.
const ACPI_PICMODE_APIC: c_int = 1;
/// `PIC_SLAVE_OCW` of the removed <i386/pic.h>: the 8259 slave OCW port.
const PIC_SLAVE_OCW: u16 = 0xa1;
/// `PIC_MASTER_OCW` of the removed <i386/pic.h>: the 8259 master OCW port.
const PIC_MASTER_OCW: u16 = 0x21;
/// `PICS_MASK` of the removed <i386/pic.h>: every slave line masked.
const PICS_MASK: u8 = 0xff;
/// `PICM_MASK` of the removed <i386/pic.h>: every master line masked.
const PICM_MASK: u8 = 0xff;
/// `SPLHI` of <i386/ipl.h>.
const SPLHI: c_int = 7;

/// `ivect` of `i386/i386at/ioapic.c`, which <i386/ipl.h> declares.  The
/// interrupt stubs index it directly, so this layout is the ABI.
#[unsafe(no_mangle)]
pub static mut ivect: [InterruptHandler; NINTR] = {
    let mut table: [InterruptHandler; NINTR] = [Some(intnull); NINTR];
    // SAFETY: The C cast `hardclock` to `interrupt_handler_fn` for this slot;
    // the trampoline passes it the one `int` its own entry point ignores.
    table[0] = Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(
                c_int,
                c_int,
                *const c_char,
                *mut crate::arch::i386::pcb::I386InterruptState,
            ),
            unsafe extern "C" fn(c_int),
        >(crate::arch::i386::hardclock_ffi::hardclock)
    });
    table[1] = Some(kdintr);
    table[13] = Some(crate::arch::i386::fpu_ffi::fpintr);
    table
};

/// `iunit` of `i386/i386at/ioapic.c`, which <i386/ipl.h> and
/// `i386/i386at/interrupt.S` index.
#[unsafe(no_mangle)]
pub static mut iunit: [c_int; NINTR] = iunit_image();

/// The `iunit` initializer of `i386/i386at/ioapic.c`: each line maps to
/// itself.
const fn iunit_image() -> [c_int; NINTR] {
    let mut table = [0; NINTR];
    let mut irq = 0;
    while irq < NINTR {
        // NINTR is 64, so the narrowing to `int` loses nothing.
        table[irq] = irq as c_int;
        irq += 1;
    }
    table
}

/// `curr_ipl` of `i386/i386at/ioapic.c`, which <i386/ipl.h> declares and
/// `spl.S` reads and writes.
#[unsafe(no_mangle)]
pub static mut curr_ipl: [c_int; NCPUS] = [0; NCPUS];

/// `spl_init` of <i386/spl.h>.
#[unsafe(no_mangle)]
pub static mut spl_init: c_int = 0;

/// `pic_mode` of `i386/i386at/ioapic.c`: the PIC mode the platform runs in.
#[unsafe(no_mangle)]
pub static mut pic_mode: c_int = ACPI_PICMODE_APIC;

/// `timer_pin` of <i386/apic.h>: the pin `ioapic_configure()` remapped the
/// timer to.
#[unsafe(no_mangle)]
pub static mut timer_pin: c_int = 0;

/// `irqinfo` of <i386/apic.h>: one entry per interrupt line.
#[unsafe(no_mangle)]
pub static mut irqinfo: [IrqInfo; NINTR] = [IrqInfo {
    trigger: 0,
    vector: 0,
}; NINTR];

/// `lapic_timer_val` of `i386/i386at/ioapic.c`.
#[unsafe(no_mangle)]
pub static mut lapic_timer_val: u32 = 0;

/// `calibrated_ticks` of `i386/i386at/ioapic.c`: the LAPIC timer ticks per
/// Mach tick.
#[unsafe(no_mangle)]
pub static mut calibrated_ticks: u32 = 0;

/// `has_irq_specific_eoi` of `i386/i386at/ioapic.c`.
static HAS_IRQ_SPECIFIC_EOI: AtomicBool = AtomicBool::new(false);

/// `ioapic_lock` of `i386/i386at/ioapic.c`: serializes the non-atomic
/// select/window register pairs.
static IOAPIC_LOCK: Mutex<()> = Mutex::new(());

/// `APIC_IO_REDIR_LOW(pin)` of <i386/apic.h>: the low redirection register.
fn redir_low(pin: c_int) -> u32 {
    // Pins are below 64, so the offset stays far inside the register byte.
    (0x10 + pin * 2) as u32
}

/// `APIC_IO_REDIR_HIGH(pin)` of <i386/apic.h>: the high redirection register.
fn redir_high(pin: c_int) -> u32 {
    // As `redir_low()`.
    (0x11 + pin * 2) as u32
}

/// The body of `ioapic_read()` in C: read one IOAPIC register.
fn read(apic: c_int, reg: u32) -> u32 {
    let Some(ioapic) = apic::ioapic(apic) else {
        return 0;
    };
    // SAFETY: `ioapic` points into `apic_data`, whose entries stay live for
    // the kernel's life.
    let unit = unsafe { (*ioapic.as_ptr()).ioapic };
    if unit.is_null() {
        // The MADT published no window for this IOAPIC.
        return 0;
    }
    // SAFETY: `unit` is the mapped register window; the select store and
    // window load are the C's volatile accesses.
    unsafe {
        ptr::write_volatile(&raw mut (*unit).select.r, reg);
        ptr::read_volatile(&raw const (*unit).window.r)
    }
}

/// The body of `ioapic_write()` in C: write one IOAPIC register.
fn write(apic: c_int, reg: u32, value: u32) {
    let Some(ioapic) = apic::ioapic(apic) else {
        return;
    };
    // SAFETY: as `read()`.
    let unit = unsafe { (*ioapic.as_ptr()).ioapic };
    if unit.is_null() {
        return;
    }
    // SAFETY: as `read()`; both stores are the C's volatile accesses.
    unsafe {
        ptr::write_volatile(&raw mut (*unit).select.r, reg);
        ptr::write_volatile(&raw mut (*unit).window.r, value);
    }
}

/// The body of `ioapic_read_entry()` in C.
fn read_entry(apic: c_int, pin: c_int) -> RouteEntry {
    RouteEntry {
        lo: read(apic, redir_low(pin)),
        hi: read(apic, redir_high(pin)),
    }
}

/// The body of `ioapic_write_entry()` in C.  The high word goes first
/// because the mask bit lives in the low word.
fn write_entry(apic: c_int, pin: c_int, entry: RouteEntry) {
    write(apic, redir_high(pin), entry.hi);
    write(apic, redir_low(pin), entry.lo);
}

/// The body of `ioapic_toggle_entry()` in C: change only the low word, so
/// the mask bit flips without rewriting the entry.
fn toggle_entry(apic: c_int, pin: c_int, mask: u32) {
    // SAFETY: `splhigh()` is the real asm routine <machine/spl.h> declares,
    // and its result is only handed back to `splx()`.
    let saved = unsafe { glue::splhigh() };
    {
        let _guard = IOAPIC_LOCK.lock();
        let mut entry = read_entry(apic, pin);
        entry.set_mask(mask & 1);
        write(apic, redir_low(pin), entry.lo);
    }
    // SAFETY: `saved` is the level `splhigh()` returned above.
    unsafe { glue::splx(saved) };
}

/// The body of `ioapic_version()` in C.
fn version(apic: c_int) -> c_int {
    let raw = read(apic, apic::APIC_IO_VERSION);
    // The mask leaves eight bits, so the narrowing is exact.
    c_int::from(((raw >> apic::APIC_IO_VERSION_SHIFT) & 0xff) as u8)
}

/// The body of `ioapic_gsis()` in C.
fn gsis(apic: c_int) -> c_int {
    let raw = read(apic, apic::APIC_IO_VERSION);
    // The mask leaves eight bits, and the field counts entries from zero.
    c_int::from(((raw >> apic::APIC_IO_ENTRIES_SHIFT) & 0xff) as u8) + 1
}

/// The override whose IRQ is `pin`, if the MADT has one.
fn override_for(pin: c_int) -> Option<apic::IrqOverrideData> {
    let pin = u8::try_from(pin).ok()?;
    // SAFETY: `irq_override()` answers a pointer into `apic_data`, whose
    // entries stay live for the kernel's life; the copy drops the borrow.
    apic::irq_override(pin).map(|over| unsafe { *over.as_ptr() })
}

/// Record one line's programmed vector and trigger for the EOI path.
fn set_irqinfo(pin: c_int, entry: RouteEntry) {
    let Ok(index) = usize::try_from(pin) else {
        return;
    };
    if NINTR <= index {
        return;
    }
    let info = IrqInfo {
        // The trigger bitfield is one bit wide.
        trigger: entry.trigger() as u8,
        vector: entry.vector(),
    };
    // SAFETY: `index` is inside `irqinfo`, which only this module writes.
    unsafe { (&raw mut irqinfo).cast::<IrqInfo>().add(index).write(info) };
}

/// The vector `irqinfo[pin]` holds, or zero when no entry was programmed.
fn irqinfo_vector(pin: c_int) -> u8 {
    let Ok(index) = usize::try_from(pin) else {
        return 0;
    };
    if NINTR <= index {
        return 0;
    }
    // SAFETY: `index` is inside `irqinfo`.
    unsafe { (*(&raw const irqinfo).cast::<IrqInfo>().add(index)).vector }
}

/// The body of `override_irq()` in C: apply one MADT override to `entry` and
/// answer the GSI it selects.
fn override_irq(over: &apic::IrqOverrideData, entry: &mut RouteEntry) -> u32 {
    if over.flags & APIC_IRQ_OVERRIDE_TRIGGER_MASK != 0 {
        entry.set_trigger(
            if over.flags & APIC_IRQ_OVERRIDE_LEVEL_TRIGGERED != 0 {
                IOAPIC_LEVEL_TRIGGERED
            } else {
                IOAPIC_EDGE_TRIGGERED
            },
        );
    } else if over.bus == 0 {
        // ISA is edge-triggered by default.
        entry.set_trigger(IOAPIC_EDGE_TRIGGERED);
    } else {
        entry.set_trigger(IOAPIC_LEVEL_TRIGGERED);
    }

    if over.flags & APIC_IRQ_OVERRIDE_POLARITY_MASK != 0 {
        entry.set_polarity(
            if over.flags & APIC_IRQ_OVERRIDE_ACTIVE_LOW != 0 {
                IOAPIC_ACTIVE_LOW
            } else {
                IOAPIC_ACTIVE_HIGH
            },
        );
    } else if over.bus == 0 {
        // EISA is active-low for level-triggered interrupts.
        if entry.trigger() == IOAPIC_LEVEL_TRIGGERED {
            entry.set_polarity(IOAPIC_ACTIVE_LOW);
        } else {
            entry.set_polarity(IOAPIC_ACTIVE_HIGH);
        }
    }

    // SAFETY: `printf` is the real C routine <kern/printf.h> declares; the
    // `%d`s take the two integer varargs and the `%s`s the static strings.
    unsafe {
        glue::printf(
            c"IRQ override: pin=%d gsi=%d trigger=%s polarity=%s\n".as_ptr(),
            c_int::from(over.irq),
            // The C passed the `uint32_t` to `%d`, which reads the same bits.
            over.gsi as c_int,
            if entry.trigger() == IOAPIC_LEVEL_TRIGGERED {
                c"LEVEL"
            } else {
                c"EDGE"
            }
            .as_ptr(),
            if entry.polarity() == IOAPIC_ACTIVE_LOW {
                c"LOW"
            } else {
                c"HIGH"
            }
            .as_ptr(),
        )
    };
    over.gsi
}

/// The body of `picdisable()` in C: stop the 8259s and raise every CPU's
/// software IPL.
fn disable_pic() {
    // SAFETY: `cli` disables interrupts; it touches no memory and uses no
    // stack.
    unsafe { asm!("cli", options(nostack, nomem)) };
    // SAFETY: `curr_ipl` is the array `spl.S` reads; the store replaces the
    // whole NCPUS-entry image the C's loop built.
    unsafe { (&raw mut curr_ipl).write([SPLHI; NCPUS]) };
    Port::new(PIC_SLAVE_OCW).write_u8(PICS_MASK);
    Port::new(PIC_MASTER_OCW).write_u8(PICM_MASK);
}

/// `timer_expiry_callback()` of `i386/i386at/ioapic.c`: mark the measurement
/// finished.
unsafe extern "C" fn timer_expiry_callback(arg: *mut c_void) {
    // SAFETY: `arg` is the address of the measuring frame's `done` flag, as
    // `measure_10x_apic_hz()` passed it.
    unsafe { ptr::write_volatile(arg.cast::<c_int>(), 1) };
}

/// The body of `timer_measure_10x_apic_hz()` in C: time the LAPIC timer
/// against ten Mach ticks.
fn measure_10x_apic_hz() -> u32 {
    let mut done: c_int = 0;
    let mut timer = Timeout {
        chain: QueueEntry::unlinked(),
        fcn: Some(timer_expiry_callback),
        param: (&raw mut done).cast::<c_void>(),
        t_time: 0,
        set: 0,
    };
    let unit = apic::lapic_ptr();
    let start = u32::MAX;

    // SAFETY: `printf` is the real C routine <kern/printf.h> declares; this
    // format has no conversion specifier.
    unsafe { glue::printf(c"timer calibration...".as_ptr()) };

    // SAFETY: `unit` is the mapped local-APIC page.
    unsafe { apic::reg_write(&raw mut (*unit).init_count, start) };

    // SAFETY: `timer` is a live element that stays at this address until the
    // timeout expires.
    unsafe { mach_clock::set_timeout(&raw mut timer, 10) };

    loop {
        // SAFETY: `done` is written only by the expiry callback, through a
        // volatile store, so the loop cannot cache it.
        if unsafe { ptr::read_volatile(&raw const done) } != 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // SAFETY: `unit` is the mapped local-APIC page; the load and the masked
    // store are the C's volatile accesses.
    unsafe {
        let value = apic::reg_read(&raw const (*unit).lvt_timer);
        apic::reg_write(
            &raw mut (*unit).lvt_timer,
            value | apic::LAPIC_DISABLE,
        );
    }

    // SAFETY: as the first message.
    unsafe { glue::printf(c" done\n".as_ptr()) };

    // SAFETY: `unit` is the mapped local-APIC page; the subtraction wraps on
    // the 32-bit counter exactly as the C's unsigned arithmetic did.
    unsafe { start.wrapping_sub(apic::reg_read(&raw const (*unit).cur_count)) }
}

/// The body of `calibrate_lapic_timer()` in C.
fn calibrate_timer() {
    let unit = apic::lapic_ptr();

    // SAFETY: `unit` is the mapped local-APIC page; both stores are the C's
    // volatile register writes.
    unsafe {
        apic::reg_write(&raw mut (*unit).divider_config, LAPIC_TIMER_DIVIDE_2);
        apic::reg_write(&raw mut (*unit).lvt_timer, IOAPIC_INT_BASE);
    }

    // SAFETY: `calibrated_ticks` is written only here and read by
    // `lapic_enable_timer()`, both at boot on one CPU.
    if unsafe { calibrated_ticks } == 0 {
        // SAFETY: `splhigh()` is the real asm routine <machine/spl.h>
        // declares, and its result is only handed back to `splx()`.
        let saved = unsafe { glue::splhigh() };
        // SAFETY: `spl0()` is the real asm routine <machine/spl.h> declares.
        unsafe { glue::spl0() };
        let ticks = measure_10x_apic_hz() / 10;
        // SAFETY: as the load above.
        unsafe { calibrated_ticks = ticks };
        // SAFETY: `saved` is the level `splhigh()` returned above.
        unsafe { glue::splx(saved) };
    }
}

/// The body of `lapic_enable_timer()` in C.
fn enable_timer() {
    let unit = apic::lapic_ptr();
    // SAFETY: as the load in `calibrate_timer()`.
    let ticks = unsafe { calibrated_ticks };

    // SAFETY: `unit` is the mapped local-APIC page; the stores are the C's
    // volatile register writes, including the divider rewrite that buggy
    // hardware needs.
    unsafe {
        apic::reg_write(&raw mut (*unit).init_count, ticks);
        apic::reg_write(&raw mut (*unit).divider_config, LAPIC_TIMER_DIVIDE_2);
        apic::reg_write(
            &raw mut (*unit).lvt_timer,
            IOAPIC_INT_BASE | LAPIC_TIMER_PERIODIC,
        );
        apic::reg_write(&raw mut (*unit).divider_config, LAPIC_TIMER_DIVIDE_2);
    }

    // SAFETY: `printf` is the real C routine; the one `%d` takes the
    // matching `c_int` vararg.
    unsafe {
        glue::printf(
            c"LAPIC timer configured on cpu%d\n".as_ptr(),
            cpu_number(),
        )
    };
}

/// The body of `ioapic_configure()` in C: program the IOAPICs from the MADT
/// data.
fn configure() {
    let mut apic: c_int = 0;
    let version = version(apic);
    let ngsis = gsis(apic);

    if 0x20 <= version {
        // The store happens at boot and the interrupt path only reads it, so
        // the boot sequence, not the ordering, publishes the flag.
        HAS_IRQ_SPECIFIC_EOI.store(true, Ordering::Relaxed);
    }

    // SAFETY: `printf` is the real C routine; the one `%x` takes the
    // matching `c_int` vararg.
    unsafe { glue::printf(c"IOAPIC version 0x%x\n".as_ptr(), version) };

    let unit = apic::lapic_ptr();
    // SAFETY: `unit` is the mapped local-APIC page.
    unsafe {
        apic::reg_write(
            &raw mut (*unit).spurious_vector,
            apic::IOAPIC_SPURIOUS_BASE,
        )
    };

    let mut entry = RouteEntry::ZERO;
    entry.set_delvmode(IOAPIC_FIXED);
    entry.set_destmode(IOAPIC_PHYSICAL);
    entry.set_mask(IOAPIC_MASK_DISABLED);
    // SAFETY: `unit` is the mapped local-APIC page.  The C read the APIC ID
    // from the register, not from `apic_id_mask`, because the IOAPIC uses
    // what is actually set there.
    let apic_id = unsafe { apic::reg_read(&raw const (*unit).apic_id) };
    entry.set_dest(apic_id >> 24);

    let mut timer_gsi = 0;
    for pin in 0..14 {
        let mut gsi = u32::try_from(pin).unwrap_or(0);
        entry.set_trigger(IOAPIC_EDGE_TRIGGERED);
        entry.set_polarity(IOAPIC_ACTIVE_HIGH);
        if let Some(over) = override_for(pin) {
            gsi = override_irq(&over, &mut entry);
        }
        entry.set_vector(IOAPIC_INT_BASE + gsi);
        write_entry(apic, pin, entry);
        set_irqinfo(pin, entry);
        mask(pin);

        // Legacy IRQ 0 is the timer unless an override remapped it.
        if pin == 0 {
            timer_gsi = gsi;
        } else if gsi == timer_gsi {
            // SAFETY: `timer_pin` is this module's global, read at boot by
            // `startrtclock()`.
            unsafe { timer_pin = pin };
            entry.set_vector(IOAPIC_INT_BASE);
            write_entry(apic, pin, entry);
            mask(0);
        }
    }

    // SAFETY: as the version message; the one `%d` takes the matching
    // `c_int`.
    unsafe {
        glue::printf(
            c"IOAPIC 0 configured with GSI 0-%d\n".as_ptr(),
            ngsis - 1,
        )
    };

    if 1 < apic::num_ioapics() {
        apic = 1;
        let ngsis2 = gsis(apic);
        for pin in 0..ngsis2 {
            let mut gsi = u32::try_from(pin + ngsis).unwrap_or(0);
            entry.set_trigger(IOAPIC_LEVEL_TRIGGERED);
            entry.set_polarity(IOAPIC_ACTIVE_LOW);
            if let Some(over) = override_for(pin + ngsis) {
                gsi = override_irq(&over, &mut entry);
            }
            entry.set_vector(IOAPIC_INT_BASE + gsi);
            write_entry(apic, pin, entry);
            set_irqinfo(pin + ngsis, entry);
            mask(pin + ngsis);
        }

        // SAFETY: as the version message; the two `%d`s take the matching
        // `c_int`s.
        unsafe {
            glue::printf(
                c"IOAPIC 1 configured with GSI %d-%d\n".as_ptr(),
                ngsis,
                ngsis + ngsis2 - 1,
            )
        };
    }

    apic::setup();
    apic::enable();
}

/// `mask_irq()` of <i386/apic.h>: disable the line.
pub(crate) fn mask(pin: c_int) {
    toggle(0, pin, IOAPIC_MASK_DISABLED);
}

/// `unmask_irq()` of <i386/apic.h>: enable the line.
pub(crate) fn unmask(pin: c_int) {
    toggle(0, pin, IOAPIC_MASK_ENABLED);
}

/// The body of `ioapic_toggle()` in C.
fn toggle(apic: c_int, pin: c_int, mask: u32) {
    toggle_entry(apic, pin, mask);
}

/// The body of `ioapic_irq_eoi()` in C, ending the interrupt on the LAPIC.
pub(crate) fn irq_eoi(pin: c_int) {
    if pin != 0 {
        // SAFETY: `splhigh()` is the real asm routine <machine/spl.h>
        // declares, and its result is only handed back to `splx()`.
        let saved = unsafe { glue::splhigh() };
        {
            let _guard = IOAPIC_LOCK.lock();
            if !HAS_IRQ_SPECIFIC_EOI.load(Ordering::Relaxed) {
                // An IOAPIC with no specific EOI needs the pin masked and
                // edge-triggered around the acknowledgement.
                let mut entry = read_entry(0, pin);
                let old = entry;
                entry.set_mask(IOAPIC_MASK_DISABLED);
                entry.set_trigger(IOAPIC_EDGE_TRIGGERED);
                write_entry(0, pin, entry);
                write_entry(0, pin, old);
            } else if let Some(ioapic) = apic::ioapic(0) {
                // SAFETY: `ioapic` points into `apic_data`.
                let unit = unsafe { (*ioapic.as_ptr()).ioapic };
                if !unit.is_null() {
                    let vector = irqinfo_vector(pin);
                    // SAFETY: `unit` is the mapped register window.
                    unsafe {
                        ptr::write_volatile(
                            &raw mut (*unit).eoi.r,
                            u32::from(vector),
                        )
                    };
                }
            }
        }
        // SAFETY: `saved` is the level `splhigh()` returned above.
        unsafe { glue::splx(saved) };
    }
    apic::eoi();
}

/// `ioapic_toggle()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ioapic_toggle(pin: c_int, mask: c_int) {
    // The C kept the mask's low bit, sign included.
    toggle(0, pin, u32::from(mask & 1 != 0));
}

/// `ioapic_irq_eoi()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ioapic_irq_eoi(pin: c_int) {
    irq_eoi(pin);
}

/// `picdisable()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn picdisable() {
    disable_pic();
}

/// `calibrate_lapic_timer()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn calibrate_lapic_timer() {
    calibrate_timer();
}

/// `lapic_enable_timer()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn lapic_enable_timer() {
    enable_timer();
}

/// `ioapic_configure()` in C.
#[unsafe(no_mangle)]
pub extern "C" fn ioapic_configure() {
    configure();
}

/// Report the interrupt on a pin that has no handler.
fn null(unit: c_int) {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares; the one
    // `%d` conversion takes the matching `c_int` vararg.
    unsafe { glue::printf(c"intnull(%d)\n".as_ptr(), unit) };
}

/// Report the interrupt on a pin that has no handler.
///
/// # Safety
///
/// No precondition: the function only formats its argument, and the C
/// prototype's contract is likewise empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn intnull(unit_dev: c_int) {
    null(unit_dev);
}
