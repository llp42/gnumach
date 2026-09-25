// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/debug_i386.c and i386/i386/debug.h:
//   Copyright (c) 1994 The University of Utah and the Computer Systems
//   Laboratory at the University of Utah (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The saved-state dump and the debug trace, which `i386/i386/debug_i386.c`
//! used to define and `i386/i386/debug.h` declares.
//!
//! The C kept the trace under `#ifdef DEBUG`, which no configured kernel
//! defines; the Rust build keeps it compiled so a `-DDEBUG` assembly build
//! still finds the globals `debug_trace.S` writes.
//!
//! The `extern "C"` edge is in [`debug_i386_ffi`].

use crate::arch::i386::pcb::I386SavedState;
use crate::arch::i386::percpu;
use crate::arch::i386::trap;
use crate::glue;
use crate::kern::task::{Task, current_task};
use core::ffi::{VaList, c_char, c_int, c_long, c_uint, c_ulong};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;

/// `DEBUG_TRACE_LEN` of <i386/debug.h>: the entries after which the trace
/// buffer wraps.
const DEBUG_TRACE_LEN: usize = 512;

/// `struct debug_trace_entry` of `i386/i386/debug_i386.c`, written by
/// `debug_trace.S` at an eight- or sixteen-byte stride.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugTraceEntry {
    pub filename: *mut c_char,
    pub linenum: c_int,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<DebugTraceEntry>() == 8);
    assert!(align_of::<DebugTraceEntry>() == align_of::<u32>());
    assert!(offset_of!(DebugTraceEntry, filename) == 0);
    assert!(offset_of!(DebugTraceEntry, linenum) == 4);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<DebugTraceEntry>() == 16);
    assert!(align_of::<DebugTraceEntry>() == align_of::<*mut c_char>());
    assert!(offset_of!(DebugTraceEntry, filename) == 0);
    assert!(offset_of!(DebugTraceEntry, linenum) == 8);
};

/// `struct mach_trap_t` of <kern/syscall_sw.h>, the syscall table entries
/// `syscall_trace_print` reads.
#[repr(C)]
pub struct MachTrap {
    pub mach_trap_arg_count: c_int,
    pub mach_trap_function: Option<unsafe extern "C" fn()>,
    pub mach_trap_stack: c_int,
    pub mach_trap_name: *const c_char,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<MachTrap>() == 16);
    assert!(align_of::<MachTrap>() == align_of::<u32>());
    assert!(offset_of!(MachTrap, mach_trap_arg_count) == 0);
    assert!(offset_of!(MachTrap, mach_trap_function) == 4);
    assert!(offset_of!(MachTrap, mach_trap_stack) == 8);
    assert!(offset_of!(MachTrap, mach_trap_name) == 12);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<MachTrap>() == 32);
    assert!(align_of::<MachTrap>() == align_of::<u64>());
    assert!(offset_of!(MachTrap, mach_trap_arg_count) == 0);
    assert!(offset_of!(MachTrap, mach_trap_function) == 8);
    assert!(offset_of!(MachTrap, mach_trap_stack) == 16);
    assert!(offset_of!(MachTrap, mach_trap_name) == 24);
};

/// `debug_trace_buf` of `i386/i386/debug_i386.c`, which `debug_trace.S`
/// writes when a debug build calls `DEBUG_TRACE`.
#[unsafe(no_mangle)]
pub static mut debug_trace_buf: [DebugTraceEntry; DEBUG_TRACE_LEN] =
    [DebugTraceEntry {
        filename: ptr::null_mut(),
        linenum: 0,
    }; DEBUG_TRACE_LEN];

/// `debug_trace_pos` of `i386/i386/debug_i386.c`.
#[unsafe(no_mangle)]
pub static mut debug_trace_pos: c_int = 0;

/// `syscall_trace` of `i386/i386/debug_i386.c`.
#[unsafe(no_mangle)]
pub static mut syscall_trace: c_int = 0;

/// `syscall_trace_task` of `i386/i386/debug_i386.c`.
#[unsafe(no_mangle)]
pub static mut syscall_trace_task: *mut Task = ptr::null_mut();

