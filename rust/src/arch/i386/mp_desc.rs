// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386/mp_desc.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Derived from i386/i386/mp_desc.h:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt stacks and the per-processor descriptor tables, which
//! `i386/i386/mp_desc.c` used to define and `i386/i386/mp_desc.h` declares.
//!
//! The `extern "C"` edge is in [`mp_desc_ffi`].

use crate::arch::i386::apic;
use crate::arch::i386::fpu;
use crate::arch::i386::model_dep;
use crate::arch::i386::pcb::{RealDescriptor, TaskTss};
use crate::arch::i386::percpu::cpu_number;
use crate::arch::i386::{gdt, idt, int_init, ktss, ldt, pmap, smp};
use crate::arch::types::VmOffset;
use crate::config::NCPUS;
use crate::glue;
use crate::kern::smp as kern_smp;
use crate::kern::types::KernError;
use core::arch::asm;
use core::ffi::{CStr, c_int, c_uint, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

/// The number of iterations [`simple_lock_pause`] spins, which the C kept in
/// the global `simple_lock_pause_loop`.
const PAUSE_LOOP: u32 = 100;

/// The count [`simple_lock_pause`] adds one to per call, which the C kept in
/// the global `simple_lock_pause_count`.
static PAUSE_COUNT: AtomicU32 = AtomicU32::new(0);

/// The counter the pause loop increments, which the C kept in a function-local
/// `static volatile int`.
static PAUSE_DUMMY: AtomicU32 = AtomicU32::new(0);

/// `APIC_LOGICAL_CPU_GROUPS` in <i386/apic.h>: the logical destination
/// register has only eight mask bits, so it can name eight CPU groups.
const APIC_LOGICAL_CPU_GROUPS: c_int = 8;

/// `INTSTACK_SIZE` of <i386/vm_param.h>: `I386_PGBYTES`, one page per
/// interrupt stack.
const INTSTACK_SIZE: usize = 4096;

const _: () = assert!(INTSTACK_SIZE == 4096);

/// `IDTSZ` of <i386at/idt.h>.
pub(crate) const IDTSZ: usize = 0x100;

/// `GDTSZ` of <i386/gdt.h>: `sel_idx(0x70)`, the eight-byte descriptors up to
/// the per-CPU segment.
pub(crate) const GDTSZ: usize = 14;

/// `LDTSZ` of <i386/ldt.h>.
const LDTSZ: usize = 4;

/// `struct real_gate` of <i386/seg.h>, its bitfields kept as two words; every
/// field packs into the low half on the 32-bit layout.
#[cfg(target_pointer_width = "32")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RealGate {
    /// `offset_low:16` followed by `selector:16`.
    pub offset_low_selector: u32,
    /// `word_count:8`, `access:8` and `offset_high:16`.
    pub word_count_access_offset_high: u32,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<RealGate>() == 8);
    assert!(align_of::<RealGate>() == align_of::<u32>());
    assert!(offset_of!(RealGate, offset_low_selector) == 0);
    assert!(offset_of!(RealGate, word_count_access_offset_high) == 4);
};

#[cfg(target_pointer_width = "32")]
impl RealGate {
    /// An all-zero gate, the image a C `static` began with.
    pub(crate) const ZERO: Self = Self {
        offset_low_selector: 0,
        word_count_access_offset_high: 0,
    };
}

/// `struct real_gate` of <i386/seg.h>, the 64-bit layout: the same two words
/// followed by the offset extension and its reserved word.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RealGate {
    /// `offset_low:16` followed by `selector:16`.
    pub offset_low_selector: u32,
    /// `word_count:8`, `access:8` and `offset_high:16`.
    pub word_count_access_offset_high: u32,
    pub offset_ext: u32,
    pub reserved: u32,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<RealGate>() == 16);
    assert!(align_of::<RealGate>() == align_of::<u32>());
    assert!(offset_of!(RealGate, offset_low_selector) == 0);
    assert!(offset_of!(RealGate, word_count_access_offset_high) == 4);
    assert!(offset_of!(RealGate, offset_ext) == 8);
    assert!(offset_of!(RealGate, reserved) == 12);
};

