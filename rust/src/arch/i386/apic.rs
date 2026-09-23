// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386/apic.c:
//   Copyright (C) 2020 Free Software Foundation, Inc.
//   Written by Almudena Garcia Jurado-Centurion
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The APIC and HPET accessors, which `i386/i386/apic.c` used to define and
//! `i386/i386/apic.h` declares; the two `hpclock_*` entries are also declared
//! in <kern/mach_clock.h>.

use crate::glue;
use core::arch::asm;
use core::ffi::{c_int, c_void};
use core::ptr::NonNull;

/// `HPET_CAP_PERIOD` of `i386/i386/apic.c`: the tick-period register.
const HPET_CAP_PERIOD: usize = 0x04;
/// `HPET_CFG`: the configuration register.
const HPET_CFG: usize = 0x10;
/// `HPET_CFG_ENABLE`: start the main counter.
const HPET_CFG_ENABLE: u32 = 1 << 0;
/// `HPET_LEGACY_ROUTE`: route timer 0 through the 8254 interrupt.
const HPET_LEGACY_ROUTE: u32 = 1 << 1;
/// `HPET_COUNTER`: the main counter register.
const HPET_COUNTER: usize = 0xf0;
/// `HPET_T0_CFG`: timer 0's configuration register.
const HPET_T0_CFG: usize = 0x100;
/// `HPET_T0_32BIT_MODE`: keep the comparator 32 bits wide.
const HPET_T0_32BIT_MODE: u32 = 1 << 8;
/// `HPET_T0_VAL_SET`: latch the comparator value.
const HPET_T0_VAL_SET: u32 = 1 << 6;
/// `HPET_T0_TYPE_PERIODIC`: reload the comparator in periodic mode.
const HPET_T0_TYPE_PERIODIC: u32 = 1 << 3;
/// `HPET_T0_INT_ENABLE`: let timer 0 raise an interrupt.
const HPET_T0_INT_ENABLE: u32 = 1 << 2;
/// `HPET_T0_COMPARATOR`: timer 0's comparator register.
const HPET_T0_COMPARATOR: usize = 0x108;

/// `FSEC_PER_NSEC`: femtoseconds in a nanosecond.
const FSEC_PER_NSEC: u32 = 1_000_000;
/// `NSEC_PER_USEC`: nanoseconds in a microsecond.
const NSEC_PER_USEC: u32 = 1000;

/// The longest wait one conversion covers, `0xffffffff / NSEC_PER_USEC`.
const MAX_DELAY_USEC: u32 = u32::MAX / NSEC_PER_USEC;

/// The mapped HPET register window.
///
/// # Invariants
///
/// `base` names the register block `i386/i386at/acpi_parse_apic.c` mapped for
/// the HPET, so the register constants above are valid byte offsets into it.
struct Hpet {
    base: NonNull<u8>,
}

impl Hpet {
    /// The HPET ACPI found and mapped, or `None` when the machine has none.
    fn new() -> Option<Self> {
        // SAFETY: `hpet_addr` is the C global <i386/apic.h> declares.
        let base = unsafe { glue::hpet_addr };
        let base = NonNull::new(base.cast::<u8>())?;
        Some(Self { base })
    }

    /// Read the 32-bit register at byte `offset`.
    fn read(&self, offset: usize) -> u32 {
        // SAFETY: `Hpet::new()` established that the base is the mapped HPET
        // window, and the callers pass the register constants above.
        unsafe {
            core::ptr::read_volatile(
                self.base.as_ptr().add(offset).cast::<u32>(),
            )
        }
    }

    /// Write `value` to the 32-bit register at byte `offset`.
    fn write(&self, offset: usize, value: u32) {
        // SAFETY: as `read()`; the volatile store is the C's `*(volatile
        // uint32_t *) =` through `HPET32()`.
        unsafe {
            core::ptr::write_volatile(
                self.base.as_ptr().add(offset).cast::<u32>(),
                value,
            )
        }
    }
}

