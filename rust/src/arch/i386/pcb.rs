// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/pcb.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Derived from i386/i386/thread.h and i386/i386/seg.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Derived from i386/include/mach/i386/thread_status.h:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The PCB, the user-state save and restore and the context switch, which
//! `i386/i386/pcb.c` used to define and `i386/i386/pcb.h`,
//! `i386/i386/thread.h` and
//! `i386/include/mach/i386/thread_status.h` declare.

use crate::arch::i386::fpu::{self, I386FpSaveState};
use crate::arch::i386::percpu::{
    cpu_number, current_stack, current_thread, set_active_thread,
};
use crate::arch::i386::pmap::{activate_user, deactivate_user};
use crate::arch::i386::{db_interface, user_ldt};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{KERNEL_STACK_SIZE, VM_MAX_USER_ADDRESS};
use crate::glue;
use crate::kern::host::realhost;
use crate::kern::lock::SimpleLock;
use crate::kern::slab::{CacheInitFlags, KmemCache};
use crate::kern::task::{Task, current_task};
use crate::kern::thread::{Continuation, StackResume, Thread};
use crate::kern::types::KernError;
use crate::vm::vm_map::VmMap;
use core::ffi::{c_int, c_long, c_uint, c_ulong, c_ushort, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};

/// `KERNEL_STACK_ALIGN` of <i386/thread.h>.
#[cfg(target_pointer_width = "64")]
const KERNEL_STACK_ALIGN: usize = 16;
#[cfg(target_pointer_width = "32")]
const KERNEL_STACK_ALIGN: usize = 4;

/// `USER_STACK_ALIGN` of <i386/thread.h>.
#[cfg(target_pointer_width = "64")]
const USER_STACK_ALIGN: VmSize = 16;
#[cfg(target_pointer_width = "32")]
const USER_STACK_ALIGN: VmSize = 4;

/// `USER_CS` and `USER_DS` of <i386/ldt.h>.
#[cfg(target_pointer_width = "64")]
const USER_CS: c_ulong = 0x1f;
#[cfg(target_pointer_width = "64")]
const USER_DS: c_ulong = 0x17;
#[cfg(target_pointer_width = "32")]
const USER_CS: c_ulong = 0x17;
#[cfg(target_pointer_width = "32")]
const USER_DS: c_ulong = 0x1f;

/// `KERNEL_LDT`, `USER_LDT` and `USER_GDT` of <i386/gdt.h>.
const KERNEL_LDT: c_ushort = 0x18;
const USER_LDT: c_ushort = 0x28;
const USER_GDT: c_ushort = 0x48;
/// `USER_GDT_SLOTS` of <i386/gdt.h>: the per-thread GDT entries.
const USER_GDT_SLOTS: usize = 2;

/// `IOPB_INVAL` of <i386/io_perm.h>: an offset outside the permission
/// bitmap, which disables all permission.
const IOPB_INVAL: c_ushort = 0x2fff;
/// `EFL_VM` of <mach/i386/eflags.h>.
#[cfg(target_pointer_width = "32")]
const EFL_VM: c_ulong = 0x0002_0000;
/// `EFL_TF` and `EFL_IF` of <mach/i386/eflags.h>.
#[cfg(target_pointer_width = "32")]
const EFL_TF: c_ulong = 0x0000_0100;
const EFL_IF: c_ulong = 0x0000_0200;
/// `EFL_USER_SET` and `EFL_USER_CLEAR` of <i386/eflags.h>.
const EFL_USER_SET: c_ulong = EFL_IF;
const EFL_USER_CLEAR: c_ulong = 0x3000 | 0x4000 | 0x0001_0000;
/// `V86_IF_PENDING` of <i386/thread.h>.
#[cfg(target_pointer_width = "32")]
const V86_IF_PENDING: c_ushort = 0x8000;

/// `SEL_PL` and `SEL_PL_U` of <i386/seg.h>.
const SEL_PL: c_uint = 0x03;
const SEL_PL_U: c_uint = 0x03;

/// `MSR_REG_FSBASE`, `MSR_REG_GSBASE` and `MSR_REG_KGSBASE` of <i386/msr.h>;
/// the other registers below are the ones `ldt.rs` and `gdt.rs` write.
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_FSBASE: u32 = 0xc000_0100;
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_GSBASE: u32 = 0xc000_0101;
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_KGSBASE: u32 = 0xc000_0102;
/// `MSR_REG_EFER`, `MSR_REG_STAR`, `MSR_REG_LSTAR` and `MSR_REG_FMASK` of
/// <i386/msr.h>.
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_EFER: u32 = 0xc000_0080;
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_STAR: u32 = 0xc000_0081;
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_LSTAR: u32 = 0xc000_0082;
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_REG_FMASK: u32 = 0xc000_0084;
/// `MSR_EFER_SCE` of <i386/msr.h>: enable `syscall`/`sysret`.
#[cfg(target_pointer_width = "64")]
pub(crate) const MSR_EFER_SCE: u64 = 0x1;

/// The thread-state flavors of <mach/i386/thread_status.h>.
pub(crate) const I386_THREAD_STATE: c_int = 1;
pub(crate) const I386_ISA_PORT_MAP_STATE: c_int = 3;
#[cfg(target_pointer_width = "32")]
pub(crate) const I386_V86_ASSIST_STATE: c_int = 4;
pub(crate) const I386_REGS_SEGS_STATE: c_int = 5;
pub(crate) const I386_DEBUG_STATE: c_int = 6;
#[cfg(target_pointer_width = "64")]
pub(crate) const I386_FSGS_BASE_STATE: c_int = 7;
/// `THREAD_STATE_FLAVOR_LIST` of <mach/thread_status.h>.
const THREAD_STATE_FLAVOR_LIST: c_int = 0;

/// `i386_THREAD_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_THREAD_STATE_COUNT: c_uint =
    (size_of::<I386ThreadState>() / size_of::<c_uint>()) as c_uint;
/// `i386_FLOAT_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_FLOAT_STATE_COUNT: c_uint =
    (size_of::<fpu::I386FloatState>() / size_of::<c_uint>()) as c_uint;
/// `i386_ISA_PORT_MAP_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_ISA_PORT_MAP_STATE_COUNT: c_uint =
    (size_of::<I386IsaPortMapState>() / size_of::<c_uint>()) as c_uint;
/// `i386_DEBUG_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_DEBUG_STATE_COUNT: c_uint =
    (size_of::<I386DebugState>() / size_of::<c_uint>()) as c_uint;
#[cfg(target_pointer_width = "32")]
/// `i386_V86_ASSIST_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_V86_ASSIST_STATE_COUNT: c_uint =
    (size_of::<I386V86AssistState>() / size_of::<c_uint>()) as c_uint;
#[cfg(target_pointer_width = "64")]
/// `i386_FSGS_BASE_STATE_COUNT` of <mach/i386/thread_status.h>.
const I386_FSGS_BASE_STATE_COUNT: c_uint = 4;

/// `struct v86_segs` of <i386/thread.h>.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V86Segs {
    pub v86_es: c_ulong,
    pub v86_ds: c_ulong,
    pub v86_fs: c_ulong,
    pub v86_gs: c_ulong,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<V86Segs>() == 16);
    assert!(align_of::<V86Segs>() == align_of::<c_ulong>());
    assert!(offset_of!(V86Segs, v86_es) == 0);
    assert!(offset_of!(V86Segs, v86_ds) == 4);
    assert!(offset_of!(V86Segs, v86_fs) == 8);
    assert!(offset_of!(V86Segs, v86_gs) == 12);
};

