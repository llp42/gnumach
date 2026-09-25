// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! A Rust panic, reported through the kernel's panic path.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let (file, line) = match info.location() {
        Some(loc) => (loc.file(), loc.line()),
        None => ("<unknown>", 0),
    };

    crate::kern::debug::panic_fmt(
        file,
        line,
        "mach_rs",
        format_args!("{}", info.message()),
    )
}
