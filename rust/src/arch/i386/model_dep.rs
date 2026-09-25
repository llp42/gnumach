// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from i386/i386at/model_dep.c:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University.
//   Copyright (c) 1986 Avadis Tevanian, Jr., Michael Wayne Young.
// Derived from i386/i386at/model_dep.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from i386/i386/model_dep.h:
//   Copyright (C) 2008 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The machine-dependent boot and halt path of `i386/i386at/model_dep.c`:
//! the idle and relax instructions, the `/dev/time` mmap hook, the wall
//! clock, the bootstrap allocator and the boot entry points.
//!
//! The `extern "C"` edge is in [`model_dep_ffi`].

use crate::arch::i386::{
    apic, biosmem, fpu, ioapic, irq, mbinfo, mp_desc, percpu, pit, pmap, rtc,
};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_MASK, PAGE_SHIFT};
use crate::glue;
use crate::glue::time_value::TimeValue64;
use crate::kern::bootstrap::{MultibootRawInfo, MultibootRawModule};
use crate::kern::mach_clock;
use crate::vm::types::VmProt;
use crate::vm::vm_kern::VM_MIN_KERNEL_ADDRESS;
use core::arch::asm;
use core::ffi::{c_char, c_int, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr;

/// `MULTIBOOT_LOADER_CMDLINE` of <mach/machine/multiboot.h>.
const MULTIBOOT_LOADER_CMDLINE: u32 = 0x04;
/// `MULTIBOOT_LOADER_MODULES` of <mach/machine/multiboot.h>.
const MULTIBOOT_LOADER_MODULES: u32 = 0x08;
/// `MULTIBOOT_LOADER_SHDR` of <mach/machine/multiboot.h>.
const MULTIBOOT_LOADER_SHDR: u32 = 0x20;
/// `MULTIBOOT_CMDLINE` of <mach/machine/multiboot.h>.
const MULTIBOOT_CMDLINE: u32 = 0x04;
/// `MULTIBOOT_MODS` of <mach/machine/multiboot.h>.
const MULTIBOOT_MODS: u32 = 0x08;

/// `ELF_SHT_SYMTAB` of i386/i386at/elf.h.
const ELF_SHT_SYMTAB: u32 = 2;
/// `ELF_SHT_STRTAB` of i386/i386at/elf.h.
const ELF_SHT_STRTAB: u32 = 3;

/// `CPU_TYPE_I386` of <mach/machine.h>.
#[cfg(target_pointer_width = "32")]
const CPU_TYPE_I386: c_int = 7;
/// `CPU_TYPE_I486` of <mach/machine.h>.
#[cfg(target_pointer_width = "32")]
const CPU_TYPE_I486: c_int = 17;
/// `CPU_TYPE_PENTIUM` of <mach/machine.h>.
#[cfg(target_pointer_width = "32")]
const CPU_TYPE_PENTIUM: c_int = 18;
/// `CPU_TYPE_PENTIUMPRO` of <mach/machine.h>.
#[cfg(target_pointer_width = "32")]
const CPU_TYPE_PENTIUMPRO: c_int = 19;
/// `CPU_TYPE_X86_64` of <mach/machine.h>.
#[cfg(target_pointer_width = "64")]
const CPU_TYPE_X86_64: c_int = 21;
/// `CPU_SUBTYPE_AT386` of <mach/machine.h>.
const CPU_SUBTYPE_AT386: c_int = 1;

/// `boot_info` of `i386/i386at/model_dep.c`: the multiboot information block
/// the boot loader left, which `c_boot_entry` copies out of low memory.
#[unsafe(no_mangle)]
// SAFETY: `MultibootRawInfo` is all integers, for which zero is a valid bit
// pattern.
pub static mut boot_info: MultibootRawInfo = unsafe { core::mem::zeroed() };

/// `kernel_cmdline` of `i386/i386at/model_dep.c`: the boot command line, `""`
/// until `i386at_init` can copy the loader's line to safe memory.
#[unsafe(no_mangle)]
pub static mut kernel_cmdline: *mut c_char = c"".as_ptr().cast_mut();

/// `rebootflag` of `i386/i386at/model_dep.c`: set when ctrl-alt-del should
/// reboot the machine.
#[unsafe(no_mangle)]
pub static mut rebootflag: c_int = 0;

/// `struct elf_shdr` of `i386/i386at/elf.h`, the section header
/// `register_boot_data` walks; `addr` and `offset` are the C's
/// `unsigned long`.
#[repr(C)]
struct ElfShdr {
    name: u32,
    type_: u32,
    flags: u32,
    addr: VmOffset,
    offset: VmOffset,
    size: u32,
    link: u32,
    info: u32,
    addralign: u32,
    entsize: u32,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<ElfShdr>() == 40);
    assert!(align_of::<ElfShdr>() == align_of::<u32>());
    assert!(offset_of!(ElfShdr, addr) == 12);
    assert!(offset_of!(ElfShdr, size) == 20);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<ElfShdr>() == 56);
    assert!(align_of::<ElfShdr>() == align_of::<u64>());
    assert!(offset_of!(ElfShdr, addr) == 16);
    assert!(offset_of!(ElfShdr, size) == 32);
};