/// `struct i386_saved_state` of <i386/thread.h>: the user registers as saved
/// on kernel entry.  It lives in the pcb and is pushed on the stack for
/// kernel exceptions.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386SavedState {
    pub r15: c_ulong,
    pub r14: c_ulong,
    pub r13: c_ulong,
    pub r12: c_ulong,
    pub r11: c_ulong,
    pub r10: c_ulong,
    pub r9: c_ulong,
    pub r8: c_ulong,
    pub edi: c_ulong,
    pub esi: c_ulong,
    pub ebp: c_ulong,
    pub cr2: c_ulong,
    pub ebx: c_ulong,
    pub edx: c_ulong,
    pub ecx: c_ulong,
    pub eax: c_ulong,
    pub trapno: c_ulong,
    pub err: c_ulong,
    pub eip: c_ulong,
    pub cs: c_ulong,
    pub efl: c_ulong,
    pub uesp: c_ulong,
    pub ss: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386SavedState>() == 184);
    assert!(align_of::<I386SavedState>() == align_of::<c_ulong>());
    assert!(offset_of!(I386SavedState, r15) == 0);
    assert!(offset_of!(I386SavedState, r8) == 56);
    assert!(offset_of!(I386SavedState, edi) == 64);
    assert!(offset_of!(I386SavedState, cr2) == 88);
    assert!(offset_of!(I386SavedState, eax) == 120);
    assert!(offset_of!(I386SavedState, trapno) == 128);
    assert!(offset_of!(I386SavedState, eip) == 144);
    assert!(offset_of!(I386SavedState, efl) == 160);
    assert!(offset_of!(I386SavedState, uesp) == 168);
    assert!(offset_of!(I386SavedState, ss) == 176);
};

/// `struct i386_saved_state`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386SavedState {
    pub gs: c_ulong,
    pub fs: c_ulong,
    pub es: c_ulong,
    pub ds: c_ulong,
    pub edi: c_ulong,
    pub esi: c_ulong,
    pub ebp: c_ulong,
    pub cr2: c_ulong,
    pub ebx: c_ulong,
    pub edx: c_ulong,
    pub ecx: c_ulong,
    pub eax: c_ulong,
    pub trapno: c_ulong,
    pub err: c_ulong,
    pub eip: c_ulong,
    pub cs: c_ulong,
    pub efl: c_ulong,
    pub uesp: c_ulong,
    pub ss: c_ulong,
    pub v86_segs: V86Segs,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386SavedState>() == 92);
    assert!(align_of::<I386SavedState>() == align_of::<c_ulong>());
    assert!(offset_of!(I386SavedState, gs) == 0);
    assert!(offset_of!(I386SavedState, edi) == 16);
    assert!(offset_of!(I386SavedState, cr2) == 28);
    assert!(offset_of!(I386SavedState, eax) == 44);
    assert!(offset_of!(I386SavedState, eip) == 56);
    assert!(offset_of!(I386SavedState, efl) == 64);
    assert!(offset_of!(I386SavedState, uesp) == 68);
    assert!(offset_of!(I386SavedState, ss) == 72);
    assert!(offset_of!(I386SavedState, v86_segs) == 76);
};

/// `struct i386_interrupt_state` of <i386/thread.h>: the registers an
/// interrupt pushes before the kernel can switch to the interrupt stack.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386InterruptState {
    pub r12: c_long,
    pub r11: c_long,
    pub r10: c_long,
    pub r9: c_long,
    pub r8: c_long,
    pub rdi: c_long,
    pub rsi: c_long,
    pub edx: c_long,
    pub ecx: c_long,
    pub eax: c_long,
    pub eip: c_long,
    pub cs: c_long,
    pub efl: c_long,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386InterruptState>() == 104);
    assert!(align_of::<I386InterruptState>() == align_of::<c_long>());
    assert!(offset_of!(I386InterruptState, r12) == 0);
    assert!(offset_of!(I386InterruptState, r8) == 32);
    assert!(offset_of!(I386InterruptState, edx) == 56);
    assert!(offset_of!(I386InterruptState, eip) == 80);
    assert!(offset_of!(I386InterruptState, efl) == 96);
};

/// `struct i386_interrupt_state`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386InterruptState {
    pub gs: c_long,
    pub fs: c_long,
    pub es: c_long,
    pub ds: c_long,
    pub edx: c_long,
    pub ecx: c_long,
    pub eax: c_long,
    pub eip: c_long,
    pub cs: c_long,
    pub efl: c_long,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386InterruptState>() == 40);
    assert!(align_of::<I386InterruptState>() == align_of::<c_long>());
    assert!(offset_of!(I386InterruptState, gs) == 0);
    assert!(offset_of!(I386InterruptState, edx) == 16);
    assert!(offset_of!(I386InterruptState, eip) == 28);
    assert!(offset_of!(I386InterruptState, efl) == 36);
};

/// `struct i386_kernel_state` of <i386/thread.h>: the kernel registers as
/// saved in a context switch, at the base of the stack.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386KernelState {
    pub k_ebx: c_long,
    pub k_esp: c_long,
    pub k_ebp: c_long,
    pub k_eip: c_long,
    pub k_r12: c_long,
    pub k_r13: c_long,
    pub k_r14: c_long,
    pub k_r15: c_long,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386KernelState>() == 64);
    assert!(align_of::<I386KernelState>() == align_of::<c_long>());
    assert!(offset_of!(I386KernelState, k_eip) == 24);
    assert!(offset_of!(I386KernelState, k_r12) == 32);
};

/// `struct i386_kernel_state`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386KernelState {
    pub k_ebx: c_long,
    pub k_esp: c_long,
    pub k_ebp: c_long,
    pub k_edi: c_long,
    pub k_esi: c_long,
    pub k_eip: c_long,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386KernelState>() == 24);
    assert!(align_of::<I386KernelState>() == align_of::<c_long>());
    assert!(offset_of!(I386KernelState, k_eip) == 20);
};

/// `struct i386_exception_link` of <i386/thread.h>: the pointer to the
/// current thread's user registers at the high end of the kernel stack.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386ExceptionLink {
    pub saved_state: *mut I386SavedState,
}

const _: () = {
    assert!(size_of::<I386ExceptionLink>() == size_of::<*mut c_void>());
    assert!(align_of::<I386ExceptionLink>() == align_of::<*mut c_void>());
    assert!(offset_of!(I386ExceptionLink, saved_state) == 0);
};

/// `struct i386_debug_state` of <machine/thread_status.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386DebugState {
    pub dr: [c_uint; 8],
}

const _: () = {
    assert!(size_of::<I386DebugState>() == 32);
    assert!(align_of::<I386DebugState>() == align_of::<c_uint>());
    assert!(offset_of!(I386DebugState, dr) == 0);
};

/// `struct v86_assist_state` of <i386/thread.h>.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386V86AssistState {
    pub int_table: VmOffset,
    pub int_count: c_ushort,
    pub flags: c_ushort,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386V86AssistState>() == 8);
    assert!(align_of::<I386V86AssistState>() == align_of::<VmOffset>());
    assert!(offset_of!(I386V86AssistState, int_table) == 0);
    assert!(offset_of!(I386V86AssistState, int_count) == 4);
    assert!(offset_of!(I386V86AssistState, flags) == 6);
};

/// `struct i386_segment_base_state` of <i386/thread.h>.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386SegmentBaseState {
    pub fsbase: c_ulong,
    pub gsbase: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386SegmentBaseState>() == 16);
    assert!(align_of::<I386SegmentBaseState>() == align_of::<c_ulong>());
    assert!(offset_of!(I386SegmentBaseState, fsbase) == 0);
    assert!(offset_of!(I386SegmentBaseState, gsbase) == 8);
};

