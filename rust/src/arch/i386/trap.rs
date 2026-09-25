// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/trap.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University
// Derived from i386/i386/trap.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University
// Derived from i386/include/mach/i386/trap.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The hardware trap and fault handlers, which `i386/i386/trap.c` used to
//! define and `i386/i386/trap.h` declares.

use crate::arch::i386::fpu;
use crate::arch::i386::pcb::I386SavedState;
use crate::arch::i386::percpu;
use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::ast;
use crate::kern::exception as exception_core;
use crate::kern::thread::Thread;
use crate::vm::error::KERN_SUCCESS;
use crate::vm::types::VmProt;
use crate::vm::vm_kern;
use crate::vm::vm_map::{VmMap, trunc_page};
use core::arch::asm;
use core::ffi::{CStr, c_int, c_long, c_uint, c_ulong};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;

/// `T_DIVIDE_ERROR` of <mach/i386/trap.h>.
const T_DIVIDE_ERROR: c_ulong = 0;
/// `T_DEBUG` of <mach/i386/trap.h>.
const T_DEBUG: c_ulong = 1;
/// `T_INT3` of <mach/i386/trap.h>.
const T_INT3: c_ulong = 3;
/// `T_OVERFLOW` of <mach/i386/trap.h>.
const T_OVERFLOW: c_ulong = 4;
/// `T_OUT_OF_BOUNDS` of <mach/i386/trap.h>.
const T_OUT_OF_BOUNDS: c_ulong = 5;
/// `T_INVALID_OPCODE` of <mach/i386/trap.h>.
const T_INVALID_OPCODE: c_ulong = 6;
/// `T_NO_FPU` of <mach/i386/trap.h>.
const T_NO_FPU: c_ulong = 7;
/// `T_FPU_FAULT` of <mach/i386/trap.h>.
const T_FPU_FAULT: c_ulong = 9;
/// `T_SEGMENT_NOT_PRESENT` of <mach/i386/trap.h>.
const T_SEGMENT_NOT_PRESENT: c_ulong = 11;
/// `T_STACK_FAULT` of <mach/i386/trap.h>.
const T_STACK_FAULT: c_ulong = 12;
/// `T_GENERAL_PROTECTION` of <mach/i386/trap.h>.
const T_GENERAL_PROTECTION: c_ulong = 13;
/// `T_PAGE_FAULT` of <mach/i386/trap.h>.
const T_PAGE_FAULT: c_ulong = 14;
/// `T_FLOATING_POINT_ERROR` of <mach/i386/trap.h>.
const T_FLOATING_POINT_ERROR: c_ulong = 16;
/// `T_PF_WRITE` of <mach/i386/trap.h>: the page fault was a write.
const T_PF_WRITE: c_ulong = 0x2;

/// `EXC_BAD_ACCESS` of <mach/exception.h>.
const EXC_BAD_ACCESS: c_int = 1;
/// `EXC_BAD_INSTRUCTION` of <mach/exception.h>.
const EXC_BAD_INSTRUCTION: c_int = 2;
/// `EXC_ARITHMETIC` of <mach/exception.h>.
const EXC_ARITHMETIC: c_int = 3;
/// `EXC_SOFTWARE` of <mach/exception.h>.
const EXC_SOFTWARE: c_int = 5;
/// `EXC_BREAKPOINT` of <mach/exception.h>.
const EXC_BREAKPOINT: c_int = 6;