/// The six bytes `cpuboot.S` reserves for `gdt_descr_tmp`: `struct
/// pseudo_descriptor` up to the padding its C type adds, which the realmode
/// GDT pointer actually occupies.
#[repr(C, packed)]
pub struct GdtDescrTmp {
    pub limit: u16,
    pub linear_base: u32,
}

const _: () = {
    assert!(size_of::<GdtDescrTmp>() == 6);
    assert!(align_of::<GdtDescrTmp>() == 1);
    assert!(offset_of!(GdtDescrTmp, linear_base) == 2);
};

/// `phystokv()` of <i386/vm_param.h>.
const fn phystokv(pa: VmOffset) -> VmOffset {
    pa.wrapping_add(VM_MIN_KERNEL_ADDRESS)
}

/// The pointer-width address a 32-bit multiboot field holds; the widening is
/// lossless on both targets.
fn address(value: u32) -> VmOffset {
    value as VmOffset
}

/// The kernel pointer a direct-map address denotes.
fn kv_ptr<T>(value: VmOffset) -> *const T {
    ptr::with_exposed_provenance(value)
}

/// The writable kernel pointer a direct-map address denotes.
fn kv_ptr_mut<T>(value: VmOffset) -> *mut T {
    ptr::with_exposed_provenance_mut(value)
}

/// Halt the calling CPU until the next interrupt.
fn idle() {
    // SAFETY: `hlt` stops the CPU until an interrupt is delivered.
    unsafe { asm!("hlt", options(nostack, preserves_flags)) };
}

/// Wait a moment for another CPU.
fn relax() {
    // SAFETY: `rep; nop` is a delay of one instruction.
    unsafe { asm!("rep; nop", options(nostack, preserves_flags)) };
}

/// `machine_idle()` of <i386/i386/model_dep.h>.
pub(crate) fn machine_idle(_cpu: c_int) {
    idle();
}

/// `machine_relax()` of <i386/i386/model_dep.h>.
pub(crate) fn machine_relax() {
    relax();
}

/// The page frame holding the kernel's mapped time value, or `None` when the
/// request asks for write access.
pub(crate) fn mapped_time_page(prot: VmProt) -> Option<VmOffset> {
    if prot.contains(VmProt::WRITE) {
        return None;
    }

    // SAFETY: `mapable_time_init()` wired the page at boot, before `/dev/time`
    // can be opened.
    let address = unsafe { mach_clock::mapped_time_page() } as VmOffset;
    // SAFETY: `kernel_pmap` is the kernel's own pmap, so it maps `address`;
    // the C called `pmap_extract` with the same two values.
    let phys = unsafe { glue::pmap_extract(glue::kernel_pmap, address) };
    Some(phys >> PAGE_SHIFT)
}

/// Set the kernel's wall clock, at high IPL.
fn set_wallclock(seconds: i64) {
    // SAFETY: `splhigh()` is the real asm function <i386/spl.h> declares, and
    // the value it returns is only handed back to `splx()`.
    let s = unsafe { glue::splhigh() };
    // SAFETY: the clock interrupt is off, so it cannot see the store
    // half-written; the C took the same level around it.
    unsafe {
        mach_clock::set_wallclock(TimeValue64 {
            seconds,
            nanoseconds: 0,
        })
    };
    // SAFETY: `s` is the level `splhigh()` returned.
    unsafe { glue::splx(s) };
}