/// `struct real_descriptor` of <i386/seg.h>, whose bitfields are kept as two
/// words because Rust cannot express them.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RealDescriptor {
    pub limit_low_base_low: u32,
    pub access_and_base_high: u32,
}

impl RealDescriptor {
    /// An all-zero descriptor, the image a C `static` began with.
    pub(crate) const ZERO: Self = Self {
        limit_low_base_low: 0,
        access_and_base_high: 0,
    };

    /// The `access` byte of the C's `desc->access`.
    pub(crate) const fn access(&self) -> u8 {
        (self.access_and_base_high >> 8) as u8
    }

    /// The `granularity` nibble of the C's `desc->granularity`.
    pub(crate) const fn granularity(&self) -> u8 {
        ((self.access_and_base_high >> 20) & 0xf) as u8
    }

    /// The `limit_low` half of the C's `desc->limit_low`.
    pub(crate) const fn limit_low(&self) -> u16 {
        self.limit_low_base_low as u16
    }
}

const _: () = {
    assert!(size_of::<RealDescriptor>() == 8);
    assert!(align_of::<RealDescriptor>() == align_of::<u32>());
    assert!(offset_of!(RealDescriptor, limit_low_base_low) == 0);
    assert!(offset_of!(RealDescriptor, access_and_base_high) == 4);
};

/// `struct user_ldt` of <i386/user_ldt.h>: the descriptor for the table
/// itself followed by the table, which is larger than one entry in practice.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserLdt {
    pub desc: RealDescriptor,
    pub ldt: [RealDescriptor; 1],
}

const _: () = {
    assert!(size_of::<UserLdt>() == 16);
    assert!(align_of::<UserLdt>() == align_of::<RealDescriptor>());
    assert!(offset_of!(UserLdt, desc) == 0);
    assert!(offset_of!(UserLdt, ldt) == 8);
};

/// `IOPB_BYTES` of <i386/io_perm.h>: one bit per I/O port, 8192 bytes.
const IOPB_BYTES: usize = 0x2000;

/// The x86 task state segment, `struct i386_tss` of <i386/tss.h>.
#[cfg(target_pointer_width = "64")]
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct I386Tss {
    pub reserved0: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    pub reserved1: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    pub reserved2: u64,
    pub reserved3: c_ushort,
    pub io_bit_map_offset: c_ushort,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386Tss>() == 104);
    assert!(align_of::<I386Tss>() == 1);
    assert!(offset_of!(I386Tss, rsp0) == 4);
    assert!(offset_of!(I386Tss, io_bit_map_offset) == 102);
};

/// The x86 task state segment, `struct i386_tss` of <i386/tss.h>, 32-bit
/// layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386Tss {
    pub back_link: c_int,
    pub esp0: c_int,
    pub ss0: c_int,
    pub esp1: c_int,
    pub ss1: c_int,
    pub esp2: c_int,
    pub ss2: c_int,
    pub cr3: c_int,
    pub eip: c_int,
    pub eflags: c_int,
    pub eax: c_int,
    pub ecx: c_int,
    pub edx: c_int,
    pub ebx: c_int,
    pub esp: c_int,
    pub ebp: c_int,
    pub esi: c_int,
    pub edi: c_int,
    pub es: c_int,
    pub cs: c_int,
    pub ss: c_int,
    pub ds: c_int,
    pub fs: c_int,
    pub gs: c_int,
    pub ldt: c_int,
    pub trace_trap: c_ushort,
    pub io_bit_map_offset: c_ushort,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386Tss>() == 104);
    assert!(align_of::<I386Tss>() == align_of::<c_int>());
    assert!(offset_of!(I386Tss, esp0) == 4);
    assert!(offset_of!(I386Tss, io_bit_map_offset) == 102);
};

/// `struct task_tss` of <i386/tss.h>: the TSS plus the I/O permission
/// bitmap and its terminating barrier byte.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TaskTss {
    pub tss: I386Tss,
    pub iopb: [u8; IOPB_BYTES],
    pub barrier: u8,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<TaskTss>() == 8297);
    assert!(align_of::<TaskTss>() == 1);
    assert!(offset_of!(TaskTss, tss) == 0);
    assert!(offset_of!(TaskTss, barrier) == 8296);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<TaskTss>() == 8300);
    assert!(align_of::<TaskTss>() == align_of::<c_int>());
    assert!(offset_of!(TaskTss, tss) == 0);
    assert!(offset_of!(TaskTss, barrier) == 8296);
};

/// `struct i386_machine_state` of <i386/thread.h>: the machine-dependent
/// part of a pcb that is not saved by default.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
pub struct I386MachineState {
    pub ldt: *mut UserLdt,
    pub ifps: *mut I386FpSaveState,
    pub user_gdt: [RealDescriptor; USER_GDT_SLOTS],
    pub ids: I386DebugState,
    pub sbs: I386SegmentBaseState,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386MachineState>() == 80);
    assert!(align_of::<I386MachineState>() == align_of::<*mut c_void>());
    assert!(offset_of!(I386MachineState, ldt) == 0);
    assert!(offset_of!(I386MachineState, ifps) == 8);
    assert!(offset_of!(I386MachineState, user_gdt) == 16);
    assert!(offset_of!(I386MachineState, ids) == 32);
    assert!(offset_of!(I386MachineState, sbs) == 64);
};

/// `struct i386_machine_state`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
pub struct I386MachineState {
    pub ldt: *mut UserLdt,
    pub ifps: *mut I386FpSaveState,
    pub v86s: I386V86AssistState,
    pub user_gdt: [RealDescriptor; USER_GDT_SLOTS],
    pub ids: I386DebugState,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386MachineState>() == 64);
    assert!(align_of::<I386MachineState>() == align_of::<*mut c_void>());
    assert!(offset_of!(I386MachineState, ldt) == 0);
    assert!(offset_of!(I386MachineState, ifps) == 4);
    assert!(offset_of!(I386MachineState, v86s) == 8);
    assert!(offset_of!(I386MachineState, user_gdt) == 16);
    assert!(offset_of!(I386MachineState, ids) == 32);
};

/// `struct pcb` of <i386/thread.h>: the process control block.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
pub struct Pcb {
    pub iis: [I386InterruptState; 2],
    pub pad: c_ulong,
    pub iss: I386SavedState,
    pub ims: I386MachineState,
    pub lock: SimpleLock,
    pub init_control: c_ushort,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Pcb>() == 488);
    assert!(align_of::<Pcb>() == align_of::<c_ulong>());
    assert!(offset_of!(Pcb, iis) == 0);
    assert!(offset_of!(Pcb, pad) == 208);
    assert!(offset_of!(Pcb, iss) == 216);
    assert!(offset_of!(Pcb, ims) == 400);
    assert!(offset_of!(Pcb, lock) == 480);
    assert!(offset_of!(Pcb, init_control) == 484);
};

/// `struct pcb`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
pub struct Pcb {
    pub iis: [I386InterruptState; 2],
    pub iss: I386SavedState,
    pub ims: I386MachineState,
    pub lock: SimpleLock,
    pub init_control: c_ushort,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Pcb>() == 244);
    assert!(offset_of!(Pcb, iis) == 0);
    assert!(offset_of!(Pcb, iss) == 80);
    assert!(offset_of!(Pcb, ims) == 172);
    assert!(offset_of!(Pcb, lock) == 236);
    assert!(offset_of!(Pcb, init_control) == 240);
};

