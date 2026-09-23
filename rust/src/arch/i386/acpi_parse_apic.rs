// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/acpi_parse_apic.c:
//   Copyright (C) 2018 Juan Bosco Garcia
//   Copyright (C) 2019 2020 Almudena Garcia Jurado-Centurion
//   Written by Juan Bosco Garcia and Almudena Garcia Jurado-Centurion
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The ACPI table-address report, which `i386/i386at/acpi_parse_apic.c`
//! used to define and `i386/i386at/acpi_parse_apic.h` declares.
//!
//! The rest of `acpi_parse_apic.c` stays C: it is the MADT parser
//! itself, with the ACPI table structs and the APIC and IOAPIC lists.

use crate::arch::types::VmOffset;
use crate::glue;
use core::ffi::{c_int, c_void};

/// Report the physical RSDP address, the mapped RSDT or XSDT address
/// and the number of entries the table holds.  The body of
/// `acpi_print_info()` in `i386/i386at/acpi_parse_apic.c`.
fn print_info(rsdp: VmOffset, rsdt: *mut c_void, acpi_rsdt_n: c_int) {
    // SAFETY: `printf` is the real C routine <kern/printf.h> declares,
    // and this format has no conversion specifier.
    unsafe { glue::printf(c"ACPI:\n".as_ptr()) };
    // SAFETY: as above; the one conversion `%llx` takes the C's own
    // widening of `rsdp` to `unsigned long long`.  `phys_addr_t` is 32
    // bits on i386 and 64 on x86_64, so the cast widens on one target
    // and is the identity on the other, and loses nothing on either.
    unsafe { glue::printf(c" rsdp = 0x%llx\n".as_ptr(), rsdp as u64) };
    // SAFETY: as above; `%p` prints the pointer without following it,
    // and `%d` takes the entry count, one `c_int` for one vararg.
    unsafe {
        glue::printf(
            c" rsdt/xsdt = 0x%p (n = %d)\n".as_ptr(),
            rsdt,
            acpi_rsdt_n,
        )
    };
}

/// Report the ACPI tables the MADT parse found, at boot.  The
/// `acpi_print_info()` entry of <i386at/acpi_parse_apic.h>, which
/// `i386/i386at/acpi_parse_apic.c` used to define.
///
/// # Safety
///
/// The C prototype takes an unchecked `void *`, so the marker records
/// the C entry point: `rsdt` is only printed, never followed, and the
/// function reads no memory the caller owns.  The caller must be the
/// boot path, which has the console and `printf` running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn acpi_print_info(
    rsdp: VmOffset,
    rsdt: *mut c_void,
    acpi_rsdt_n: c_int,
) {
    print_info(rsdp, rsdt, acpi_rsdt_n);
}