/// `inittodr()` of <i386/i386at/model_dep.h>.
pub(crate) fn inittodr() {
    let mut seconds: u64 = 0;
    // SAFETY: `seconds` is a local valid for a write, and `readtodc` leaves it
    // alone when it fails.
    unsafe { rtc::readtodc(&mut seconds) };
    // The C converted the `uint64_t` seconds to the record's `int64_t` field;
    // the cast reinterprets the bits as that conversion does.
    set_wallclock(seconds as i64);
}

/// `resettodr()` of <i386/i386/model_dep.h>.
pub(crate) fn resettodr() {
    // SAFETY: `writetodc` takes no argument, and the C passed none.
    unsafe { rtc::writetodc() };
}

/// Allocate `size` bytes of physical memory during bootstrap, page-rounded, or
/// `None` when the bootstrap allocator is out of pages.
pub(crate) fn alloc_aligned(size: VmSize) -> Option<VmOffset> {
    let rounded = size.wrapping_add(PAGE_MASK) & !PAGE_MASK;
    // vm_page_atop(): the page count, whose C parameter is an `unsigned int`,
    // so only the low 32 bits reach the allocator.
    let pages = (rounded >> PAGE_SHIFT) as u32;
    let address = biosmem::bootalloc(pages);
    if address == 0 { None } else { Some(address) }
}

/// `machine_init()` of <i386/i386/model_dep.h>.
pub(crate) fn machine_init() {
    // SAFETY: `machine_init` runs once, from `setup_main`, before any other
    // `biosmem` entry point is used again.
    unsafe { biosmem::biosmem_free_usable() };
    // SAFETY: `init_fpu` is the real C routine of `i386/i386/fpu.c`, and the
    // boot CPU is the caller's.
    unsafe { crate::arch::i386::fpu_ffi::init_fpu() };

    let err = crate::arch::i386::acpi_parse_apic::acpi_apic_init();
    if err != 0 {
        // SAFETY: the `%d` takes the matching `c_int`.
        unsafe {
            glue::printf(c"acpi_apic_init failed with %d\n".as_ptr(), err)
        };
        loop {
            core::hint::spin_loop();
        }
    }

    crate::arch::i386::smp::init();
    irq::init_irqs();
    ioapic::ioapic_configure();
    pit::clkstart();

    // SAFETY: `cninit` and `probeio` are the real C routines of
    // `device/cons.c` and `i386/i386at/autoconf.c`.
    unsafe {
        glue::cninit();
        glue::probeio();
    }

    inittodr();

    // SAFETY: the BIOS data word at 0x472 lives in the direct map, and the C
    // wrote the same value there.
    unsafe { ptr::write_volatile(phystokv(0x472) as *mut u16, 0x1234) };

    if VM_MIN_KERNEL_ADDRESS == 0 {
        // SAFETY: page 0 is mapped in this configuration, and the C unmapped
        // it here.
        unsafe { pmap::pmap_unmap_page_zero() };
    }

    patch_realmode_gdt();

    apic::hpet_init();
}

/// Patch the realmode GDT and the far jump after it with the address the AP
/// boot code was copied to.
fn patch_realmode_gdt() {
    // SAFETY: `gdt_descr_tmp` and `apboot_jmp_offset` are `cpuboot.S`'s
    // objects, and `machine_init` is their only writer.
    unsafe {
        // The C added the `phys_addr_t` to the 32-bit fields, so the
        // truncation below is the C's own.
        #[cfg(target_pointer_width = "32")]
        {
            let base = ptr::addr_of_mut!(glue::gdt_descr_tmp.linear_base);
            *base = (*base).wrapping_add(mp_desc::apboot_addr as u32);
            glue::apboot_jmp_offset = glue::apboot_jmp_offset
                .wrapping_add(mp_desc::apboot_addr as u32);
        }
        #[cfg(target_pointer_width = "64")]
        {
            let base = phystokv(
                ptr::addr_of_mut!(glue::gdt_descr_tmp.linear_base).addr(),
            ) as *mut u32;
            *base = (*base).wrapping_add(mp_desc::apboot_addr as u32);
            let jmp =
                phystokv(ptr::addr_of_mut!(glue::apboot_jmp_offset).addr())
                    as *mut u32;
            *jmp = (*jmp).wrapping_add(mp_desc::apboot_addr as u32);
        }
    }
}