/// `struct i386_thread_state` of <machine/thread_status.h>, the x86_64
/// layout.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386ThreadState {
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: c_uint,
    pub rfl: u64,
    pub ursp: u64,
    pub ss: c_uint,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386ThreadState>() == 168);
    assert!(align_of::<I386ThreadState>() == align_of::<u64>());
    assert!(offset_of!(I386ThreadState, r8) == 0);
    assert!(offset_of!(I386ThreadState, rdi) == 64);
    assert!(offset_of!(I386ThreadState, rip) == 128);
    assert!(offset_of!(I386ThreadState, cs) == 136);
    assert!(offset_of!(I386ThreadState, rfl) == 144);
    assert!(offset_of!(I386ThreadState, ursp) == 152);
    assert!(offset_of!(I386ThreadState, ss) == 160);
};

/// `struct i386_thread_state`, the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386ThreadState {
    pub gs: c_uint,
    pub fs: c_uint,
    pub es: c_uint,
    pub ds: c_uint,
    pub edi: c_uint,
    pub esi: c_uint,
    pub ebp: c_uint,
    pub esp: c_uint,
    pub ebx: c_uint,
    pub edx: c_uint,
    pub ecx: c_uint,
    pub eax: c_uint,
    pub eip: c_uint,
    pub cs: c_uint,
    pub efl: c_uint,
    pub uesp: c_uint,
    pub ss: c_uint,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<I386ThreadState>() == 68);
    assert!(align_of::<I386ThreadState>() == align_of::<c_uint>());
    assert!(offset_of!(I386ThreadState, gs) == 0);
    assert!(offset_of!(I386ThreadState, edi) == 16);
    assert!(offset_of!(I386ThreadState, eip) == 48);
    assert!(offset_of!(I386ThreadState, cs) == 52);
    assert!(offset_of!(I386ThreadState, efl) == 56);
    assert!(offset_of!(I386ThreadState, uesp) == 60);
    assert!(offset_of!(I386ThreadState, ss) == 64);
};

/// `struct i386_isa_port_map_state` of <machine/thread_status.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386IsaPortMapState {
    pub pm: [u8; 0x400 >> 3],
}

const _: () = {
    assert!(size_of::<I386IsaPortMapState>() == 128);
    assert!(align_of::<I386IsaPortMapState>() == 1);
    assert!(offset_of!(I386IsaPortMapState, pm) == 0);
};

/// `struct i386_fsgs_base_state` of <machine/thread_status.h>.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct I386FsgsBaseState {
    pub fs_base: c_ulong,
    pub gs_base: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<I386FsgsBaseState>() == 16);
    assert!(align_of::<I386FsgsBaseState>() == align_of::<c_ulong>());
    assert!(offset_of!(I386FsgsBaseState, fs_base) == 0);
    assert!(offset_of!(I386FsgsBaseState, gs_base) == 8);
};

/// `struct exec_info` of <mach/exec/exec.h>, of which `set_user_regs()`
/// reads the entry point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExecInfo {
    pub format: c_int,
    pub entry: VmOffset,
    pub init_dp: VmOffset,
    pub interp: VmOffset,
    pub stack_prot: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<ExecInfo>() == 40);
    assert!(align_of::<ExecInfo>() == align_of::<VmOffset>());
    assert!(offset_of!(ExecInfo, entry) == 8);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<ExecInfo>() == 20);
    assert!(align_of::<ExecInfo>() == align_of::<VmOffset>());
    assert!(offset_of!(ExecInfo, entry) == 4);
};

/// `pcb_cache` of i386/i386/pcb.c: the `struct pcb` slab cache.
static mut PCB_CACHE: KmemCache = KmemCache::zeroed();

/// `kernel_stack` of i386/i386/pcb.c: the top of each CPU's active stack,
/// which `locore.S` and `cswitch.S` index by CPU number.
#[unsafe(no_mangle)]
pub static mut kernel_stack: [VmOffset; crate::config::NCPUS] =
    [0; crate::config::NCPUS];

/// `sel_idx()` of <i386/seg.h>.
fn sel_idx(selector: c_ushort) -> usize {
    usize::from(selector >> 3)
}

/// `STACK_IKS()` of <i386/thread.h>.
fn stack_iks(stack: VmOffset) -> *mut I386KernelState {
    ptr::with_exposed_provenance_mut(
        stack + KERNEL_STACK_SIZE - size_of::<I386KernelState>(),
    )
}

/// `STACK_IEL()` of <i386/thread.h>.
fn stack_iel(stack: VmOffset) -> *mut I386ExceptionLink {
    ptr::with_exposed_provenance_mut(
        stack + KERNEL_STACK_SIZE
            - size_of::<I386KernelState>()
            - size_of::<I386ExceptionLink>(),
    )
}

/// The C's `get_ldt()` of <i386/proc_reg.h>.
fn get_ldt() -> c_ushort {
    let segment: c_ushort;
    // SAFETY: `sldt` reads the local descriptor table register at CPL0.
    unsafe {
        core::arch::asm!("sldt {segment:x}", segment = out(reg) segment, options(nostack))
    };
    segment
}

/// The C's `set_ldt()` of <i386/proc_reg.h>.
fn set_ldt(segment: c_ushort) {
    // SAFETY: `lldt` loads the LDT register with a descriptor the kernel
    // built in its GDT.
    unsafe {
        core::arch::asm!("lldt {segment:x}", segment = in(reg) segment, options(nostack))
    };
}

/// The C's `wrmsr()` of <i386/msr.h>.
#[cfg(target_pointer_width = "64")]
pub(crate) fn write_msr(register: u32, value: u64) {
    // SAFETY: `wrmsr` writes a model-specific register at CPL0; the caller
    // names one the CPU has.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") register,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack),
        )
    };
}

/// The C's `rdmsr()` of <i386/msr.h>.
#[cfg(target_pointer_width = "64")]
pub(crate) fn read_msr(register: u32) -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: `rdmsr` reads a model-specific register at CPL0; the caller
    // names one the CPU has.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") register,
            out("eax") low,
            out("edx") high,
            options(nostack),
        )
    };
    (u64::from(high) << 32) | u64::from(low)
}

/// `fpu_save_context()` of <i386/fpu.h>: save the registers if they are live.
///
/// # Safety
///
/// `thread` must be live and not running on another CPU.
unsafe fn fpu_save_context(thread: *mut Thread) {
    // SAFETY: the caller promises a live thread whose pcb came from
    // `pcb_init()`.
    let ifps = unsafe { (*(*thread).pcb).ims.ifps };
    // SAFETY: `ifps` is the thread's own live save area.
    if !ifps.is_null() && unsafe { (*ifps).fp_valid } == 0 {
        // SAFETY: the caller promises the thread is not running elsewhere.
        unsafe { fpu::fpu_save(ifps) };
        fpu::set_ts();
    }
}

/// `stack_attach()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `thread` must be a live thread whose stack is not attached, `stack` must
/// be a whole kernel stack, and `continuation` a stack continuation.
pub(crate) unsafe fn stack_attach(
    thread: *mut Thread,
    stack: VmOffset,
    continuation: StackResume,
) {
    // SAFETY: the caller promises a live thread.
    unsafe { (*thread).kernel_stack = stack };

    let iks = stack_iks(stack);
    let iel = stack_iel(stack);
    // SAFETY: `stack` is a whole kernel stack, so the two records at its top
    // are inside it, and the thread's pcb is live.
    unsafe {
        (*iks).k_eip = glue::Thread_continue as *const () as usize as c_long;
        (*iks).k_ebx = continuation.map_or(0, |f| f as usize) as c_long;
        (*iks).k_esp = iel as usize as c_long;
        (*iks).k_ebp = 0;
        (*iel).saved_state = &raw mut (*(*thread).pcb).iss;
    }
}