/// `EXC_I386_DIV` of <mach/i386/exception.h>.
const EXC_I386_DIV: c_int = 1;
/// `EXC_I386_SGL` of <mach/i386/exception.h>.
const EXC_I386_SGL: c_int = 1;
/// `EXC_I386_BPT` of <mach/i386/exception.h>.
const EXC_I386_BPT: c_int = 2;
/// `EXC_I386_INTO` of <mach/i386/exception.h>.
const EXC_I386_INTO: c_int = 2;
/// `EXC_I386_BOUND` of <mach/i386/exception.h>.
const EXC_I386_BOUND: c_int = 7;
/// `EXC_I386_INVOP` of <mach/i386/exception.h>.
const EXC_I386_INVOP: c_int = 1;
/// `EXC_I386_INVTSSFLT` of <mach/i386/exception.h>.
const EXC_I386_INVTSSFLT: c_int = 10;
/// `EXC_I386_SEGNPFLT` of <mach/i386/exception.h>.
const EXC_I386_SEGNPFLT: c_int = 11;
/// `EXC_I386_STKFLT` of <mach/i386/exception.h>.
const EXC_I386_STKFLT: c_int = 12;
/// `EXC_I386_GPFLT` of <mach/i386/exception.h>.
const EXC_I386_GPFLT: c_int = 13;
/// `EXC_I386_PGFLT` of <mach/i386/exception.h>.
const EXC_I386_PGFLT: c_int = 14;

/// `AST_I386_FP` of <i386/ast.h>: a delayed floating-point exception.
const AST_I386_FP: usize = 0x8000_0000;

/// `LINEAR_MIN_KERNEL_ADDRESS` of <i386/vm_param.h>, where the kernel's
/// linear range begins; both bases coincide in the configured builds.
const LINEAR_MIN_KERNEL_ADDRESS: VmOffset = vm_kern::VM_MIN_KERNEL_ADDRESS;

/// `struct recovery` of <i386/locore.h>: one fault-address/recovery-address
/// pair of the `copyin`/`copyout` recovery tables.  The C fields are
/// `vm_offset_t`, the same width as the trap frame's instruction pointer in
/// both configured builds.
#[repr(C)]
pub struct Recovery {
    fault_addr: c_ulong,
    recover_addr: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Recovery>() == 16);
    assert!(align_of::<Recovery>() == 8);
    assert!(offset_of!(Recovery, fault_addr) == 0);
    assert!(offset_of!(Recovery, recover_addr) == 8);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Recovery>() == 8);
    assert!(align_of::<Recovery>() == 4);
    assert!(offset_of!(Recovery, fault_addr) == 0);
    assert!(offset_of!(Recovery, recover_addr) == 4);
};

/// `trap_type[]` of `trap.c`, in trap-number order.
static TRAP_TYPE: [&CStr; 17] = [
    c"Divide error",
    c"Debug trap",
    c"NMI",
    c"Breakpoint",
    c"Overflow",
    c"Bounds check",
    c"Invalid opcode",
    c"No coprocessor",
    c"Double fault",
    c"Coprocessor overrun",
    c"Invalid TSS",
    c"Segment not present",
    c"Stack bounds",
    c"General protection",
    c"Page fault",
    c"(reserved)",
    c"Coprocessor error",
];

/// The `trap_type[]` lookup `kernel_trap()` and `user_trap()` share.
fn trap_type(trapnum: c_ulong) -> Option<&'static CStr> {
    let index = usize::try_from(trapnum).ok()?;
    TRAP_TYPE.get(index).copied()
}

/// `trap_name()` of i386/i386/trap.h.
pub(crate) fn trap_name(trapnum: c_uint) -> &'static CStr {
    trap_type(trapnum as c_ulong).unwrap_or(c"(unknown)")
}

/// `lintokv()` of <i386/vm_param.h>.
const fn lintokv(lin: VmOffset) -> VmOffset {
    lin.wrapping_sub(LINEAR_MIN_KERNEL_ADDRESS)
        .wrapping_add(vm_kern::VM_MIN_KERNEL_ADDRESS)
}

/// The linear scan of a recovery table in `kernel_trap()`: on a match, jump
/// `regs.eip` to the recovery address.
fn retry(
    regs: &mut I386SavedState,
    mut entry: *const Recovery,
    end: *const Recovery,
) -> bool {
    while entry < end {
        // SAFETY: the tables are `Recovery` arrays ending at `end`.
        let recovery = unsafe { &*entry };
        if regs.eip == recovery.fault_addr {
            regs.eip = recovery.recover_addr;
            return true;
        }
        entry = entry.wrapping_add(1);
    }
    false
}