/// `halt_cpu()` of <i386/i386/model_dep.h>.
pub(crate) fn halt_cpu() -> ! {
    // SAFETY: `cli` is legal at CPL 0, and this CPU never returns to the
    // interrupted code.
    unsafe { asm!("cli", options(nostack, preserves_flags)) };
    loop {
        idle();
    }
}

/// `halt_all_cpus()` of <i386/i386/model_dep.h>.
pub(crate) fn halt_all_cpus(reboot: c_int) -> ! {
    if reboot != 0 {
        // SAFETY: `kdreboot` is the keyboard controller's reset path, and the
        // C took it under the same flag.
        unsafe { crate::arch::i386::kd::kdreboot() };
    } else {
        // SAFETY: `rebootflag` has no other writer, and this CPU stops here.
        unsafe { rebootflag = 1 };
        // SAFETY: both messages hold no conversion specifiers.
        unsafe {
            glue::printf(
                c"Shutdown completed successfully, now in tight loop.\n"
                    .as_ptr(),
            );
            glue::printf(
                c"You can safely power off the system or hit ctl-alt-del to reboot\n"
                    .as_ptr(),
            );
        }
        // SAFETY: `spl0()` is the real asm function <i386/spl.h> declares.
        unsafe { glue::spl0() };
    }
    loop {
        idle();
    }
}

/// Register the boot loader's data with `biosmem` and `mbinfo`.
fn register_boot_data(mbi: &MultibootRawInfo) {
    let begin = ptr::addr_of!(glue::_start).addr();
    let end = ptr::addr_of!(glue::_end).addr();
    // SAFETY: the image bounds are the linker's, and this is the bootstrap
    // phase the call requires.
    unsafe {
        biosmem::biosmem_register_boot_data(
            begin.wrapping_sub(VM_MIN_KERNEL_ADDRESS),
            end.wrapping_sub(VM_MIN_KERNEL_ADDRESS),
            0,
        )
    };

    if mbi.flags & MULTIBOOT_LOADER_CMDLINE != 0 && mbi.cmdline != 0 {
        let start = address(mbi.cmdline);
        // SAFETY: the loader stored a NUL-terminated line at `start`, which
        // is in the direct map.
        let length = unsafe {
            crate::utils::string::strlen(kv_ptr::<c_char>(phystokv(start)))
        } + 1;
        // SAFETY: the range is the line the loader stored.
        unsafe {
            biosmem::biosmem_register_boot_data(
                start,
                start.wrapping_add(length),
                1,
            )
        };
    }

    if mbi.flags & MULTIBOOT_LOADER_MODULES != 0 && mbi.mods_count != 0 {
        let bytes = mbi
            .mods_count
            .wrapping_mul(size_of::<MultibootRawModule>() as u32);
        // SAFETY: the loader stored `mods_count` module records at
        // `mods_addr`.
        unsafe {
            biosmem::biosmem_register_boot_data(
                address(mbi.mods_addr),
                address(mbi.mods_addr.wrapping_add(bytes)),
                1,
            )
        };

        let modules =
            kv_ptr_mut::<MultibootRawModule>(phystokv(address(mbi.mods_addr)));
        for i in 0..mbi.mods_count {
            // SAFETY: `i` is below `mods_count`, and the records are live.
            let module = unsafe { modules.add(i as usize) };
            // SAFETY: as above.
            let (start, end, string) = unsafe {
                ((*module).mod_start, (*module).mod_end, (*module).string)
            };
            if end != start {
                // SAFETY: the loader's two bounds bracket a module image.
                unsafe {
                    biosmem::biosmem_register_boot_data(
                        address(start),
                        address(end),
                        1,
                    )
                };
            }

            if string != 0 {
                let string_start = address(string);
                // SAFETY: the loader stored a NUL-terminated name there.
                let length = unsafe {
                    crate::utils::string::strlen(kv_ptr::<c_char>(phystokv(
                        string_start,
                    )))
                } + 1;
                // SAFETY: the range is the name's.
                unsafe {
                    biosmem::biosmem_register_boot_data(
                        string_start,
                        string_start.wrapping_add(length),
                        1,
                    )
                };
            }
        }
    }

    if mbi.flags & MULTIBOOT_LOADER_SHDR != 0 {
        let bytes = mbi.shdr_num.wrapping_mul(mbi.shdr_size);
        if bytes != 0 {
            // SAFETY: the loader stored `shdr_num` headers at `shdr_addr`.
            unsafe {
                biosmem::biosmem_register_boot_data(
                    address(mbi.shdr_addr),
                    address(mbi.shdr_addr.wrapping_add(bytes)),
                    0,
                )
            };
        }

        let table = phystokv(address(mbi.shdr_addr));
        for i in 0..mbi.shdr_num {
            let offset = i.wrapping_mul(mbi.shdr_size);
            // `i` is below `shdr_num`, and each record is `shdr_size` bytes
            // of the loader's table.
            let shdr = ptr::with_exposed_provenance::<ElfShdr>(
                table.wrapping_add(address(offset)),
            );
            // SAFETY: as above.
            let (type_, size, addr) =
                unsafe { ((*shdr).type_, (*shdr).size, (*shdr).addr) };
            if type_ != ELF_SHT_SYMTAB && type_ != ELF_SHT_STRTAB {
                continue;
            }

            if size != 0 {
                // SAFETY: the header names `size` bytes of the section at
                // `addr`.
                unsafe {
                    biosmem::biosmem_register_boot_data(
                        addr,
                        addr.wrapping_add(address(size)),
                        0,
                    )
                };
            }
        }
    }

    // SAFETY: this is the bootstrap phase the call requires.
    // The two mirrors describe the same `struct multiboot_raw_info`;
    // `mbinfo`'s shorter `fb_info` only affects `/dev/mbinfo`'s view.
    unsafe {
        mbinfo::mbinfo_register_boot_data(
            (mbi as *const MultibootRawInfo).cast(),
        )
    };
}