/// `switch_ktss()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `pcb` must be the live pcb of a thread about to run on this CPU.
pub(crate) unsafe fn switch_ktss(pcb: *mut Pcb) {
    let mycpu = cpu_number();

    #[cfg(target_pointer_width = "64")]
    // SAFETY: the caller promises a live pcb; its `iss` is inside it.
    let pcb_stack_top =
        unsafe { ptr::addr_of!((*pcb).iss).add(1) as VmOffset };
    #[cfg(target_pointer_width = "32")]
    // SAFETY: as above; `v86_segs` is also inside the pcb.
    let pcb_stack_top = unsafe {
        if (*pcb).iss.efl & EFL_VM != 0 {
            ptr::addr_of!((*pcb).iss).add(1) as VmOffset
        } else {
            ptr::addr_of!((*pcb).iss.v86_segs) as VmOffset
        }
    };

    // SAFETY: `mp_ktss` holds one live TSS per CPU, and `mycpu` names the
    // CPU this code runs on.
    let ktss = unsafe {
        (*ptr::addr_of!(crate::arch::i386::mp_desc::mp_ktss))[mycpu as usize]
    };
    // SAFETY: as above; the field is the one the architecture uses for the
    // ring-0 stack.
    #[cfg(target_pointer_width = "64")]
    unsafe {
        ptr::addr_of_mut!((*ktss).tss.rsp0).write(pcb_stack_top as u64)
    };
    #[cfg(target_pointer_width = "32")]
    unsafe {
        ptr::addr_of_mut!((*ktss).tss.esp0).write(pcb_stack_top as c_int)
    };

    // SAFETY: the caller promises a live pcb and `mp_gdt` one table per CPU.
    unsafe {
        let tldt = (*pcb).ims.ldt;
        if tldt.is_null() {
            if get_ldt() != KERNEL_LDT {
                set_ldt(KERNEL_LDT);
            }
        } else {
            let gdt = (*ptr::addr_of!(crate::arch::i386::mp_desc::mp_gdt))
                [mycpu as usize];
            *gdt.add(sel_idx(USER_LDT)) = (*tldt).desc;
            set_ldt(USER_LDT);
        }

        let gdt = (*ptr::addr_of!(crate::arch::i386::mp_desc::mp_gdt))
            [mycpu as usize];
        *gdt.add(sel_idx(USER_GDT)) = (*pcb).ims.user_gdt[0];
        *gdt.add(sel_idx(USER_GDT) + 1) = (*pcb).ims.user_gdt[1];
    }

    #[cfg(target_pointer_width = "64")]
    // SAFETY: the caller's pcb is live; the segment-base fields are inside
    // it, and the MSRs are the ones the C wrote.
    unsafe {
        write_msr(MSR_REG_FSBASE, (*pcb).ims.sbs.fsbase);
        write_msr(MSR_REG_KGSBASE, (*pcb).ims.sbs.gsbase);
    }

    // SAFETY: the caller promises a live pcb, which `db_load_context()`
    // only reads.
    unsafe { db_interface::load_context(pcb) };
}

/// `update_ktss_iopb()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// The caller must hold `iopb_lock` of the task whose bitmap this is, and
/// `new_iopb` must be readable for `size` bytes when it is non-null.
pub(crate) unsafe fn update_ktss_iopb(new_iopb: *mut u8, size: c_ushort) {
    // SAFETY: `mp_ktss` holds one live TSS per CPU.
    let tss = unsafe {
        (*ptr::addr_of!(crate::arch::i386::mp_desc::mp_ktss))
            [cpu_number() as usize]
    };
    if !new_iopb.is_null() && size > 0 {
        let offset = offset_of!(TaskTss, barrier) - usize::from(size);
        // SAFETY: the task's `iopb_size` is an `IOPB_MAX`-wide port count, so
        // the offset lands inside the task_tss the TSS points at; the caller
        // promises the source is readable.
        unsafe {
            ptr::addr_of_mut!((*tss).tss.io_bit_map_offset)
                .write(offset as c_ushort);
            ptr::copy_nonoverlapping(
                new_iopb,
                (tss as *mut u8).add(offset),
                usize::from(size),
            );
        }
    } else {
        // SAFETY: the caller passes a live TSS.
        unsafe {
            ptr::addr_of_mut!((*tss).tss.io_bit_map_offset).write(IOPB_INVAL)
        };
    }
}

/// `stack_handoff()` of `kern/sched_prim.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// `old` must be the running thread and `new` the thread about to run, both
/// live and not running on any other CPU.
pub(crate) unsafe fn stack_handoff(old: *mut Thread, new: *mut Thread) {
    let mycpu = cpu_number();
    // SAFETY: the caller promises both threads live.
    unsafe { fpu_save_context(old) };

    // SAFETY: as above; each task's map is the one it was created with.
    unsafe {
        let old_task = (*old).task;
        let new_task = (*new).task;
        if old_task != new_task {
            deactivate_user((*(*old_task).map.cast::<VmMap>()).pmap, mycpu);
            activate_user((*(*new_task).map.cast::<VmMap>()).pmap, mycpu);

            (*new_task).machine.iopb_lock.lock();
            // The C passed the `int` through `io_port_t`; the size is at
            // most IOPB_MAX.
            update_ktss_iopb(
                (*new_task).machine.iopb,
                (*new_task).machine.iopb_size as c_ushort,
            );
            (*new_task).machine.iopb_lock.unlock();
        }
    }

    // SAFETY: the new thread's pcb is live.
    unsafe { switch_ktss((*new).pcb) };

    let stack = current_stack();
    // SAFETY: the caller promises both threads live and running here.
    unsafe {
        (*old).kernel_stack = 0;
        (*new).kernel_stack = stack;
        set_active_thread(new);
        (*stack_iel(stack)).saved_state = &raw mut (*(*new).pcb).iss;
    }
}

/// `switch_context()` of <kern/sched_prim.h>, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// `old` must be the running thread and `new` the thread about to run, both
/// live and not running on any other CPU; `continuation` is where `old`
/// resumes.
pub(crate) unsafe fn switch_context(
    old: *mut Thread,
    continuation: Continuation,
    new: *mut Thread,
) -> *mut Thread {
    // SAFETY: the caller promises both threads live.
    unsafe { fpu_save_context(old) };

    let mycpu = cpu_number();
    // SAFETY: as above; each task's map is the one it was created with.
    unsafe {
        let old_task = (*old).task;
        let new_task = (*new).task;
        if old_task != new_task {
            deactivate_user((*(*old_task).map.cast::<VmMap>()).pmap, mycpu);
            activate_user((*(*new_task).map.cast::<VmMap>()).pmap, mycpu);

            (*new_task).machine.iopb_lock.lock();
            update_ktss_iopb(
                (*new_task).machine.iopb,
                (*new_task).machine.iopb_size as c_ushort,
            );
            (*new_task).machine.iopb_lock.unlock();
        }

        // SAFETY: the new thread's pcb is live, and `Switch_context()`
        // switches to its saved kernel context.
        switch_ktss((*new).pcb);
        glue::Switch_context(old, continuation, new)
    }
}