/// `dump_ss()` of <i386/debug.h>.
///
/// # Safety
///
/// `st` must point at a live `I386SavedState`.
pub(crate) unsafe fn dump_ss(st: *const I386SavedState) {
    // SAFETY: the caller promises `st` is live.
    let st = unsafe { &*st };
    // SAFETY: `%p` takes the pointer, and the literals name the C's format.
    unsafe { glue::printf(c"Dump of i386_saved_state %p:\n".as_ptr(), st) };

    #[cfg(target_pointer_width = "64")]
    // SAFETY: each `%016lx` takes the matching `c_ulong`.
    unsafe {
        glue::printf(
            c"RAX %016lx RBX %016lx RCX %016lx RDX %016lx\n".as_ptr(),
            st.eax,
            st.ebx,
            st.ecx,
            st.edx,
        );
        glue::printf(
            c"RSI %016lx RDI %016lx RBP %016lx RSP %016lx\n".as_ptr(),
            st.esi,
            st.edi,
            st.ebp,
            st.uesp,
        );
        glue::printf(
            c"R8  %016lx R9  %016lx R10 %016lx R11 %016lx\n".as_ptr(),
            st.r8,
            st.r9,
            st.r10,
            st.r11,
        );
        glue::printf(
            c"R12 %016lx R13 %016lx R14 %016lx R15 %016lx\n".as_ptr(),
            st.r12,
            st.r13,
            st.r14,
            st.r15,
        );
        glue::printf(c"RIP %016lx EFLAGS %08lx\n".as_ptr(), st.eip, st.efl);
    }

    #[cfg(target_pointer_width = "32")]
    // SAFETY: each `%08lx` takes the matching `c_ulong`.
    unsafe {
        glue::printf(
            c"EAX %08lx EBX %08lx ECX %08lx EDX %08lx\n".as_ptr(),
            st.eax,
            st.ebx,
            st.ecx,
            st.edx,
        );
        glue::printf(
            c"ESI %08lx EDI %08lx EBP %08lx ESP %08lx\n".as_ptr(),
            st.esi,
            st.edi,
            st.ebp,
            st.uesp,
        );
        glue::printf(
            c"CS %04lx SS %04lx DS %04lx ES %04lx FS %04lx GS %04lx\n"
                .as_ptr(),
            st.cs & 0xffff,
            st.ss & 0xffff,
            st.ds & 0xffff,
            st.es & 0xffff,
            st.fs & 0xffff,
            st.gs & 0xffff,
        );
        glue::printf(
            c"v86:            DS %04lx ES %04lx FS %04lx GS %04lx\n".as_ptr(),
            st.v86_segs.v86_ds & 0xffff,
            st.v86_segs.v86_es & 0xffff,
            st.v86_segs.v86_gs & 0xffff,
            st.v86_segs.v86_gs & 0xffff,
        );
        glue::printf(c"EIP %08lx EFLAGS %08lx\n".as_ptr(), st.eip, st.efl);
    }

    // SAFETY: the `%ld`, `%s` and `%08lx` take the `c_long`, the
    // NUL-terminated name and the `c_ulong`, and `trap_name` returns a
    // `'static` C string.
    unsafe {
        glue::printf(
            c"trapno %ld: %s, error %08lx\n".as_ptr(),
            st.trapno as c_long,
            trap::trap_name(st.trapno as c_uint).as_ptr(),
            st.err,
        )
    };
}

/// `debug_trace_reset()` of <i386/debug.h>.
pub(crate) fn debug_trace_reset() {
    // SAFETY: `splhigh()` is the real asm function <i386/spl.h> declares.
    let s = unsafe { glue::splhigh() };
    // SAFETY: `debug_trace_pos` and the buffer have no other writer on this
    // CPU while interrupts are off.
    unsafe {
        debug_trace_pos = 0;
        debug_trace_buf[DEBUG_TRACE_LEN - 1].filename = ptr::null_mut();
    }
    // SAFETY: `s` is the level `splhigh()` returned.
    unsafe { glue::splx(s) };
}

/// `print_entry()` in `i386/i386/debug_i386.c`.
fn print_entry(i: usize, col: &mut c_int) {
    // SAFETY: `i` is below `DEBUG_TRACE_LEN` at every call site.
    let mut filename = unsafe { debug_trace_buf[i].filename };
    let mut p = filename;
    loop {
        // SAFETY: the entry names the NUL-terminated file of a `DEBUG_TRACE`
        // call site, and the walk stops at its NUL.
        let byte = unsafe { *p };
        if byte == 0 {
            break;
        }
        if byte == b'/' as c_char {
            // SAFETY: `p` points inside that same string.
            filename = unsafe { p.add(1) };
        }
        // SAFETY: as above.
        p = unsafe { p.add(1) };
    }

    // SAFETY: `%9s` takes the pointer and `%-4d` the `c_int`; `i` is in
    // bounds.
    unsafe {
        glue::printf(
            c" %9s:%-4d".as_ptr(),
            filename,
            debug_trace_buf[i].linenum,
        )
    };
    *col += 1;
    if *col == 5 {
        // SAFETY: the message holds no conversion specifier.
        unsafe { glue::printf(c"\n".as_ptr()) };
        *col = 0;
    }
}