/// `i386at_init()` in `i386/i386at/model_dep.c`.
fn i386at_init() {
    ioapic::picdisable();

    // SAFETY: `boot_info` is the loader's block, copied to safe memory by
    // `c_boot_entry`, which is the only writer.
    register_boot_data(unsafe { &*ptr::addr_of!(boot_info) });
    // SAFETY: as above; this is the boot phase `biosmem_bootstrap` requires.
    unsafe { biosmem::biosmem_bootstrap(ptr::addr_of!(boot_info).cast()) };

    // The copy below overwrites `boot_info`'s own fields, so the loader's
    // values are read first.
    // SAFETY: as above; no other CPU is running yet.
    let (flags, cmdline, mods_count, mods_addr) = unsafe {
        (
            boot_info.flags,
            boot_info.cmdline,
            boot_info.mods_count,
            boot_info.mods_addr,
        )
    };

    // The loader's command line and modules are copied out of low memory
    // before `biosmem_setup` can hand those pages to the VM system.
    if flags & MULTIBOOT_CMDLINE != 0 {
        let source = address(cmdline);
        // SAFETY: the loader stored a NUL-terminated line at `source`.
        let length = unsafe {
            crate::utils::string::strlen(kv_ptr::<c_char>(phystokv(source)))
        } + 1;
        let Some(mem) = alloc_aligned(length) else {
            // SAFETY: `Panic` does not return, and the message holds no
            // conversion specifiers.
            unsafe {
                glue::Panic(
                    c"i386/i386at/model_dep.c".as_ptr(),
                    line!() as c_int,
                    c"i386at_init".as_ptr(),
                    c"could not allocate memory for multiboot command line"
                        .as_ptr(),
                )
            }
        };
        // SAFETY: `source` names `length` readable bytes and the boot
        // allocator returned `length` writable ones.
        unsafe {
            crate::utils::string::memcpy(
                kv_ptr_mut::<c_void>(phystokv(mem)),
                kv_ptr::<c_void>(phystokv(source)),
                length,
            )
        };
        // SAFETY: `kernel_cmdline` and `boot_info` are written only here and
        // only on the boot CPU.
        unsafe {
            kernel_cmdline = kv_ptr_mut::<c_char>(phystokv(mem));
            boot_info.cmdline = mem as u32;
        }
    }

    if flags & MULTIBOOT_MODS != 0 && mods_count != 0 {
        let bytes =
            mods_count.wrapping_mul(size_of::<MultibootRawModule>() as u32);
        let Some(mem) = alloc_aligned(address(bytes)) else {
            // SAFETY: `Panic` does not return, and the message holds no
            // conversion specifiers.
            unsafe {
                glue::Panic(
                    c"i386/i386at/model_dep.c".as_ptr(),
                    line!() as c_int,
                    c"i386at_init".as_ptr(),
                    c"could not allocate memory for multiboot modules"
                        .as_ptr(),
                )
            }
        };
        let modules = kv_ptr_mut::<MultibootRawModule>(phystokv(mem));
        // SAFETY: the loader stored `mods_count` records at `mods_addr`, the
        // boot allocator returned `bytes` for the copy, and the two do not
        // overlap.
        unsafe {
            crate::utils::string::memcpy(
                modules.cast(),
                kv_ptr::<c_void>(phystokv(address(mods_addr))),
                address(bytes),
            )
        };
        // SAFETY: as the command-line store above.
        unsafe { boot_info.mods_addr = mem as u32 };

        for i in 0..mods_count {
            // SAFETY: `i` is below `mods_count`, and the records were just
            // copied into `modules`.
            let module = unsafe { modules.add(i as usize) };
            // SAFETY: as above.
            let (start, end, string) = unsafe {
                ((*module).mod_start, (*module).mod_end, (*module).string)
            };
            let size = end.wrapping_sub(start);
            let Some(image) = alloc_aligned(address(size)) else {
                // SAFETY: `Panic` does not return, and the `%d` takes `i`.
                unsafe {
                    glue::Panic(
                        c"i386/i386at/model_dep.c".as_ptr(),
                        line!() as c_int,
                        c"i386at_init".as_ptr(),
                        c"could not allocate memory for multiboot module %d"
                            .as_ptr(),
                        i,
                    )
                }
            };
            // SAFETY: `start` names `size` readable bytes and the boot
            // allocator returned `size` writable ones.
            unsafe {
                crate::utils::string::memcpy(
                    kv_ptr_mut::<c_void>(phystokv(image)),
                    kv_ptr::<c_void>(phystokv(address(start))),
                    address(size),
                )
            };
            // SAFETY: the record was copied into `modules`, and this CPU is
            // its only writer.
            unsafe {
                (*module).mod_start = image as u32;
                (*module).mod_end = image.wrapping_add(address(size)) as u32;
            }

            let string_start = address(string);
            // SAFETY: the loader stored a NUL-terminated name at `string`.
            let length = unsafe {
                crate::utils::string::strlen(kv_ptr::<c_char>(phystokv(
                    string_start,
                )))
            } + 1;
            let Some(name) = alloc_aligned(length) else {
                // SAFETY: `Panic` does not return, and the `%d` takes `i`.
                unsafe {
                    glue::Panic(
                        c"i386/i386at/model_dep.c".as_ptr(),
                        line!() as c_int,
                        c"i386at_init".as_ptr(),
                        c"could not allocate memory for multiboot module command line %d"
                            .as_ptr(),
                        i,
                    )
                }
            };
            // SAFETY: `string_start` names `length` readable bytes and the
            // boot allocator returned `length` writable ones.
            unsafe {
                crate::utils::string::memcpy(
                    kv_ptr_mut::<c_void>(phystokv(name)),
                    kv_ptr::<c_void>(phystokv(string_start)),
                    length,
                )
            };
            // SAFETY: the record was copied into `modules`, and this CPU is
            // its only writer.
            unsafe { (*module).string = name as u32 };
        }
    }

    pmap::pmap_bootstrap();
    // SAFETY: `biosmem_setup` runs once, after `biosmem_bootstrap`, on the
    // same CPU.
    unsafe { biosmem::biosmem_setup() };

    pmap::pmap_make_temporary_mapping();
    pmap::pmap_set_page_dir();

    let cr0 = fpu::read_cr0();
    fpu::write_cr0(cr0 | fpu::CR0_PG | fpu::CR0_WP);
    let cr0 = fpu::read_cr0();
    fpu::write_cr0(cr0 & !(fpu::CR0_CD | fpu::CR0_NW));
    if pmap::cpu_has_feature(pmap::CPU_FEATURE_PGE) {
        let cr4 = fpu::read_cr4();
        fpu::write_cr4(cr4 | fpu::CR4_PGE);
    }
    mp_desc::flush_instr_queue();

    // SAFETY: the descriptor tables are the real C routines of `gdt.c`,
    // `idt.c`, `int_init.c`, `ldt.c` and `ktss.c`, and this runs on the boot
    // CPU before any other one starts.
    unsafe {
        glue::gdt_init();
        glue::idt_init();
        glue::int_init();
        glue::ldt_init();
        glue::ktss_init();
        glue::init_percpu(0);
    }
    mp_desc::mp_desc_init(0);

    pmap::pmap_remove_temporary_mapping();

    mp_desc::interrupt_stack_alloc();
    // SAFETY: `spl_init` is `ioapic.rs`'s global, and this is its only writer
    // once the real IOAPIC is up.
    unsafe { ioapic::spl_init = 1 };
}