/// `pcb_module_init()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// Called once at startup, before any thread is created.
pub(crate) unsafe fn pcb_module_init() {
    // SAFETY: the caller promises this runs once, before any thread can
    // allocate a pcb, so nothing else touches the cache object while the
    // slab layer builds it.
    unsafe {
        (*ptr::addr_of_mut!(PCB_CACHE)).init(
            b"pcb",
            size_of::<Pcb>(),
            KERNEL_STACK_ALIGN,
            None,
            CacheInitFlags::EMPTY,
        );
    }
    // SAFETY: the FPU cache is built once, in the same startup step.
    unsafe { fpu::fpu_module_init() };
}

/// The C's `panic("pcb_init")` when the slab layer reports no memory.
fn panic_no_pcb() -> ! {
    // SAFETY: `Panic()` does not return.
    unsafe {
        glue::Panic(
            c"i386/i386/pcb.c".as_ptr(),
            line!() as c_int,
            c"pcb_init".as_ptr(),
            c"pcb_init".as_ptr(),
        )
    }
}

/// `pcb_init()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `parent_task` must be the task `thread` is being created in, both live,
/// called before the thread can run.
///
/// # Panics
///
/// Halts the kernel if the slab layer reports no memory for the pcb, as the
/// C `panic("pcb_init")` did.
pub(crate) unsafe fn pcb_init(parent_task: *mut Task, thread: *mut Thread) {
    // SAFETY: the caller promises startup's single-threaded context, so the
    // cache is not touched concurrently.
    let pcb = match unsafe { (*ptr::addr_of_mut!(PCB_CACHE)).alloc() } {
        Some(buf) => buf.as_ptr().cast::<Pcb>(),
        None => panic_no_pcb(),
    };
    // SAFETY: the object just came from the cache, is uninitialized, and the
    // C zeroed it whole so no random value would leak to the user.
    unsafe {
        ptr::write_bytes(pcb.cast::<u8>(), 0, size_of::<Pcb>());
        (*pcb).lock.init();

        (*pcb).iss.cs = USER_CS;
        (*pcb).iss.ss = USER_DS;
        #[cfg(target_pointer_width = "32")]
        {
            (*pcb).iss.ds = USER_DS;
            (*pcb).iss.es = USER_DS;
            (*pcb).iss.fs = USER_DS;
            (*pcb).iss.gs = USER_DS;
        }
        (*pcb).iss.efl = EFL_USER_SET;

        (*thread).pcb = pcb;

        if !current_thread().is_null() && parent_task == current_task() {
            fpu::fpinherit(current_thread(), thread);
        }
    }
}

/// `pcb_terminate()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `thread` must be a live thread that will not run again.
pub(crate) unsafe fn pcb_terminate(thread: *mut Thread) {
    // SAFETY: the caller promises a live thread; its pcb came from
    // `pcb_init()`.
    let pcb = unsafe { (*thread).pcb };

    // SAFETY: the save area and LDT belong to this pcb alone, and the caller
    // gives the thread up.
    unsafe {
        if !(*pcb).ims.ifps.is_null() {
            fpu::free_fp_state((*pcb).ims.ifps);
        }
        if !(*pcb).ims.ldt.is_null() {
            user_ldt::free((*pcb).ims.ldt);
        }
        if let Some(buf) = NonNull::new(pcb.cast::<u8>()) {
            (*ptr::addr_of_mut!(PCB_CACHE)).free(buf);
        }
        (*thread).pcb = ptr::null_mut();
    }
}