#[cfg(target_pointer_width = "64")]
impl RealGate {
    /// An all-zero gate, the image a C `static` began with.
    pub(crate) const ZERO: Self = Self {
        offset_low_selector: 0,
        word_count_access_offset_high: 0,
        offset_ext: 0,
        reserved: 0,
    };
}

/// `struct mp_desc_table` of <i386/mp_desc.h>: one CPU's descriptor tables,
/// which the `gdt`, `idt`, `ktss` and `ldt` modules fill.
///
/// The sizes, offsets and alignment below were read from both built kernels'
/// debug information.
#[repr(C)]
pub struct MpDescTable {
    pub idt: [RealGate; IDTSZ],
    pub gdt: [RealDescriptor; GDTSZ],
    pub ldt: [RealDescriptor; LDTSZ],
    pub ktss: TaskTss,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<MpDescTable>() == 10492);
    assert!(align_of::<MpDescTable>() == align_of::<u32>());
    assert!(offset_of!(MpDescTable, idt) == 0);
    assert!(offset_of!(MpDescTable, gdt) == 2048);
    assert!(offset_of!(MpDescTable, ldt) == 2160);
    assert!(offset_of!(MpDescTable, ktss) == 2192);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<MpDescTable>() == 12540);
    assert!(align_of::<MpDescTable>() == align_of::<u32>());
    assert!(offset_of!(MpDescTable, idt) == 0);
    assert!(offset_of!(MpDescTable, gdt) == 4096);
    assert!(offset_of!(MpDescTable, ldt) == 4208);
    assert!(offset_of!(MpDescTable, ktss) == 4240);
};

/// The interrupt stacks, which `boothdr.S` starts the boot CPU on and the
/// interrupt entry points switch to.
#[repr(C, align(4096))]
pub(crate) struct IntStacks(pub(crate) [u8; NCPUS * INTSTACK_SIZE]);

/// `solid_intstack` of `i386/i386/mp_desc.c`.
#[unsafe(no_mangle)]
pub(crate) static mut solid_intstack: IntStacks =
    IntStacks([0; NCPUS * INTSTACK_SIZE]);

/// `int_stack_base` of <i386at/model_dep.h>: one stack bottom per CPU.
#[unsafe(no_mangle)]
pub static mut int_stack_base: [VmOffset; NCPUS] = [0; NCPUS];

/// `int_stack_top` of <i386at/model_dep.h>: one stack top per CPU.
#[unsafe(no_mangle)]
pub static mut int_stack_top: [VmOffset; NCPUS] = [0; NCPUS];

/// `apboot_addr` of <i386/model_dep.h>: the physical page the AP boot code
/// was copied to.
#[unsafe(no_mangle)]
pub static mut apboot_addr: VmOffset = 0;

/// `mp_desc_table` of <i386/mp_desc.h>: one allocated table set per CPU other
/// than the boot CPU, which shares the `gdt.c`/`ktss.c` tables.
#[unsafe(no_mangle)]
pub static mut mp_desc_table: [*mut MpDescTable; NCPUS] =
    [ptr::null_mut(); NCPUS];

/// `mp_ktss` of <i386/mp_desc.h>: the TSS of each CPU.
#[unsafe(no_mangle)]
pub static mut mp_ktss: [*mut TaskTss; NCPUS] = [ptr::null_mut(); NCPUS];

/// `mp_gdt` of <i386/mp_desc.h>: the GDT of each CPU.
#[unsafe(no_mangle)]
pub static mut mp_gdt: [*mut RealDescriptor; NCPUS] = [ptr::null_mut(); NCPUS];