/// `c_boot_entry()` of <i386/i386/model_dep.h>, the C entry `boothdr.S` calls.
pub(crate) fn c_boot_entry(bi: VmOffset) {
    // SAFETY: `bi` is the physical address `boothdr.S` passes, and the
    // loader's block there is readable.
    unsafe { boot_info = *kv_ptr::<MultibootRawInfo>(phystokv(bi)) };

    // SAFETY: both are the literals of the C, and the second takes no
    // arguments.
    unsafe {
        glue::printf(c"%s".as_ptr(), ptr::addr_of!(glue::version));
        glue::printf(c"\n".as_ptr());
    }

    // SAFETY: `discover_x86_cpu_type` is the real asm routine of
    // `i386/i386/locore.S`.
    // The call also fills `cpu_features`; the i386 kernel uses the
    // return value, the x86_64 one does not.
    #[cfg_attr(target_pointer_width = "64", expect(unused_variables))]
    let cpu_type = unsafe { glue::discover_x86_cpu_type() };

    i386at_init();

    // SAFETY: the boot CPU's slot is this CPU's to fill, and the C filled the
    // same fields.
    unsafe {
        glue::machine_slot[0].is_cpu = 1;
        glue::machine_slot[0].cpu_subtype = CPU_SUBTYPE_AT386;
    }

    #[cfg(target_pointer_width = "64")]
    // SAFETY: as above.
    unsafe {
        glue::machine_slot[0].cpu_type = CPU_TYPE_X86_64
    };

    #[cfg(target_pointer_width = "32")]
    {
        let type_ = match cpu_type {
            3 => CPU_TYPE_I386,
            4 => CPU_TYPE_I486,
            5 => CPU_TYPE_PENTIUM,
            6 | 15 => CPU_TYPE_PENTIUMPRO,
            other => {
                // SAFETY: the `%d` takes the matching `c_int`.
                unsafe {
                    glue::printf(
                        c"warning: unknown cpu type %d, assuming i386\n"
                            .as_ptr(),
                        other,
                    )
                };
                CPU_TYPE_I386
            }
        };
        // SAFETY: the boot CPU's slot is this CPU's to fill.
        unsafe { glue::machine_slot[0].cpu_type = type_ };
    }

    // SAFETY: `setup_main` is the real C routine of `kern/startup.c`.
    unsafe { glue::setup_main() };
}

/// `startrtclock()` of <i386/i386/model_dep.h>.
pub(crate) fn startrtclock() {
    // The C's non-APIC branch (`clkstart()` plus `unmask_irq(0)`) is not part
    // of either configured kernel; both define `APIC`.
    // SAFETY: `timer_pin` is read after `ioapic_configure` picked it, and the
    // boot path is single-threaded.
    let pin = unsafe { ioapic::timer_pin };
    ioapic::unmask(pin);
    ioapic::calibrate_lapic_timer();
    if percpu::cpu_number() != 0 {
        ioapic::lapic_enable_timer();
    }
}