/// `thread_setstatus()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// `thread` must point at a live thread, and `tstate` must be readable for
/// `count` words of the record `flavor` names.
pub(crate) unsafe fn thread_setstatus(
    thread: *mut Thread,
    flavor: c_int,
    tstate: *mut c_uint,
    count: c_uint,
) -> Result<(), KernError> {
    match flavor {
        I386_THREAD_STATE | I386_REGS_SEGS_STATE => {
            if count < I386_THREAD_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            let state = tstate.cast::<I386ThreadState>();
            // SAFETY: the caller promises a live thread and the state
            // record, which is `i386_THREAD_STATE_COUNT` words.
            let saved_state = unsafe { &mut (*(*thread).pcb).iss };

            if flavor == I386_REGS_SEGS_STATE {
                // SAFETY: as above; only the low 16 bits of the selectors
                // are valid.
                unsafe {
                    (*state).cs &= 0xffff;
                    (*state).ss &= 0xffff;
                    #[cfg(target_pointer_width = "32")]
                    {
                        (*state).ds &= 0xffff;
                        (*state).es &= 0xffff;
                        (*state).fs &= 0xffff;
                        (*state).gs &= 0xffff;
                    }

                    if (*state).cs == 0
                        || ((*state).cs & SEL_PL) != SEL_PL_U
                        || (*state).ss == 0
                        || ((*state).ss & SEL_PL) != SEL_PL_U
                    {
                        return Err(KernError::InvalidArgument);
                    }
                }
            }

            #[cfg(target_pointer_width = "64")]
            // SAFETY: the state record and the saved state are live.
            unsafe {
                saved_state.r8 = (*state).r8;
                saved_state.r9 = (*state).r9;
                saved_state.r10 = (*state).r10;
                saved_state.r11 = (*state).r11;
                saved_state.r12 = (*state).r12;
                saved_state.r13 = (*state).r13;
                saved_state.r14 = (*state).r14;
                saved_state.r15 = (*state).r15;
                saved_state.edi = (*state).rdi;
                saved_state.esi = (*state).rsi;
                saved_state.ebp = (*state).rbp;
                saved_state.uesp = (*state).ursp;
                saved_state.ebx = (*state).rbx;
                saved_state.edx = (*state).rdx;
                saved_state.ecx = (*state).rcx;
                saved_state.eax = (*state).rax;
                saved_state.eip = (*state).rip;
                saved_state.efl =
                    ((*state).rfl & !EFL_USER_CLEAR) | EFL_USER_SET;
            }
            #[cfg(target_pointer_width = "32")]
            // SAFETY: as above.
            unsafe {
                saved_state.edi = c_ulong::from((*state).edi);
                saved_state.esi = c_ulong::from((*state).esi);
                saved_state.ebp = c_ulong::from((*state).ebp);
                saved_state.uesp = c_ulong::from((*state).uesp);
                saved_state.ebx = c_ulong::from((*state).ebx);
                saved_state.edx = c_ulong::from((*state).edx);
                saved_state.ecx = c_ulong::from((*state).ecx);
                saved_state.eax = c_ulong::from((*state).eax);
                saved_state.eip = c_ulong::from((*state).eip);
                saved_state.efl = (c_ulong::from((*state).efl)
                    & !EFL_USER_CLEAR)
                    | EFL_USER_SET;
            }

            #[cfg(target_pointer_width = "32")]
            // SAFETY: as above; the pcb's v86 state is live.
            unsafe {
                if saved_state.efl & EFL_VM != 0 {
                    saved_state.cs = c_ulong::from((*state).cs & 0xffff);
                    saved_state.ss = c_ulong::from((*state).ss & 0xffff);
                    saved_state.v86_segs.v86_ds =
                        c_ulong::from((*state).ds & 0xffff);
                    saved_state.v86_segs.v86_es =
                        c_ulong::from((*state).es & 0xffff);
                    saved_state.v86_segs.v86_fs =
                        c_ulong::from((*state).fs & 0xffff);
                    saved_state.v86_segs.v86_gs =
                        c_ulong::from((*state).gs & 0xffff);

                    saved_state.ds = 0;
                    saved_state.es = 0;
                    saved_state.fs = 0;
                    saved_state.gs = 0;

                    if (*(*thread).pcb).ims.v86s.int_table != 0 {
                        (*(*thread).pcb).ims.v86s.flags =
                            (saved_state.efl & (EFL_TF | EFL_IF)) as c_ushort;
                    }
                } else if flavor == I386_THREAD_STATE {
                    saved_state.cs = USER_CS;
                    saved_state.ss = USER_DS;
                    saved_state.ds = USER_DS;
                    saved_state.es = USER_DS;
                    saved_state.fs = USER_DS;
                    saved_state.gs = USER_DS;
                } else {
                    saved_state.cs = c_ulong::from((*state).cs);
                    saved_state.ss = c_ulong::from((*state).ss);
                    saved_state.ds = c_ulong::from((*state).ds);
                    saved_state.es = c_ulong::from((*state).es);
                    saved_state.fs = c_ulong::from((*state).fs);
                    saved_state.gs = c_ulong::from((*state).gs);
                }
            }
            Ok(())
        }

        fpu::I386_FLOAT_STATE => {
            if count < I386_FLOAT_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record.
            unsafe {
                fpu::fpu_set_state(thread, tstate.cast::<c_void>(), flavor)
            }
        }

        fpu::I386_XFLOAT_STATE => {
            let mut xfp_size: VmSize = 0;
            // SAFETY: the host is the live `realhost`, and `xfp_size` is a
            // local.
            unsafe {
                fpu::i386_get_xstate_size(
                    realhost().cast::<c_void>(),
                    &mut xfp_size,
                )?
            };
            xfp_size /= size_of::<c_int>();
            // `c_uint` is 32 bits and `usize` is at least that wide here.
            if (count as usize) < xfp_size {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record, whose size the
            // check above confirmed.
            unsafe {
                fpu::fpu_set_state(thread, tstate.cast::<c_void>(), flavor)
            }
        }

        I386_ISA_PORT_MAP_STATE => {
            if count < I386_ISA_PORT_MAP_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            Ok(())
        }

        #[cfg(target_pointer_width = "32")]
        I386_V86_ASSIST_STATE => {
            if count < I386_V86_ASSIST_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record.
            let state = tstate.cast::<I386V86AssistState>();
            // SAFETY: as above; the record is two words.
            unsafe {
                let int_table = (*state).int_table;
                let int_count = (*state).int_count;

                if int_table >= VM_MAX_USER_ADDRESS
                    || int_table
                        + usize::from(int_count)
                            * size_of::<V86InterruptTable>()
                        > VM_MAX_USER_ADDRESS
                {
                    return Err(KernError::InvalidArgument);
                }

                (*(*thread).pcb).ims.v86s.int_table = int_table;
                (*(*thread).pcb).ims.v86s.int_count = int_count;
                (*(*thread).pcb).ims.v86s.flags =
                    ((*(*thread).pcb).iss.efl & (EFL_TF | EFL_IF)) as c_ushort;
            }
            Ok(())
        }

        I386_DEBUG_STATE => {
            if count < I386_DEBUG_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            unsafe {
                db_interface::set_debug_state(
                    (*thread).pcb,
                    tstate.cast::<I386DebugState>(),
                )
            }
        }

        #[cfg(target_pointer_width = "64")]
        I386_FSGS_BASE_STATE => {
            if count < I386_FSGS_BASE_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            unsafe {
                let state = tstate.cast::<I386FsgsBaseState>();
                if (*state).gs_base & 0x8000_0000_0000_0000 != 0 {
                    glue::printf(
                        c"WARNING: negative gs base not allowed\n".as_ptr(),
                    );
                }
                (*(*thread).pcb).ims.sbs.fsbase = (*state).fs_base;
                (*(*thread).pcb).ims.sbs.gsbase =
                    (*state).gs_base & 0x7fff_ffff_ffff_ffff;
                if thread == current_thread() {
                    write_msr(MSR_REG_FSBASE, (*state).fs_base);
                    write_msr(MSR_REG_KGSBASE, (*state).gs_base);
                }
            }
            Ok(())
        }

        _ => Err(KernError::InvalidArgument),
    }
}

/// `struct v86_interrupt_table` of <machine/thread_status.h>: one 8086
/// interrupt's pending state.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V86InterruptTable {
    pub count: c_uint,
    pub mask: c_ushort,
    pub vec: c_ushort,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<V86InterruptTable>() == 8);
    assert!(align_of::<V86InterruptTable>() == align_of::<c_uint>());
    assert!(offset_of!(V86InterruptTable, count) == 0);
    assert!(offset_of!(V86InterruptTable, mask) == 4);
    assert!(offset_of!(V86InterruptTable, vec) == 6);
};