/// `debug_trace_dump()` of <i386/debug.h>.
pub(crate) fn debug_trace_dump() {
    // SAFETY: `splhigh()` is the real asm function <i386/spl.h> declares.
    let s = unsafe { glue::splhigh() };
    let mut col: c_int = 0;

    // SAFETY: the message holds no conversion specifier.
    unsafe { glue::printf(c"Debug trace dump ".as_ptr()) };

    // If the wrapper left the last entry's name non-null, the buffer is full:
    // print from the current position around to just before it, then the rest.
    // SAFETY: the buffer has no other writer while interrupts are off.
    let last = unsafe { debug_trace_buf[DEBUG_TRACE_LEN - 1].filename };
    // SAFETY: as above.
    let pos = unsafe { debug_trace_pos };
    if !last.is_null() {
        // SAFETY: the message holds no conversion specifier.
        unsafe { glue::printf(c"(full):\n".as_ptr()) };

        for i in (pos as usize)..DEBUG_TRACE_LEN {
            print_entry(i, &mut col);
        }
    } else {
        // SAFETY: the `%d` takes the matching `c_int`.
        unsafe { glue::printf(c"(%d entries):\n".as_ptr(), pos) };
    }

    for i in 0..(pos as usize) {
        print_entry(i, &mut col);
    }

    if col != 0 {
        // SAFETY: the message holds no conversion specifier.
        unsafe { glue::printf(c"\n".as_ptr()) };
    }

    debug_trace_reset();

    // SAFETY: `s` is the level `splhigh()` returned.
    unsafe { glue::splx(s) };
}

/// `syscall_trace_print()` of `i386/i386/debug_i386.c`, the variadic body the
/// `extern "C"` edge in [`debug_i386_ffi`] hands its argument list to.
pub(crate) fn trace_print(syscallvec: c_int, args: &mut VaList<'_>) -> c_int {
    let syscallnum = (syscallvec >> 4) as usize;
    // SAFETY: `mach_trap_table` has an entry per syscall number, and the
    // caller passed one the syscall path extracted from `mach_trap_table`.
    let trap =
        unsafe { &*ptr::addr_of!(glue::mach_trap_table).add(syscallnum) };
    // SAFETY: the syscall path calls this with a live current thread.
    let task = unsafe { current_task() };
    // SAFETY: `syscall_trace_task` is the trace filter, written only by the
    // debugger.
    let filter = unsafe { syscall_trace_task };
    if !filter.is_null() && filter != task {
        return syscallvec;
    }

    // SAFETY: `%08x` takes the low `c_uint` of each pointer and `%s` the
    // table's NUL-terminated name, as the C passed them.
    unsafe {
        glue::printf(
            c"0x%08x:0x%08x:%s(".as_ptr(),
            task.expose_provenance() as c_uint,
            percpu::current_thread().expose_provenance() as c_uint,
            trap.mach_trap_name,
        )
    };

    let count = trap.mach_trap_arg_count;
    for i in 0..count {
        // SAFETY: the caller's `va_arg` order matches the syscall's argument
        // count, which the table holds.
        let value: c_ulong = unsafe { args.next_arg() };
        // SAFETY: `%x` takes the low `c_uint` and `%d` the `c_int`, as the C
        // passed the `unsigned long`.
        unsafe {
            if value > 1024 {
                glue::printf(c"0x%08x".as_ptr(), value as c_uint);
            } else {
                glue::printf(c"%d".as_ptr(), value as c_int);
            }
        }

        if i + 1 < count {
            // SAFETY: the message holds no conversion specifier.
            unsafe { glue::printf(c", ".as_ptr()) };
        }
    }
    // SAFETY: the message holds no conversion specifier.
    unsafe { glue::printf(c")\n".as_ptr()) };

    syscallvec
}