/// `cpuid()` of <i386/proc_reg.h> with `eax = 1` and `ecx = 0`; EBX's high
/// byte is the APIC ID.
fn cpuid_leaf1() -> u32 {
    let ebx: u32;
    // SAFETY: `cpuid` is the CPU's own instruction and leaf 1 is defined by
    // the architecture.
    #[cfg(target_pointer_width = "64")]
    unsafe {
        asm!(
            "mov {scratch:r}, rbx",
            "cpuid",
            "xchg {scratch:r}, rbx",
            scratch = out(reg) ebx,
            inout("eax") 1u32 => _,
            inout("ecx") 0u32 => _,
            out("edx") _,
            options(nostack, preserves_flags),
        );
    }
    // SAFETY: as above, with the 32-bit macro's register save.
    #[cfg(target_pointer_width = "32")]
    unsafe {
        asm!(
            "mov {scratch:e}, ebx",
            "cpuid",
            "xchg {scratch:e}, ebx",
            scratch = out(reg) ebx,
            inout("eax") 1u32 => _,
            inout("ecx") 0u32 => _,
            out("edx") _,
            options(nostack, preserves_flags),
        );
    }
    ebx
}

/// Publish the mapped local-APIC page.
fn lapic_init(lapic_ptr: *mut c_void) {
    // SAFETY: `lapic` is the C global <i386/apic.h> declares.
    unsafe { glue::lapic = lapic_ptr };
}

/// The kernel ID the lookup table records for `apic_id`.
fn kernel_id(apic_id: u16) -> c_int {
    let Ok(index) = u8::try_from(apic_id) else {
        return 0;
    };
    // SAFETY: `cpu_id_lut` is the 256-entry table the C side defines, and
    // `index` came from a `u8`, so the read stays inside it.
    unsafe {
        core::ptr::read(
            core::ptr::addr_of!(glue::cpu_id_lut)
                .cast::<c_int>()
                .add(usize::from(index)),
        )
    }
}

/// The mapped local-APIC page.
fn lapic_ptr() -> *mut c_void {
    // SAFETY: `lapic` is the C global; the load only reads the pointer.
    unsafe { glue::lapic }
}

/// The APIC ID of the running CPU, masked to the bits the platform implements.
fn current_cpu() -> c_int {
    // The APIC ID is EBX's high byte, and the shift leaves eight bits, so the
    // narrowing cast loses nothing.
    let apic_id = (cpuid_leaf1() >> 24) as u8;
    // SAFETY: `apic_id_mask` is the C global `fix_apic_id_mask()` sets once at
    // boot, before CPUs other than the master run it.
    let mask = unsafe { glue::apic_id_mask };
    c_int::from(apic_id & mask)
}

/// Program the HPET for 32-bit periodic counting with interrupts off.
fn hpet_setup() {
    let Some(hpet) = Hpet::new() else {
        // SAFETY: `printf` is the real C routine <kern/printf.h> declares, and
        // this format has no conversion specifier.
        unsafe { glue::printf(c"HPET not available\n".as_ptr()) };
        return;
    };

    let period = hpet.read(HPET_CAP_PERIOD);
    let period_nsec = period / FSEC_PER_NSEC;
    // SAFETY: `hpet_period_nsec` is the C global; `hpet_init()` runs once, at
    // boot, and is its only writer.
    unsafe { glue::hpet_period_nsec = period_nsec };
    // SAFETY: as the message above; the one vararg matches `%d`.
    unsafe {
        glue::printf(
            c"HPET ticks every %d nanoseconds\n".as_ptr(),
            period_nsec as c_int,
        )
    };

    let val = hpet.read(HPET_CFG) & !(HPET_LEGACY_ROUTE | HPET_CFG_ENABLE);
    hpet.write(HPET_CFG, val);

    hpet.write(HPET_COUNTER, 0);

    let val = (hpet.read(HPET_T0_CFG) & !HPET_T0_INT_ENABLE)
        | HPET_T0_32BIT_MODE
        | HPET_T0_TYPE_PERIODIC
        | HPET_T0_VAL_SET;
    hpet.write(HPET_T0_CFG, val);

    hpet.write(HPET_T0_COMPARATOR, u32::MAX);

    let val = hpet.read(HPET_CFG) | HPET_CFG_ENABLE;
    hpet.write(HPET_CFG, val);

    // SAFETY: as the first message.
    unsafe { glue::printf(c"HPET enabled\n".as_ptr()) };
}