/// `inst_fetch()` of <i386/locore.h> at `eip + offset`, narrowed to the
/// opcode byte the callers compare.
fn fetch_byte(eip: c_ulong, cs: c_ulong, offset: c_ulong) -> u8 {
    // The C passed the frame's `unsigned long`s through the `int` parameters.
    // SAFETY: `inst_fetch()` is the assembly routine whose own recovery table
    // handles an unreadable address, and both values come from the live trap
    // frame.
    let fetched = unsafe {
        glue::inst_fetch(eip.wrapping_add(offset) as c_int, cs as c_int)
    };
    (fetched & 0xff) as u8
}

/// `get_dr6()` of <i386/proc_reg.h>.
fn get_dr6() -> c_ulong {
    let value: c_ulong;
    // SAFETY: `mov r, dr6` reads the debug status register and touches no
    // memory; the stack stays balanced.
    unsafe {
        asm!(
            "mov {value}, dr6",
            value = out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// `set_dr6()` of <i386/proc_reg.h>.
fn set_dr6(value: c_ulong) {
    // SAFETY: `mov dr6, r` writes the debug status register and touches no
    // memory; the stack stays balanced.
    unsafe {
        asm!(
            "mov dr6, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

/// `i386_exception()` of i386/i386/trap.h.
///
/// # Safety
///
/// Called on the trap or FPU path with nothing locked.
pub(crate) unsafe fn i386_exception(
    exc: c_int,
    code: c_int,
    subcode: c_long,
) -> ! {
    // SAFETY: `splsched()` and `splx()` are the real asm routines.
    let s = unsafe { glue::splsched() };
    ast::ast_off(percpu::cpu_number(), AST_I386_FP);
    let _ = unsafe { glue::splx(s) };

    // SAFETY: the caller's contract, and `exception()` does not return.
    unsafe { exception_core::exception(exc, code, subcode) }
}

/// `i386_astintr()` of i386/i386/trap.h.
///
/// # Safety
///
/// Called from the interrupt path with the CPU's AST set by an IPI.
pub(crate) unsafe fn astintr() {
    // SAFETY: `splsched()` is the real asm routine.
    let _ = unsafe { glue::splsched() };
    let mycpu = percpu::cpu_number();

    if ast::ast_needed(mycpu) & AST_I386_FP != 0 {
        ast::ast_off(mycpu, AST_I386_FP);
        // SAFETY: `spl0()` is the real asm routine.
        let _ = unsafe { glue::spl0() };
        // SAFETY: the FPU path runs on the trap stack with nothing locked.
        unsafe { fpu::fpastintr() };
    } else {
        // SAFETY: `ast_taken()` of `kern/ast.c` runs on this CPU's trap
        // stack.
        unsafe { crate::kern::ast::taken() };
    }
}

/// The `badtrap` tail of `kernel_trap()`: report the unhandled trap and
/// halt.
fn bad_trap(regs: &I386SavedState, type_: c_ulong, code: c_ulong) -> ! {
    // SAFETY: `printf` accepts the C format and arguments.
    unsafe { glue::printf(c"Kernel ".as_ptr()) };
    match trap_type(type_) {
        // SAFETY: `printf` accepts the C format and arguments.
        Some(name) => unsafe {
            glue::printf(c"%s trap".as_ptr(), name.as_ptr())
        },
        // SAFETY: as above.
        None => unsafe { glue::printf(c"trap %ld".as_ptr(), type_) },
    };
    // SAFETY: as above.
    unsafe {
        glue::printf(
            c", eip 0x%lx, code %lx, cr2 %lx\n".as_ptr(),
            regs.eip,
            code,
            regs.cr2,
        )
    };
    // SAFETY: `splhigh()` is the real asm routine.
    let _ = unsafe { glue::splhigh() };
    // SAFETY: as above.
    unsafe {
        glue::printf(
            c"kernel trap, type %ld, code = %lx\n".as_ptr(),
            type_,
            code,
        )
    };
    // SAFETY: `regs` is the live trap frame.
    unsafe { crate::arch::i386::debug_i386::dump_ss(regs) };
    // SAFETY: `Panic()` does not return.
    unsafe {
        glue::Panic(
            c"i386/i386/trap.c".as_ptr(),
            line!() as c_int,
            c"kernel_trap".as_ptr(),
            c"trap".as_ptr(),
        )
    }
}

/// The `T_GENERAL_PROTECTION` recovery path of `kernel_trap()`, shared with
/// the page-fault fall-through.
fn general_protection(
    regs: &mut I386SavedState,
    thread: *mut Thread,
    type_: c_ulong,
    code: c_ulong,
) {
    if retry(
        regs,
        ptr::addr_of!(glue::recover_table),
        ptr::addr_of!(glue::recover_table_end),
    ) {
        return;
    }

    // SAFETY: the trap path runs on a live thread.
    if unsafe { (*thread).recover } != 0 {
        // SAFETY: the thread is live, and its recovery address is set only by
        // the `copyin`/`copyout` paths this trap resumes through.
        unsafe {
            regs.eip = (*thread).recover as c_ulong;
            (*thread).recover = 0;
        }
        return;
    }

    bad_trap(regs, type_, code);
}

/// The kernel-mode `T_PAGE_FAULT` handling of `kernel_trap()`.
fn page_fault(
    regs: &mut I386SavedState,
    thread: *mut Thread,
    type_: c_ulong,
    code: c_ulong,
) {
    // The C's `vm_offset_t` and the frame's `unsigned long` are the same
    // width in both configured builds, so the fault address is the register
    // value.
    let mut subcode = regs.cr2 as VmOffset;

    let map = if lintokv(subcode) == 0 || subcode >= LINEAR_MIN_KERNEL_ADDRESS
    {
        // SAFETY: `kernel_map` is the C global the boot path sets up.
        let map = unsafe { glue::kernel_map }.cast::<VmMap>();
        subcode = lintokv(subcode);

        let image_start = ptr::addr_of!(glue::_start).addr();
        let image_end = ptr::addr_of!(glue::etext).addr();
        if trunc_page(subcode) == 0
            || (image_start <= subcode && subcode < image_end)
        {
            // SAFETY: `printf` accepts the C format and arguments.
            unsafe {
                glue::printf(
                    c"Kernel page fault at address 0x%lx, eip = 0x%lx\n"
                        .as_ptr(),
                    subcode,
                    regs.eip,
                )
            };
            bad_trap(regs, type_, code);
        }
        map
    } else {
        let mut map = ptr::null_mut::<VmMap>();
        if !thread.is_null() {
            // SAFETY: the caller promises a live thread, whose task and map
            // are live.
            map = unsafe { (*(*thread).task).map }.cast::<VmMap>();
        }
        if thread.is_null()
            || map == unsafe { glue::kernel_map }.cast::<VmMap>()
        {
            // SAFETY: `printf` accepts the C format and arguments.
            unsafe {
                glue::printf(
                    c"kernel page fault at %08lx:\n".as_ptr(),
                    subcode,
                )
            };
            // SAFETY: `regs` is the live trap frame.
            unsafe { crate::arch::i386::debug_i386::dump_ss(regs) };
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"i386/i386/trap.c".as_ptr(),
                    line!() as c_int,
                    c"kernel_trap".as_ptr(),
                    c"kernel thread accessed user space!\n".as_ptr(),
                )
            };
        }
        map
    };

    let protection = if code & T_PF_WRITE != 0 {
        VmProt::READ | VmProt::WRITE
    } else {
        VmProt::READ
    };

    // SAFETY: `map` is live, either the kernel map or the faulting thread's,
    // and `vm_fault()` handles the fault at the page `trunc_page()` names.
    let result = unsafe {
        glue::vm_fault(map, trunc_page(subcode), protection, 0, 0, None)
    };

    if result == KERN_SUCCESS {
        let _ = retry(
            regs,
            ptr::addr_of!(glue::retry_table),
            ptr::addr_of!(glue::retry_table_end),
        );
        return;
    }

    general_protection(regs, thread, type_, code);
}

/// `kernel_trap()` of i386/i386/trap.h.
///
/// # Safety
///
/// `regs` must be the live frame the assembly trap entry built.
pub(crate) unsafe fn kernel_trap(regs: &mut I386SavedState) {
    let type_ = regs.trapno;
    let code = regs.err;
    let thread = percpu::current_thread();

    match type_ {
        T_NO_FPU => {
            // SAFETY: the FPU path runs on the trap stack with no lock held.
            unsafe { fpu::fpnoextflt() };
        }
        T_FPU_FAULT => {
            // SAFETY: as above; the handler does not return.
            unsafe { fpu::fpextovrflt() }
        }
        T_FLOATING_POINT_ERROR => {
            // SAFETY: as above.
            unsafe { fpu::fpexterrflt() };
        }
        T_PAGE_FAULT => page_fault(regs, thread, type_, code),
        T_GENERAL_PROTECTION => general_protection(regs, thread, type_, code),
        _ => bad_trap(regs, type_, code),
    }
}

/// The emulated-system-call check of `user_trap()`: an `int 0x80` and, on
/// x86_64, an `lcall 7:0`, each with the instruction-pointer bump.
fn emulated_syscall(regs: &mut I386SavedState, thread: *mut Thread) -> bool {
    // SAFETY: the trap path runs on a live thread, whose task is live.
    if unsafe { (*(*thread).task).eml_dispatch }.is_null() {
        return false;
    }

    let opcode = fetch_byte(regs.eip, regs.cs, 0);
    let intno = fetch_byte(regs.eip, regs.cs, 1);
    if opcode == 0xcd && intno == 0x80 {
        regs.eip = regs.eip.wrapping_add(2);
        return true;
    }

    #[cfg(target_arch = "x86_64")]
    {
        let mut address = [0u8; 4];
        for (i, byte) in address.iter_mut().enumerate() {
            *byte = fetch_byte(regs.eip, regs.cs, i as c_ulong + 1);
        }
        let mut segment = [0u8; 2];
        for (i, byte) in segment.iter_mut().enumerate() {
            *byte = fetch_byte(regs.eip, regs.cs, i as c_ulong + 5);
        }
        if opcode == 0x9a && segment[0] == 0x7 && segment[1] == 0 {
            regs.eip = regs.eip.wrapping_add(7);
            return true;
        }
    }

    false
}

/// `user_page_fault_continue()` of `i386/i386/trap.c`: the continuation
/// `vm_fault()` resumes a user page fault through.
///
/// # Safety
///
/// `vm_fault()` calls this with the `kern_return_t` its fault attempt
/// finished with.
unsafe extern "C" fn user_page_fault_continue(kr: c_int) {
    let thread = percpu::current_thread();
    // SAFETY: the trap path runs on a live thread whose pcb is set up.
    let regs = unsafe { &mut (*(*thread).pcb).iss };

    if kr == KERN_SUCCESS {
        // SAFETY: the routine returns to user mode and never comes back.
        unsafe { glue::thread_exception_return() };
    }

    // SAFETY: the caller's contract, and `i386_exception()` does not return.
    unsafe { i386_exception(EXC_BAD_ACCESS, kr, regs.cr2 as c_long) };
}

/// `user_trap()` of i386/i386/trap.h.
///
/// # Safety
///
/// `regs` must be the live frame the assembly trap entry built.
pub(crate) unsafe fn user_trap(regs: &mut I386SavedState) -> c_int {
    let thread = percpu::current_thread();
    let type_ = regs.trapno;

    let (exc, code, subcode) = match type_ {
        T_DIVIDE_ERROR => (EXC_ARITHMETIC, EXC_I386_DIV, 0),
        T_DEBUG => {
            // SAFETY: the trap path runs on a live thread; its pcb may not be
            // built yet.
            unsafe {
                if !(*thread).pcb.is_null() {
                    (*(*thread).pcb).ims.ids.dr[6] =
                        (get_dr6() & 0x600f) as c_uint;
                }
            }
            set_dr6(0);
            (EXC_BREAKPOINT, EXC_I386_SGL, 0)
        }
        T_INT3 => (EXC_BREAKPOINT, EXC_I386_BPT, 0),
        T_OVERFLOW => (EXC_ARITHMETIC, EXC_I386_INTO, 0),
        T_OUT_OF_BOUNDS => (EXC_SOFTWARE, EXC_I386_BOUND, 0),
        T_INVALID_OPCODE => (EXC_BAD_INSTRUCTION, EXC_I386_INVOP, 0),
        T_NO_FPU | 32 => {
            // SAFETY: the FPU path runs on the trap stack with no lock held.
            unsafe { fpu::fpnoextflt() };
            return 0;
        }
        T_FPU_FAULT => {
            // SAFETY: as above; the handler does not return.
            unsafe { fpu::fpextovrflt() }
        }
        10 => (
            EXC_BAD_INSTRUCTION,
            EXC_I386_INVTSSFLT,
            (regs.err & 0xffff) as c_long,
        ),
        T_SEGMENT_NOT_PRESENT => (
            EXC_BAD_INSTRUCTION,
            EXC_I386_SEGNPFLT,
            (regs.err & 0xffff) as c_long,
        ),
        T_STACK_FAULT => (
            EXC_BAD_INSTRUCTION,
            EXC_I386_STKFLT,
            (regs.err & 0xffff) as c_long,
        ),
        T_GENERAL_PROTECTION => {
            if emulated_syscall(regs, thread) {
                return 1;
            }
            (
                EXC_BAD_INSTRUCTION,
                EXC_I386_GPFLT,
                (regs.err & 0xffff) as c_long,
            )
        }
        T_PAGE_FAULT => {
            // As above: the fault address is the frame's register value.
            let subcode = regs.cr2 as VmOffset;
            if subcode >= LINEAR_MIN_KERNEL_ADDRESS {
                // SAFETY: `i386_exception()` does not return.
                unsafe {
                    i386_exception(
                        EXC_BAD_ACCESS,
                        EXC_I386_PGFLT,
                        subcode as c_long,
                    )
                };
            }

            let protection = if regs.err & T_PF_WRITE != 0 {
                VmProt::READ | VmProt::WRITE
            } else {
                VmProt::READ
            };
            // SAFETY: the trap path runs on a live thread whose task map is
            // live; the continuation resumes the faulting thread.
            unsafe {
                glue::vm_fault(
                    (*(*thread).task).map.cast::<VmMap>(),
                    trunc_page(subcode),
                    protection,
                    0,
                    0,
                    Some(user_page_fault_continue),
                )
            };
            (0, 0, subcode as c_long)
        }
        T_FLOATING_POINT_ERROR => {
            // SAFETY: the FPU path runs on the trap stack with no lock held.
            unsafe { fpu::fpexterrflt() };
            return 0;
        }
        _ => {
            // SAFETY: `splhigh()` is the real asm routine.
            let _ = unsafe { glue::splhigh() };
            // SAFETY: `printf` accepts the C format and arguments.
            unsafe {
                glue::printf(
                    c"user trap, type %ld, code = %lx\n".as_ptr(),
                    type_,
                    regs.err,
                )
            };
            // SAFETY: `regs` is the live trap frame.
            unsafe { crate::arch::i386::debug_i386::dump_ss(regs) };
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"i386/i386/trap.c".as_ptr(),
                    line!() as c_int,
                    c"user_trap".as_ptr(),
                    c"trap".as_ptr(),
                )
            };
        }
    };

    // SAFETY: the trap path holds no lock, and `i386_exception()` does not
    // return.
    unsafe { i386_exception(exc, code, subcode) }
}

/// `handle_double_fault()` of `i386/i386/trap.c`, called from locore.
///
/// # Safety
///
/// `regs` must be the live frame the double-fault entry built.
pub(crate) unsafe fn handle_double_fault(regs: &I386SavedState) {
    // SAFETY: `regs` is the live double-fault frame.
    unsafe { crate::arch::i386::debug_i386::dump_ss(regs) };
    // SAFETY: `Panic()` does not return.
    unsafe {
        glue::Panic(
            c"i386/i386/trap.c".as_ptr(),
            line!() as c_int,
            c"handle_double_fault".as_ptr(),
            c"DOUBLE FAULT! This is critical\n".as_ptr(),
        )
    }
}