/// `phystokv()` of <i386/vm_param.h>.
const fn phystokv(pa: VmOffset) -> VmOffset {
    pa.wrapping_add(crate::vm::vm_kern::VM_MIN_KERNEL_ADDRESS)
}

/// `flush_instr_queue()` of <i386/proc_reg.h>: the jump that discards the
/// instructions the processor prefetched before a control-register change.
pub(crate) fn flush_instr_queue() {
    // SAFETY: the jump changes no machine state, its label is local to the
    // block, and it neither reads nor writes memory.
    unsafe { asm!("jmp 2f", "2:", options(nostack, nomem, preserves_flags)) };
}

/// Wait a bit for a lock another CPU holds in the opposite order, which
/// `kern/lock.h` declares.
pub(crate) fn simple_lock_pause() {
    PAUSE_COUNT.fetch_add(1, Ordering::Relaxed);
    for _ in 0..PAUSE_LOOP {
        // Nothing is published and no one reads `PAUSE_DUMMY`, so the ordering
        // is `Relaxed`; the increment itself is the delay, and a relaxed
        // atomic keeps the spin from being optimized away.
        PAUSE_DUMMY.fetch_add(1, Ordering::Relaxed);
    }
}

/// The machine-dependent processor control hook, which
/// `i386/i386/mp_desc.h` declares.
///
/// # Safety
///
/// `info` must be valid for `count` reads.
pub(crate) unsafe fn cpu_control(
    cpu: c_int,
    info: *const c_int,
    count: c_uint,
) -> c_int {
    // SAFETY: `printf` receives the same three values the C passed.
    unsafe {
        glue::printf(
            c"cpu_control(%d, %p, %d) not implemented\n".as_ptr(),
            cpu,
            info,
            count,
        );
    }
    c_int::from(KernError::Failure)
}

/// The logical destination bit `APIC_LOGICAL_ID(cpu)` in <i386/apic.h>
/// computes, `1u << ((cpu) % APIC_LOGICAL_CPU_GROUPS)`.
fn logical_id(cpu: c_int) -> u32 {
    let group = (cpu as u32) % (APIC_LOGICAL_CPU_GROUPS as u32);
    1u32 << group
}

/// Interrupt processor `cpu` to make it flush its pmap.
pub(crate) fn interrupt_processor(cpu: c_int) {
    // The local APIC is initialized before any processor runs, and
    // `logical_id` is the APIC destination bit the C macro computes.
    smp::pmap_update(logical_id(cpu));
}

/// `interrupt_stack_alloc()` of <i386/mp_desc.h>.
pub(crate) fn interrupt_stack_alloc() {
    // SAFETY: `interrupt_stack_alloc` runs before any other CPU, and it is
    // the first reader or writer of the stacks.
    let stacks = unsafe { ptr::addr_of_mut!(solid_intstack.0) }.cast::<u8>();
    for i in 0..NCPUS {
        let base = stacks.wrapping_add(i * INTSTACK_SIZE);
        let top = stacks.wrapping_add((i + 1) * INTSTACK_SIZE).wrapping_sub(4);
        // SAFETY: `interrupt_stack_alloc` runs once from `i386at_init`,
        // before any interrupt stack is used, and `i` is below `NCPUS`.
        unsafe {
            int_stack_base[i] = base.addr();
            int_stack_top[i] = top.addr();
        }
    }
}