/// Busy-wait for `usec` microseconds on the HPET counter.
///
/// # Panics
///
/// Panics when `hpet_period_nsec` is zero, which is only possible when no HPET
/// was initialized; the C divided by the same zero and faulted.
fn delay_us(mut usec: u32) {
    if usec > MAX_DELAY_USEC {
        // SAFETY: `printf` is the real C routine; the three `%d` specifiers
        // consume the three varargs, each of which fits `c_int` (the clamp and
        // the clamp limit are both below `c_int::MAX`).
        unsafe {
            glue::printf(
                c"HPET ERROR: Delay too long, %d usec, truncating to %d usec\n"
                    .as_ptr(),
                usec as c_int,
                MAX_DELAY_USEC as c_int,
            )
        };
        usec = MAX_DELAY_USEC;
    }

    // SAFETY: `hpet_period_nsec` is the C global; `hpet_init()` is its only
    // writer, and a 32-bit load is atomic on both targets.
    let period_nsec = unsafe { glue::hpet_period_nsec };
    usec = (usec * NSEC_PER_USEC) / period_nsec;

    let Some(hpet) = Hpet::new() else {
        return;
    };
    let start = hpet.read(HPET_COUNTER);
    loop {
        let now = hpet.read(HPET_COUNTER);
        if now.wrapping_sub(start) >= usec {
            break;
        }
    }
}

/// Busy-wait for `ms` milliseconds on the HPET counter.
fn delay_ms(ms: u32) {
    delay_us(ms.wrapping_mul(1000));
}

/// Read the HPET main counter, or zero when ACPI found no timer.
fn read_counter() -> u32 {
    match Hpet::new() {
        Some(hpet) => hpet.read(HPET_COUNTER),
        None => 0,
    }
}

/// The HPET tick period in nanoseconds.
fn counter_period_nsec() -> u32 {
    // SAFETY: `hpet_period_nsec` is the C global; `hpet_init()` is its only
    // writer, and a 32-bit load is atomic on both targets.
    unsafe { glue::hpet_period_nsec }
}

/// Publish the mapped local-APIC page.
///
/// # Safety
///
/// `lapic_ptr` must be the mapped local-APIC page, mapped for read and write
/// for the rest of the kernel's life.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apic_lapic_init(lapic_ptr: *mut c_void) {
    lapic_init(lapic_ptr);
}

/// The kernel ID recorded for an APIC ID.
#[unsafe(no_mangle)]
pub extern "C" fn apic_get_cpu_kernel_id(apic_id: u16) -> c_int {
    kernel_id(apic_id)
}

/// The mapped local-APIC page.
#[unsafe(no_mangle)]
pub extern "C" fn apic_get_lapic() -> *mut c_void {
    lapic_ptr()
}

/// The APIC ID of the running CPU.
#[unsafe(no_mangle)]
pub extern "C" fn apic_get_current_cpu() -> c_int {
    current_cpu()
}

/// Initialize the HPET.
#[unsafe(no_mangle)]
pub extern "C" fn hpet_init() {
    hpet_setup();
}

/// Busy-wait for `us` microseconds on the HPET counter.
///
/// # Panics
///
/// As [`delay_us()`]: a zero `hpet_period_nsec` is a division by zero.
#[unsafe(no_mangle)]
pub extern "C" fn hpet_udelay(us: u32) {
    delay_us(us);
}

/// Busy-wait for `ms` milliseconds on the HPET counter.
///
/// # Panics
///
/// As [`delay_ms()`]: a zero `hpet_period_nsec` is a division by zero.
#[unsafe(no_mangle)]
pub extern "C" fn hpet_mdelay(ms: u32) {
    delay_ms(ms);
}

/// Read the HPET main counter, or zero when there is no HPET.
#[unsafe(no_mangle)]
pub extern "C" fn hpclock_read_counter() -> u32 {
    read_counter()
}

/// The HPET tick period in nanoseconds.
#[unsafe(no_mangle)]
pub extern "C" fn hpclock_get_counter_period_nsec() -> u32 {
    counter_period_nsec()
}