/// `thread_getstatus()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
///
/// # Safety
///
/// `thread` must point at a live thread, `tstate` must be writable for
/// `count` words of the record `flavor` names, and `count` must be valid for
/// a read and a write.
pub(crate) unsafe fn thread_getstatus(
    thread: *mut Thread,
    flavor: c_int,
    tstate: *mut c_uint,
    count: *mut c_uint,
) -> Result<(), KernError> {
    // SAFETY: the caller promises a live thread and a readable count.
    let requested = unsafe { *count };

    match flavor {
        THREAD_STATE_FLAVOR_LIST => {
            #[cfg(target_pointer_width = "64")]
            let ncount: c_uint = 3;
            #[cfg(target_pointer_width = "32")]
            let ncount: c_uint = 4;
            if requested < ncount {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the check above leaves four writable words.
            unsafe {
                *tstate.add(0) = I386_THREAD_STATE as c_uint;
                *tstate.add(1) = fpu::I386_FLOAT_STATE as c_uint;
                *tstate.add(2) = I386_ISA_PORT_MAP_STATE as c_uint;
                #[cfg(target_pointer_width = "32")]
                {
                    *tstate.add(3) = I386_V86_ASSIST_STATE as c_uint;
                }
                *count = ncount;
            }
            Ok(())
        }

        I386_THREAD_STATE | I386_REGS_SEGS_STATE => {
            if requested < I386_THREAD_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            let (state, saved_state) = unsafe {
                (tstate.cast::<I386ThreadState>(), &mut (*(*thread).pcb).iss)
            };

            #[cfg(target_pointer_width = "64")]
            // SAFETY: both records are live.
            unsafe {
                (*state).r8 = saved_state.r8;
                (*state).r9 = saved_state.r9;
                (*state).r10 = saved_state.r10;
                (*state).r11 = saved_state.r11;
                (*state).r12 = saved_state.r12;
                (*state).r13 = saved_state.r13;
                (*state).r14 = saved_state.r14;
                (*state).r15 = saved_state.r15;
                (*state).rdi = saved_state.edi;
                (*state).rsi = saved_state.esi;
                (*state).rbp = saved_state.ebp;
                (*state).rbx = saved_state.ebx;
                (*state).rdx = saved_state.edx;
                (*state).rcx = saved_state.ecx;
                (*state).rax = saved_state.eax;
                (*state).rip = saved_state.eip;
                (*state).ursp = saved_state.uesp;
                (*state).rfl = saved_state.efl;
                (*state).rsp = 0;
            }
            #[cfg(target_pointer_width = "32")]
            // SAFETY: as above.
            unsafe {
                (*state).edi = saved_state.edi as c_uint;
                (*state).esi = saved_state.esi as c_uint;
                (*state).ebp = saved_state.ebp as c_uint;
                (*state).ebx = saved_state.ebx as c_uint;
                (*state).edx = saved_state.edx as c_uint;
                (*state).ecx = saved_state.ecx as c_uint;
                (*state).eax = saved_state.eax as c_uint;
                (*state).eip = saved_state.eip as c_uint;
                (*state).uesp = saved_state.uesp as c_uint;
                (*state).efl = saved_state.efl as c_uint;
                (*state).esp = 0;
            }

            // SAFETY: as above.
            unsafe {
                (*state).cs = saved_state.cs as c_uint;
                (*state).ss = saved_state.ss as c_uint;
            }

            #[cfg(target_pointer_width = "32")]
            // SAFETY: as above; the pcb's v86 state is live.
            unsafe {
                if saved_state.efl & EFL_VM != 0 {
                    (*state).ds =
                        (saved_state.v86_segs.v86_ds & 0xffff) as c_uint;
                    (*state).es =
                        (saved_state.v86_segs.v86_es & 0xffff) as c_uint;
                    (*state).fs =
                        (saved_state.v86_segs.v86_fs & 0xffff) as c_uint;
                    (*state).gs =
                        (saved_state.v86_segs.v86_gs & 0xffff) as c_uint;

                    if (*(*thread).pcb).ims.v86s.int_table != 0
                        && (*(*thread).pcb).ims.v86s.flags
                            & (EFL_IF as c_ushort | V86_IF_PENDING)
                            == 0
                    {
                        saved_state.efl &= !EFL_IF;
                    }
                } else {
                    (*state).ds = (saved_state.ds & 0xffff) as c_uint;
                    (*state).es = (saved_state.es & 0xffff) as c_uint;
                    (*state).fs = (saved_state.fs & 0xffff) as c_uint;
                    (*state).gs = (saved_state.gs & 0xffff) as c_uint;
                }
                *count = I386_THREAD_STATE_COUNT;
            }
            #[cfg(target_pointer_width = "64")]
            // SAFETY: the caller's count pointer is writable.
            unsafe {
                *count = I386_THREAD_STATE_COUNT;
            }
            Ok(())
        }

        fpu::I386_FLOAT_STATE => {
            if requested < I386_FLOAT_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the writable record and count.
            unsafe {
                *count = I386_FLOAT_STATE_COUNT;
                fpu::fpu_get_state(thread, tstate.cast::<c_void>(), flavor)
            }
        }

        fpu::I386_XFLOAT_STATE => {
            let mut xfp_size: VmSize = 0;
            // SAFETY: the host is the live `realhost` and `xfp_size` a local.
            unsafe {
                fpu::i386_get_xstate_size(
                    realhost().cast::<c_void>(),
                    &mut xfp_size,
                )?
            };
            xfp_size /= size_of::<c_int>();
            // `c_uint` is 32 bits and `usize` is at least that wide here.
            if (requested as usize) < xfp_size {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the writable record and count.
            unsafe {
                *count = xfp_size as c_uint;
                fpu::fpu_get_state(thread, tstate.cast::<c_void>(), flavor)
            }
        }

        I386_ISA_PORT_MAP_STATE => {
            if requested < I386_ISA_PORT_MAP_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live task
            // with a live `iopb_lock`.
            unsafe {
                let state = tstate.cast::<I386IsaPortMapState>();
                let task = (*thread).task;
                (*task).machine.iopb_lock.lock();
                if (*task).machine.iopb.is_null() {
                    ptr::write_bytes(
                        (*state).pm.as_mut_ptr(),
                        0xff,
                        0x400 >> 3,
                    );
                } else {
                    ptr::copy_nonoverlapping(
                        (*task).machine.iopb,
                        (*state).pm.as_mut_ptr(),
                        0x400 >> 3,
                    );
                }
                (*task).machine.iopb_lock.unlock();
                *count = I386_ISA_PORT_MAP_STATE_COUNT;
            }
            Ok(())
        }

        #[cfg(target_pointer_width = "32")]
        I386_V86_ASSIST_STATE => {
            if requested < I386_V86_ASSIST_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            unsafe {
                let state = tstate.cast::<I386V86AssistState>();
                (*state).int_table = (*(*thread).pcb).ims.v86s.int_table;
                (*state).int_count = (*(*thread).pcb).ims.v86s.int_count;
                *count = I386_V86_ASSIST_STATE_COUNT;
            }
            Ok(())
        }

        I386_DEBUG_STATE => {
            if requested < I386_DEBUG_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            unsafe {
                db_interface::get_debug_state(
                    (*thread).pcb,
                    tstate.cast::<I386DebugState>(),
                );
                *count = I386_DEBUG_STATE_COUNT;
            }
            Ok(())
        }

        #[cfg(target_pointer_width = "64")]
        I386_FSGS_BASE_STATE => {
            if requested < I386_FSGS_BASE_STATE_COUNT {
                return Err(KernError::InvalidArgument);
            }
            // SAFETY: the caller promises the state record and a live pcb.
            unsafe {
                let state = tstate.cast::<I386FsgsBaseState>();
                (*state).fs_base = (*(*thread).pcb).ims.sbs.fsbase;
                (*state).gs_base = (*(*thread).pcb).ims.sbs.gsbase;
                *count = I386_FSGS_BASE_STATE_COUNT;
            }
            Ok(())
        }

        _ => Err(KernError::InvalidArgument),
    }
}

/// `thread_set_syscall_return()` of `i386/i386/pcb.h`, which
/// `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `thread` must point at a live thread.
pub(crate) unsafe fn thread_set_syscall_return(
    thread: *mut Thread,
    retval: c_int,
) {
    // SAFETY: the caller promises a live thread; the C stored the return
    // value in the saved `eax`.
    unsafe { (*(*thread).pcb).iss.eax = retval as c_ulong };
}

/// `user_stack_low()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c`
/// defined.
pub(crate) fn user_stack_low(stack_size: VmSize) -> VmOffset {
    VM_MAX_USER_ADDRESS.wrapping_sub(stack_size)
}

/// `set_user_regs()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// Runs on the current thread, whose pcb is live, and `exec_info` points at
/// a live record.
pub(crate) unsafe fn set_user_regs(
    stack_base: VmOffset,
    stack_size: VmOffset,
    exec_info: *const ExecInfo,
    arg_size: VmSize,
) -> VmOffset {
    let arg_size =
        arg_size.wrapping_add(USER_STACK_ALIGN - 1) & !(USER_STACK_ALIGN - 1);
    let arg_addr = stack_base + stack_size - arg_size;

    // SAFETY: the caller runs on the current thread, and `exec_info` is a
    // live record.
    unsafe {
        let saved_state = &mut (*(*current_thread()).pcb).iss;
        saved_state.uesp = arg_addr as c_ulong;
        saved_state.eip = (*exec_info).entry as c_ulong;
    }

    arg_addr
}

/// `stack_detach()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must own its
/// `kernel_stack` field for the duration: `stack_free()` calls this at
/// splsched with the thread locked.
pub(crate) unsafe fn stack_detach(thread: *mut Thread) -> VmOffset {
    // SAFETY: the caller promises a live thread that no other CPU is
    // detaching from.
    unsafe { core::mem::replace(&mut (*thread).kernel_stack, 0) }
}

/// `load_context()` of `i386/i386/pcb.h`, which `i386/i386/pcb.c` defined.
///
/// # Safety
///
/// `new` must point at a live thread whose kernel stack and saved context
/// are ready to resume, and no other CPU may be running it.
pub(crate) unsafe fn load_context(new: *mut Thread) -> ! {
    // SAFETY: the caller promises a live thread whose pcb `pcb_init()`
    // built.
    let pcb = unsafe { (*new).pcb };
    // SAFETY: the caller promises a resumable thread.
    unsafe { switch_ktss(pcb) };
    // SAFETY: the caller promises a resumable thread.
    unsafe { glue::Load_context(new) }
}