/// `mp_desc_init()` of <i386/mp_desc.h>.
pub(crate) fn mp_desc_init(mycpu: c_int) -> c_int {
    if mycpu == 0 {
        // SAFETY: the boot CPU uses the tables `gdt.rs` and `ktss.rs` built,
        // and `mp_desc_init` runs on each CPU only once.
        unsafe {
            mp_ktss[0] = ptr::addr_of_mut!(ktss::ktss);
            mp_gdt[0] = ptr::addr_of_mut!(gdt::gdt).cast::<RealDescriptor>();
        }
        return 0;
    }

    let Some(mem) = model_dep::alloc_aligned(size_of::<MpDescTable>()) else {
        // SAFETY: `Panic` does not return, and the message holds no
        // conversion specifier for the varargs it never receives.
        unsafe {
            glue::Panic(
                c"i386/i386/mp_desc.c".as_ptr(),
                line!() as c_int,
                c"mp_desc_init".as_ptr(),
                c"not enough memory for descriptor tables".as_ptr(),
            )
        }
    };
    let mpt = ptr::with_exposed_provenance_mut::<MpDescTable>(phystokv(mem));

    // SAFETY: `mpt` is the table set `init_alloc_aligned` just took from the
    // boot allocator, and `mycpu` is the CPU this call initializes.
    unsafe {
        mp_desc_table[mycpu as usize] = mpt;
        mp_ktss[mycpu as usize] = ptr::addr_of_mut!((*mpt).ktss);
        mp_gdt[mycpu as usize] = (*mpt).gdt.as_mut_ptr();

        ptr::write_bytes(
            ptr::addr_of_mut!((*mpt).idt).cast::<u8>(),
            0,
            size_of::<[RealGate; IDTSZ]>(),
        );
        ptr::write_bytes(
            (*mpt).gdt.as_mut_ptr().cast::<u8>(),
            0,
            size_of::<[RealDescriptor; GDTSZ]>(),
        );
        ptr::write_bytes(
            (*mpt).ldt.as_mut_ptr().cast::<u8>(),
            0,
            size_of::<[RealDescriptor; LDTSZ]>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!((*mpt).ktss).cast::<u8>(),
            0,
            size_of::<TaskTss>(),
        );
    }

    mycpu
}

/// `paging_enable()` in `i386/i386/mp_desc.c`.  The C's `CR0_WP` is left off,
/// as its own comment asked.
fn paging_enable() {
    #[cfg(target_pointer_width = "64")]
    fpu::write_cr4(fpu::read_cr4() | fpu::CR4_PAE);
    fpu::write_cr0(fpu::read_cr0() | fpu::CR0_PG);
    fpu::write_cr0(fpu::read_cr0() & !(fpu::CR0_CD | fpu::CR0_NW));
    if pmap::cpu_has_feature(pmap::CPU_FEATURE_PGE) {
        fpu::write_cr4(fpu::read_cr4() | fpu::CR4_PGE);
    }
}

/// The boot message of one stage of [`cpu_setup`], which the C spelled as
/// `printf("AP=(%u) <stage> done\n", cpu)`.
fn ap_stage(cpu: c_int, stage: &CStr) {
    // SAFETY: the `%u` takes the `c_uint` and the `%s` the pointer.
    unsafe {
        glue::printf(
            c"AP=(%u) %s done\n".as_ptr(),
            cpu as c_uint,
            stage.as_ptr(),
        )
    };
}

/// `cpu_setup()` in `i386/i386/mp_desc.c`, the boot path of an AP.
fn cpu_setup(cpu: c_int) -> ! {
    pmap::pmap_set_page_dir();
    ap_stage(cpu, c"pagedir");

    paging_enable();
    flush_instr_queue();
    ap_stage(cpu, c"paging");

    // SAFETY: `init_percpu` is the real C routine of `i386/i386/percpu.c`,
    // and `cpu` is a CPU the machine reported.
    unsafe { glue::init_percpu(cpu) };
    mp_desc_init(cpu);
    ap_stage(cpu, c"mpdesc");

    // The AP runs one CPU's copy of each descriptor table.
    gdt::ap_gdt_init(cpu);
    ap_stage(cpu, c"gdt");
    idt::ap_idt_init(cpu);
    ap_stage(cpu, c"idt");
    int_init::ap_int_init(cpu);
    ap_stage(cpu, c"int");
    ldt::ap_ldt_init(cpu);
    ap_stage(cpu, c"ldt");
    ktss::ap_ktss_init(cpu);
    ap_stage(cpu, c"ktss");

    // SAFETY: the slot is this CPU's to fill while the BSP's type is already
    // recorded.
    unsafe {
        let slot = crate::kern::machine::slot(cpu as usize);
        (*slot).cpu_subtype = CPU_SUBTYPE_AT386;
        (*slot).cpu_type = (*crate::kern::machine::slot(0)).cpu_type;
    }
    // SAFETY: `init_fpu` is the real C routine of `i386/i386/fpu.c`.
    unsafe { crate::arch::i386::fpu_ffi::init_fpu() };
    apic::lapic_setup();
    apic::lapic_enable();
    // SAFETY: `cpu_launch_first_thread` is the real C routine of
    // `kern/startup.c`, and it never returns.
    unsafe { glue::cpu_launch_first_thread(ptr::null_mut()) }
}

/// `cpu_ap_main()` of <i386/mp_desc.h>, the entry `cpuboot.S` calls.
pub(crate) fn cpu_ap_main() -> ! {
    cpu_setup(cpu_number())
}

/// `CPU_SUBTYPE_AT386` of <mach/machine.h>.
const CPU_SUBTYPE_AT386: c_int = 1;

/// Copy the AP boot code to the page `biosmem_bootstrap()` claimed.
fn copy_apboot() {
    let begin = ptr::addr_of!(glue::apboot).addr();
    let length = ptr::addr_of!(glue::apbootend).addr() - begin;
    // SAFETY: `apboot_addr` names the page `biosmem_bootstrap()` reserved for
    // this copy, `apboot`/`apbootend` bracket the image, and this runs on one
    // CPU before any AP starts.
    unsafe {
        #[cfg(target_pointer_width = "32")]
        let source = ptr::addr_of!(glue::apboot).cast::<c_void>();
        #[cfg(target_pointer_width = "64")]
        let source = ptr::with_exposed_provenance::<c_void>(phystokv(begin));
        crate::utils::string::memcpy(
            ptr::with_exposed_provenance_mut::<c_void>(phystokv(apboot_addr)),
            source,
            length,
        );
    }
}

/// `start_other_cpus()` of <i386/mp_desc.h>.
pub(crate) fn start_other_cpus() {
    let mut ncpus = c_int::from(kern_smp::smp_get_numcpus());
    if ncpus == 1 {
        return;
    }

    copy_apboot();

    // SAFETY: `splhigh()` is the real asm function <i386/spl.h> declares, and
    // nothing restores the level because the BSP stays at it afterwards.
    unsafe { glue::splhigh() };

    apic::lapic_disable();
    pmap::pmap_make_temporary_mapping();

    for cpu in 1..ncpus {
        // SAFETY: `cpu` is below the probed CPU count and the slot array has
        // `NCPUS` entries the probe never exceeds.
        unsafe { (*crate::kern::machine::slot(cpu as usize)).running = 0 };
    }

    // SAFETY: `apboot_addr` is the physical page `copy_apboot` filled.
    let bsp = apic::apic_get_current_cpu();
    smp::startup_cpus(bsp as c_uint, unsafe { apboot_addr } as c_ulong);

    for cpu in 1..ncpus {
        // SAFETY: the `%d` takes the matching `c_int`.
        unsafe {
            glue::printf(c"Waiting for AP %d\n".as_ptr(), cpu);
        }

        loop {
            // SAFETY: `cpu` is below the probed CPU count.
            let running =
                unsafe { (*crate::kern::machine::slot(cpu as usize)).running };
            if running != 0 {
                break;
            }
            smp::pause();
        }
    }
    // SAFETY: the message holds no conversion specifier.
    unsafe { glue::printf(c"BSP: Completed SMP init\n".as_ptr()) };

    pmap::pmap_remove_temporary_mapping();

    ncpus = ncpus.min(APIC_LOGICAL_CPU_GROUPS);
    for cpu in 1..ncpus {
        interrupt_processor(cpu);
    }

    apic::lapic_enable();
}
