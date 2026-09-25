// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/bootstrap.h:
//   Copyright (c) 2013 Free Software Foundation.
// Derived from kern/bootstrap.c:
//   Copyright (c) 1992-1989 Carnegie Mellon University.
//   Copyright (c) 1995-1993 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Derived from kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The multiboot bootstrap, which `kern/bootstrap.c` used to define, and
//! the `struct multiboot_raw_info` mirror of
//! <i386/include/mach/i386/multiboot.h>.

use crate::arch::i386::pcb::{set_user_regs, user_stack_low};
use crate::arch::i386::percpu::current_thread;
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue;
use crate::ipc::ipc_port;
use crate::ipc::mach_port;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::boot_script::{self, Cmd};
use crate::kern::elf_load::{self, ExecInfo, ExecSectype};
use crate::kern::host;
use crate::kern::lock::SimpleLock;
use crate::kern::printf;
use crate::kern::sched_prim::{self, THREAD_AWAKENED};
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::{self, MapSource, Task, current_task};
use crate::kern::thread::Thread;
use crate::vm::types::VmProt;
use crate::vm::vm_kern::VM_MIN_KERNEL_ADDRESS;
use crate::vm::vm_map::{VmMap, round_page, trunc_page};
use crate::vm::{vm_page, vm_user};
use core::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use core::mem::{MaybeUninit, offset_of, size_of};
use core::ptr::{
    self, NonNull, addr_of_mut, null_mut, with_exposed_provenance,
    with_exposed_provenance_mut,
};
use core::slice;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// `MULTIBOOT_MODS` of <mach/machine/multiboot.h>: the boot loader supplied
/// module information.
const MULTIBOOT_MODS: u32 = 0x0000_0008;

/// `MACH_MSG_TYPE_PORT_SEND` of <mach/message.h>.
const MACH_MSG_TYPE_PORT_SEND: c_uint = 17;

/// `BASEPRI_USER` of <kern/sched.h>: a fresh user task's priority.
const BASEPRI_USER: c_int = 25;

/// `STACK_SIZE` of `kern/bootstrap.c`: the user stack of the bootstrap task.
const STACK_SIZE: VmSize = 2 * 64 * 1024;

/// `IP_NULL` of <ipc/ipc_port.h>.
const IP_NULL: *mut c_void = null_mut();

/// `VM_INHERIT_DEFAULT` of <mach/vm_inherit.h>.
const VM_INHERIT_COPY: c_int = 1;

/// The bytes one `mach_port_name_t` can spell, the C's `host_string[12]`.
const PORT_STRING_SIZE: usize = 12;

/// `struct multiboot_raw_module` of <mach/machine/multiboot.h>, field for
/// field.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootRawModule {
    pub mod_start: u32,
    pub mod_end: u32,
    pub string: u32,
    pub reserved: u32,
}

const _: () = {
    assert!(size_of::<MultibootRawModule>() == 16);
    assert!(offset_of!(MultibootRawModule, mod_start) == 0);
    assert!(offset_of!(MultibootRawModule, mod_end) == 4);
    assert!(offset_of!(MultibootRawModule, string) == 8);
    assert!(offset_of!(MultibootRawModule, reserved) == 12);
};

/// The anonymous union of `struct multiboot_framebuffer_info`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union MultibootFramebufferFields {
    pub palette: MultibootFramebufferPalette,
    pub rgb: MultibootFramebufferRgb,
}

/// The palette arm of the framebuffer union.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MultibootFramebufferPalette {
    pub framebuffer_palette_addr: u32,
    pub framebuffer_palette_num_colors: u16,
}

/// The RGB arm of the framebuffer union.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MultibootFramebufferRgb {
    pub framebuffer_red_field_position: u8,
    pub framebuffer_red_mask_size: u8,
    pub framebuffer_green_field_position: u8,
    pub framebuffer_green_mask_size: u8,
    pub framebuffer_blue_field_position: u8,
    pub framebuffer_blue_mask_size: u8,
}

/// `struct multiboot_framebuffer_info` of <mach/machine/multiboot.h>, field
/// for field.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootFramebufferInfo {
    pub framebuffer_addr: u64,
    pub framebuffer_pitch: u32,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_bpp: u8,
    pub framebuffer_type: u8,
    pub framebuffer: MultibootFramebufferFields,
}

const _: () = {
    assert!(size_of::<MultibootFramebufferInfo>() == 30);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_addr) == 0);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_pitch) == 8);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_width) == 12);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_height) == 16);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_bpp) == 20);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer_type) == 21);
    assert!(offset_of!(MultibootFramebufferInfo, framebuffer) == 22);
};

/// `struct multiboot_raw_info` of <mach/machine/multiboot.h>, field for
/// field.  The `boot_info` global of `src/arch/i386/model_dep.rs` has this
/// type, and both kernels' debug info shows the layout below.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootRawInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub unused0: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
    pub shdr_num: u32,
    pub shdr_size: u32,
    pub shdr_addr: u32,
    pub shdr_strndx: u32,
    pub mmap_length: u32,
    pub mmap_addr: u32,
    pub unused1: [u32; 9],
    pub fb_info: MultibootFramebufferInfo,
}

const _: () = {
    assert!(size_of::<MultibootRawInfo>() == 118);
    assert!(offset_of!(MultibootRawInfo, flags) == 0);
    assert!(offset_of!(MultibootRawInfo, mem_lower) == 4);
    assert!(offset_of!(MultibootRawInfo, mem_upper) == 8);
    assert!(offset_of!(MultibootRawInfo, cmdline) == 16);
    assert!(offset_of!(MultibootRawInfo, mods_count) == 20);
    assert!(offset_of!(MultibootRawInfo, mods_addr) == 24);
    assert!(offset_of!(MultibootRawInfo, shdr_num) == 28);
    assert!(offset_of!(MultibootRawInfo, shdr_size) == 32);
    assert!(offset_of!(MultibootRawInfo, shdr_addr) == 36);
    assert!(offset_of!(MultibootRawInfo, shdr_strndx) == 40);
    assert!(offset_of!(MultibootRawInfo, mmap_length) == 44);
    assert!(offset_of!(MultibootRawInfo, mmap_addr) == 48);
    assert!(offset_of!(MultibootRawInfo, unused1) == 52);
    assert!(offset_of!(MultibootRawInfo, fb_info) == 88);
};

/// `phystokv()` of <i386/vm_param.h>: a physical address in the kernel's
/// direct map.
fn phystokv(pa: VmOffset) -> VmOffset {
    pa.wrapping_add(VM_MIN_KERNEL_ADDRESS)
}

/// The pointer-width address a 32-bit multiboot field holds; the widening is
/// lossless on both targets.
fn address(value: u32) -> VmOffset {
    value as VmOffset
}

/// The kernel pointer a direct-map address denotes.
fn kv_ptr<T>(address: VmOffset) -> *const T {
    with_exposed_provenance(address)
}

/// The writable kernel pointer a direct-map address denotes.
fn kv_ptr_mut<T>(address: VmOffset) -> *mut T {
    with_exposed_provenance_mut(address)
}

/// The user address a kernel value denotes, both targets' identity
/// conversion.
fn user_ptr(address: VmOffset) -> *mut c_void {
    with_exposed_provenance_mut(address)
}

/// `boot_host_port` and `boot_device_port` of `kern/bootstrap.c`: the local
/// names `bootstrap_exec_compat()` inserted for the user bootstrap.
///
/// The stores happen before `Thread::resume()` publishes the bootstrap
/// thread, and `user_bootstrap_compat()` runs only after that publish, so
/// `Release`/`Acquire` record the hand-off.
static BOOT_HOST_PORT: AtomicU32 = AtomicU32::new(0);
static BOOT_DEVICE_PORT: AtomicU32 = AtomicU32::new(0);

/// `free_bootstrap_pages()` of `kern/bootstrap.c`: hand the module's pages
/// to the page allocator.
///
/// # Safety
///
/// The range must name pages `vm_page_lookup_pa()` can describe.
unsafe fn free_bootstrap_pages(start: VmOffset, end: VmOffset) {
    let mut start = start;
    while start < end {
        if let Some(page) = vm_page::lookup_pa(start) {
            // SAFETY: `lookup_pa()` returned a live descriptor the kernel
            // no longer needs.
            unsafe { vm_page::manage(page.as_ptr()) };
        }
        start = start.wrapping_add(PAGE_SIZE);
    }
}

/// `itoa()` of `kern/bootstrap.c`: the decimal spelling of `num`, with its
/// NUL, in the tail of `buf`.  The C copied it to the front afterwards.
fn itoa(buf: &mut [u8; PORT_STRING_SIZE], num: u32) -> &[u8] {
    let mut end = buf.len() - 1;
    buf[end] = 0;
    let mut number = num;
    loop {
        end -= 1;
        buf[end] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    &buf[end..]
}

/// The byte of `line` at `index`, or NUL past its end.
fn line_byte(line: &[u8], index: usize) -> u8 {
    line.get(index).copied().unwrap_or(0)
}

/// `get_compat_strings()` of `kern/bootstrap.c`: the old serverboot boot
/// flags and root name from the multiboot command line.
fn get_compat_strings(
    flags: &mut [u8; 1024],
    root: &mut [u8; 1024],
    cmdline: &[u8],
) {
    root[..b"UNKNOWN".len()].copy_from_slice(b"UNKNOWN");

    let mut cp = 0;
    flags[cp] = b'-';
    cp += 1;

    let mut ip = 0;
    while line_byte(cmdline, ip) != 0 {
        // The C compared `char` values against the space byte, so bytes
        // over 0x7f stop the loops.
        let byte = line_byte(cmdline, ip) as c_char;
        if byte == b' ' as c_char {
            ip += 1;
        } else if byte == b'-' as c_char {
            ip += 1;
            while line_byte(cmdline, ip) as c_char > b' ' as c_char {
                if cp < flags.len() {
                    flags[cp] = cmdline[ip];
                    cp += 1;
                }
                ip += 1;
            }
        } else if cmdline[ip..].starts_with(b"root=") {
            let mut rp = 0;
            ip += 5;
            if cmdline[ip..].starts_with(b"/dev/") {
                ip += 5;
            }
            while line_byte(cmdline, ip) as c_char > b' ' as c_char {
                if rp < root.len() {
                    root[rp] = cmdline[ip];
                    rp += 1;
                }
                ip += 1;
            }
            if rp < root.len() {
                root[rp] = 0;
            }
        } else {
            while line_byte(cmdline, ip) as c_char > b' ' as c_char {
                ip += 1;
            }
        }
    }

    if cp == 1 && cp < flags.len() {
        flags[cp] = b'x';
        cp += 1;
    }
    if cp < flags.len() {
        flags[cp] = 0;
    }
}

/// `make_send()` of `kern/bootstrap.c`'s `ipc_port_make_send()` call.
///
/// # Safety
///
/// `port` must be a live active port.
unsafe fn make_send(port: *mut c_void) -> *mut c_void {
    // SAFETY: the caller promises the live port.
    unsafe { ipc_port::make_send(IpcPort::from_raw(port)) }.as_ptr()
}

/// `task_insert_send_right()` of `kern/bootstrap.c`: name `port` in `task`'s
/// space, retrying the names the space already holds.
///
/// # Safety
///
/// `task` must be a live task and `port` a live port the call consumes one
/// reference to.
unsafe fn insert_send_right(task: *mut Task, port: *mut c_void) -> c_uint {
    let mut name: c_uint = 1;
    loop {
        // SAFETY: the caller promises the live task.
        let space = unsafe { (*task).itk_space };
        // SAFETY: the space and port are live; `mach_port_insert_right()`
        // consumes the port reference only when it succeeds.
        let result = unsafe {
            mach_port::insert_right(
                IpcSpace::new(space),
                name,
                port,
                MACH_MSG_TYPE_PORT_SEND,
            )
        };
        if result.is_ok() {
            return name;
        }
        name = name.wrapping_add(1);
    }
}

/// `boot_script_task_create()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `cmd` must be a live command with a NUL-terminated path, and the caller
/// must hold no locks: the routine allocates and may block.
pub(crate) unsafe fn task_create(
    cmd: *mut Cmd,
) -> Result<(), boot_script::Error> {
    // SAFETY: the caller promises a live command; creation may block, as
    // `MapSource::Fresh` expects, and the caller holds no locks.
    let task = match unsafe {
        task::create_kernel_task(null_mut(), MapSource::Fresh)
    } {
        Ok(task) => task,
        Err(error) => {
            // SAFETY: `printf` takes the format and one integer.
            unsafe {
                glue::printf(
                    c"boot_script_task_create failed with %x\n".as_ptr(),
                    c_int::from(error),
                )
            };
            return Err(boot_script::Error::MachError);
        }
    };
    // SAFETY: the command is live and the task fresh.
    unsafe { (*cmd).task = task };
    // SAFETY: the command's path is NUL-terminated.
    let name = unsafe { CStr::from_ptr((*cmd).path) }.to_bytes();
    // SAFETY: the task is live.
    let _ = unsafe { task::set_name(task, name) };
    // SAFETY: the host object is live from `host_init()` on.
    let host = unsafe { (*host::realhost()).host_self };
    // SAFETY: the task is live and the caller holds no locks.
    let _ =
        unsafe { task::max_priority(host, task, BASEPRI_USER, true, true) };
    Ok(())
}

/// `boot_script_task_resume()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `cmd` must be a live command whose task is live, and the caller must hold
/// no locks: the routine may block.
pub(crate) unsafe fn task_resume(
    cmd: *mut Cmd,
) -> Result<(), boot_script::Error> {
    // SAFETY: the caller promises the live command and task.
    match unsafe { task::resume((*cmd).task) } {
        Ok(()) => {
            // SAFETY: `printf` takes the format and one string.
            unsafe { glue::printf(c"\nstart %s: ".as_ptr(), (*cmd).path) };
            Ok(())
        }
        Err(error) => {
            // SAFETY: `printf` takes the format and one integer.
            unsafe {
                glue::printf(
                    c"boot_script_task_resume failed with %x\n".as_ptr(),
                    c_int::from(error),
                )
            };
            Err(boot_script::Error::MachError)
        }
    }
}

/// `boot_script_prompt_task_resume()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `cmd` must be a live command whose task is live.
pub(crate) unsafe fn prompt_task_resume(
    cmd: *mut Cmd,
) -> Result<(), boot_script::Error> {
    // SAFETY: both `printf` calls take their format and, in the first, one
    // string.
    unsafe {
        glue::printf(c"Pausing for %s...\n".as_ptr(), (*cmd).path);
        glue::printf(c"Hit <return> to resume bootstrap.".as_ptr());
    }
    let mut line = [0 as c_char; 5];
    // SAFETY: the buffer holds five bytes and the reader stops inside them.
    unsafe { printf::safe_gets(line.as_mut_ptr(), 5) };
    // SAFETY: the caller promises the live command.
    unsafe { task_resume(cmd) }
}

/// `boot_script_insert_right()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `cmd` must be a live command with a live task, and `port` a live port.
pub(crate) unsafe fn insert_right(cmd: *mut Cmd, port: *mut c_void) -> c_uint {
    // SAFETY: the caller promises the command and port.
    unsafe { insert_send_right((*cmd).task, make_send(port)) }
}

/// `boot_script_insert_task_port()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `cmd` must be a live command with a live task, and `task` a live task.
pub(crate) unsafe fn insert_task_port(
    cmd: *mut Cmd,
    task: *mut Task,
) -> c_uint {
    // SAFETY: the task's self port is live.
    unsafe { insert_send_right((*cmd).task, make_send((*task).itk_sself)) }
}

/// `boot_script_free_task()` of `kern/bootstrap.c`.
///
/// # Safety
///
/// `task` must be null or the live task the matching
/// `boot_script_task_create()` returned; the caller must hold no locks,
/// because termination may block.
pub(crate) unsafe fn free_task(task: *mut Task, aborting: bool) {
    // SAFETY: the caller's contract; both callees accept a null task.
    unsafe {
        if aborting {
            let _ = task::terminate(task);
        }
        task::deallocate(task);
    }
}

/// `boot_read()` of `kern/bootstrap.c`: read a module's bytes for the ELF
/// loader.
///
/// # Safety
///
/// `handle` must be the module `bootstrap_create()` passed, `buf` writable
/// for `size` bytes, and `out_actual` writable.
unsafe extern "C" fn boot_read(
    handle: *mut c_void,
    file_ofs: VmOffset,
    buf: *mut c_void,
    size: VmSize,
    out_actual: *mut VmSize,
) -> c_int {
    let module = handle.cast::<MultibootRawModule>();
    // SAFETY: the caller promises the module handle.
    let start = address(unsafe { (*module).mod_start });
    let end = address(unsafe { (*module).mod_end });
    if start.wrapping_add(file_ofs).wrapping_add(size) > end {
        return -1;
    }

    let source = phystokv(start).wrapping_add(file_ofs);
    // SAFETY: the module's bytes are mapped and `buf` holds `size` bytes.
    unsafe {
        ptr::copy_nonoverlapping(kv_ptr::<u8>(source), buf.cast::<u8>(), size);
        *out_actual = size;
    }
    0
}

/// `read_exec()` of `kern/bootstrap.c`: place one executable section in the
/// current task.
///
/// # Safety
///
/// `handle` must be the module `bootstrap_create()` passed; the routine runs
/// on the current task and may block in `vm_allocate()`.
unsafe extern "C" fn read_exec(
    handle: *mut c_void,
    file_ofs: VmOffset,
    file_size: VmSize,
    mem_addr: VmOffset,
    mem_size: VmSize,
    section_type: ExecSectype,
) -> c_int {
    let module = handle.cast::<MultibootRawModule>();
    // SAFETY: the caller promises the module handle.
    let start = address(unsafe { (*module).mod_start });
    let end = address(unsafe { (*module).mod_end });
    if start.wrapping_add(file_ofs).wrapping_add(file_size) > end {
        return -1;
    }
    if !section_type.contains(ExecSectype::ALLOC) {
        return 0;
    }

    // SAFETY: the routine runs on the current task, whose map is live.
    let map = unsafe { (*current_task()).map }.cast::<VmMap>();
    let mut start_page = trunc_page(mem_addr);
    let end_page = round_page(mem_addr.wrapping_add(mem_size));
    let page_count = end_page - start_page;
    if let Some(mut map) = NonNull::new(map) {
        // SAFETY: the map is live and the caller holds no locks.
        let _ = vm_user::allocate(
            unsafe { map.as_mut() },
            &mut start_page,
            page_count,
            false,
        );
    }

    if file_size > 0 {
        let source = phystokv(start).wrapping_add(file_ofs);
        // SAFETY: the section's file bytes are mapped and `mem_addr` is
        // user storage in the task's map.
        unsafe {
            glue::copyout(
                kv_ptr::<c_void>(source),
                user_ptr(mem_addr),
                file_size,
            )
        };
    }

    let mem_prot = section_type.protection();
    if mem_prot != VmProt::ALL
        && let Some(mut map) = NonNull::new(map)
    {
        // SAFETY: the map is live and the caller holds no locks.
        let _ = vm_user::protect(
            unsafe { map.as_mut() },
            start_page,
            end_page - start_page,
            false,
            mem_prot,
        );
    }
    0
}

/// `copy_bootstrap()` of `kern/bootstrap.c`: load the image into the current
/// task's map, or halt.
///
/// # Safety
///
/// `handle` must be the module `bootstrap_create()` passed.
unsafe fn load_bootstrap(handle: *mut c_void) -> ExecInfo {
    let mut info = MaybeUninit::<ExecInfo>::uninit();
    // SAFETY: both callbacks accept the handle, and `info` is writable
    // storage for the record.
    let error = unsafe {
        elf_load::exec_load(boot_read, read_exec, handle, info.as_mut_ptr())
    };
    if error != 0 {
        // SAFETY: `Panic()` does not return.
        unsafe {
            glue::Panic(
                c"kern/bootstrap.c".as_ptr(),
                line!() as c_int,
                c"copy_bootstrap".as_ptr(),
                c"Cannot load user-bootstrap image: error code %d".as_ptr(),
                error,
            )
        };
    }
    // SAFETY: `exec_load()` zeroes the record and fills it before success.
    unsafe { info.assume_init() }
}

/// One environment entry, which the C built as one `NAME=value` string.
struct EnvVar<'a> {
    name: &'a [u8],
    value: &'a [u8],
}

/// `copyout()` one value to the user stack.
///
/// # Safety
///
/// `to` must be a writable user address of `size_of::<T>()` bytes.
unsafe fn copyout_value<T>(value: &T, to: VmOffset) {
    // SAFETY: the caller promises the destination and `value` is live.
    unsafe {
        glue::copyout(
            ptr::from_ref(value).cast(),
            user_ptr(to),
            size_of::<T>(),
        )
    };
}

/// `copyout()` raw bytes to the user stack.
///
/// # Safety
///
/// `from` must be readable for `len` bytes and `to` a writable user address
/// of that many bytes.
unsafe fn copyout_bytes(from: *const c_void, to: VmOffset, len: usize) {
    // SAFETY: the caller promises both ranges.
    unsafe { glue::copyout(from, user_ptr(to), len) };
}

/// `build_args_and_stack()` of `kern/bootstrap.c`: allocate the user stack
/// and write the argument and environment vectors into it.
///
/// # Safety
///
/// Runs on the current thread whose pcb is live, `info` must be the record
/// `exec_load()` filled, and every string must outlive the call.
unsafe fn build_args_and_stack(
    info: &ExecInfo,
    argv: &[*const c_char],
    envp: &[EnvVar],
) {
    let mut arg_len = 0usize;
    for &arg in argv {
        // SAFETY: every argument is a NUL-terminated string.
        arg_len += unsafe { CStr::from_ptr(arg) }.to_bytes().len() + 1;
    }
    for env in envp {
        arg_len += env.name.len() + env.value.len() + 1;
    }
    arg_len += size_of::<VmOffset>()
        + (argv.len() + 1 + envp.len() + 1) * size_of::<VmOffset>();

    let stack_size = round_page(STACK_SIZE);
    let mut stack_base = user_stack_low(stack_size);
    // SAFETY: the current task's map is live, the stack range is free, and
    // `stack_base` is a writable slot.
    unsafe {
        glue::vm_map(
            (*current_task()).map.cast(),
            &mut stack_base,
            stack_size,
            0,
            0,
            IP_NULL,
            0,
            0,
            info.stack_prot().bits(),
            VmProt::ALL.bits(),
            VM_INHERIT_COPY,
        )
    };

    // SAFETY: the stack is mapped in the current task, and `info` is live.
    // Its record is the same `exec_info_t` `arch/i386/pcb.rs` mirrors, so the
    // pointer carries over.
    let mut arg_pos = unsafe {
        set_user_regs(
            stack_base,
            stack_size,
            core::ptr::from_ref(info).cast(),
            arg_len,
        )
    };
    let mut string_pos = arg_pos
        + size_of::<VmOffset>()
        + (argv.len() + 1 + envp.len() + 1) * size_of::<VmOffset>();

    let count = argv.len();
    // SAFETY: `arg_pos` names the user stack the call just mapped.
    unsafe { copyout_value(&count, arg_pos) };
    arg_pos += size_of::<VmOffset>();
    for &arg in argv {
        // SAFETY: every argument is a NUL-terminated string.
        let bytes = unsafe { CStr::from_ptr(arg) }.to_bytes_with_nul();
        // SAFETY: the user stack has room for the pointer and the bytes.
        unsafe {
            copyout_value(&string_pos, arg_pos);
            arg_pos += size_of::<VmOffset>();
            copyout_bytes(arg.cast(), string_pos, bytes.len());
        }
        string_pos += bytes.len();
    }

    let zero: VmOffset = 0;
    // SAFETY: as above; this is the null terminator of `argv`.
    unsafe { copyout_value(&zero, arg_pos) };
    arg_pos += size_of::<VmOffset>();
    for env in envp {
        // SAFETY: the user stack has room for the pointer, both halves and
        // the NUL.
        unsafe {
            copyout_value(&string_pos, arg_pos);
            arg_pos += size_of::<VmOffset>();
            copyout_bytes(
                env.name.as_ptr().cast(),
                string_pos,
                env.name.len(),
            );
            string_pos += env.name.len();
            copyout_bytes(
                env.value.as_ptr().cast(),
                string_pos,
                env.value.len(),
            );
            string_pos += env.value.len();
            let nul: u8 = 0;
            copyout_bytes(ptr::from_ref(&nul).cast(), string_pos, 1);
        }
        string_pos += 1;
    }
    // SAFETY: as above; this is the null terminator of `envp`.
    unsafe { copyout_value(&zero, arg_pos) };
}

/// `struct user_bootstrap_info` of `kern/bootstrap.c`: what the waiting
/// `boot_script_exec_cmd()` hands the bootstrap thread.
///
/// The lock is the kernel's [`SimpleLock`], not a spin mutex, because the
/// C's wait protocol hands it straight to `thread_sleep()`.
struct UserBootstrapInfo {
    module: *mut c_void,
    argv: *mut *mut c_char,
    done: AtomicI32,
    lock: SimpleLock,
}

/// Walk a C `char **` up to its null terminator.
///
/// # Safety
///
/// `argv` must point at a null-terminated vector of readable strings that
/// outlive the returned slice.
unsafe fn argv_slice<'a>(argv: *mut *mut c_char) -> &'a [*const c_char] {
    let mut len = 0;
    while !unsafe { *argv.add(len) }.is_null() {
        len += 1;
    }
    // SAFETY: the walk above bounded the vector.
    unsafe { slice::from_raw_parts(argv.cast::<*const c_char>(), len) }
}

/// `user_bootstrap()` of `kern/bootstrap.c`: the kernel-mode half of the
/// first user thread.
///
/// # Safety
///
/// The thread's `saved.other` must be the info `boot_script_exec_cmd()`
/// published.
unsafe extern "C" fn user_bootstrap() {
    // SAFETY: the launcher published the pointer before starting this
    // thread and keeps it alive until `done` is set.
    let info = current_thread();
    let info = unsafe { (*info).saved.other.cast::<UserBootstrapInfo>() };
    // SAFETY: the info's module handle is live.
    let exec_info = unsafe { load_bootstrap((*info).module) };

    // SAFETY: `printf` takes the format and no varargs here.
    unsafe { glue::printf(c"task loaded:".as_ptr()) };
    // SAFETY: the vector outlives the thread's argument copy.
    let argv = unsafe { argv_slice((*info).argv) };
    // SAFETY: `exec_info` is live and the vector's strings mapped.
    unsafe { build_args_and_stack(&exec_info, argv, &[]) };
    for &arg in argv {
        // SAFETY: `printf` takes the format and one string.
        unsafe { glue::printf(c" %s".as_ptr(), arg) };
    }

    // SAFETY: runs on the current task.
    let _ = unsafe { task::suspend(current_task()) };

    // SAFETY: the info is live and the lock is the one the waiter holds.
    unsafe {
        (*info).lock.lock();
        (*info).done.store(1, Ordering::Relaxed);
        (*info).lock.unlock();
        sched_prim::thread_wakeup_prim(info.cast(), 0, THREAD_AWAKENED);
    }
    // SAFETY: does not return.
    unsafe { glue::thread_bootstrap_return() };
}

/// `boot_script_exec_cmd()` of `kern/bootstrap.c`: run one parsed command in
/// a fresh thread and wait until it has copied its arguments.
///
/// # Safety
///
/// `hook` must be the module `boot_script_parse_line()` recorded, `task` a
/// live task or null, and `argv` its null-terminated vector.
pub(crate) unsafe fn exec_cmd(
    hook: *mut c_void,
    task: *mut Task,
    argv: *mut *mut c_char,
) {
    if task.is_null() {
        return;
    }

    let mut info = UserBootstrapInfo {
        module: hook,
        argv,
        done: AtomicI32::new(0),
        lock: SimpleLock::new(),
    };
    let info = addr_of_mut!(info);

    // SAFETY: the caller promises the live task, and creation may block.
    let thread = match unsafe { Thread::create(task) } {
        Ok(thread) => thread,
        Err(_) => {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"boot_script_exec_cmd".as_ptr(),
                    c"cannot create the bootstrap thread".as_ptr(),
                )
            }
        }
    };

    // SAFETY: the info is live, and the thread is suspended until the
    // `resume` below.
    unsafe {
        (*info).lock.lock();
        (*thread).saved.other = info.cast();
        (*thread).start(Some(user_bootstrap));
        let _ = Thread::resume(thread);

        // We need to synchronize with the new thread and block this main
        // thread until it has finished referring to our local state.
        while (*info).done.load(Ordering::Relaxed) == 0 {
            sched_prim::thread_sleep(
                info.cast(),
                addr_of_mut!((*info).lock),
                0,
            );
            (*info).lock.lock();
        }
        (*info).lock.unlock();
        Thread::deallocate(thread);
    }
    // SAFETY: `printf` takes the format and no varargs here.
    unsafe { glue::printf(c"\n".as_ptr()) };
}

/// `user_bootstrap_compat()` of `kern/bootstrap.c`: the kernel-mode half of
/// the compat-mode first user thread.
///
/// # Safety
///
/// The thread's `saved.other` must be the module
/// `bootstrap_exec_compat()` published.
unsafe extern "C" fn user_bootstrap_compat() {
    // SAFETY: the launcher stored the module in the thread before start.
    let module = current_thread();
    let module = unsafe { (*module).saved.other };
    // SAFETY: the module handle is live.
    let exec_info = unsafe { load_bootstrap(module) };

    let mut host_buf = [0u8; PORT_STRING_SIZE];
    let host = itoa(&mut host_buf, BOOT_HOST_PORT.load(Ordering::Acquire));
    let mut device_buf = [0u8; PORT_STRING_SIZE];
    let device =
        itoa(&mut device_buf, BOOT_DEVICE_PORT.load(Ordering::Acquire));

    let mut flag_buf = [0u8; 1024];
    let mut root_buf = [0u8; 1024];
    // SAFETY: the boot command line is a live NUL-terminated string.
    let cmdline = unsafe {
        CStr::from_ptr(crate::arch::i386::model_dep::kernel_cmdline)
    }
    .to_bytes();
    get_compat_strings(&mut flag_buf, &mut root_buf, cmdline);

    let argv: [*const c_char; 5] = [
        c"bootstrap".as_ptr(),
        flag_buf.as_ptr().cast(),
        host.as_ptr().cast(),
        device.as_ptr().cast(),
        root_buf.as_ptr().cast(),
    ];
    let envs = [EnvVar {
        name: b"MULTIBOOT_CMDLINE=",
        value: cmdline,
    }];
    let envp: &[EnvVar] = if cmdline.is_empty() { &[] } else { &envs };
    // SAFETY: `exec_info` is live and every string in the vectors outlives
    // the call.
    unsafe { build_args_and_stack(&exec_info, &argv, envp) };

    // SAFETY: does not return.
    unsafe { glue::thread_bootstrap_return() };
}

/// `bootstrap_exec_compat()` of `kern/bootstrap.c`: run the single module in
/// the compat-mode bootstrap task.
///
/// # Safety
///
/// `module` must point at the module data `i386at_init()` kept mapped.
unsafe fn exec_compat(module: *mut MultibootRawModule) {
    // SAFETY: creation may block and the caller holds no locks.
    let task = match unsafe {
        task::create_kernel_task(null_mut(), MapSource::Fresh)
    } {
        Ok(task) => task,
        Err(_) => {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_exec_compat".as_ptr(),
                    c"cannot create the bootstrap task".as_ptr(),
                )
            }
        }
    };
    // SAFETY: the task is live and fresh.
    let _ = unsafe { task::set_name(task, b"bootstrap") };
    // SAFETY: the parent task is live, and creation may block.
    let thread = match unsafe { Thread::create(task) } {
        Ok(thread) => thread,
        Err(_) => {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_exec_compat".as_ptr(),
                    c"cannot create the bootstrap thread".as_ptr(),
                )
            }
        }
    };
    // SAFETY: the thread is live.
    let _ = unsafe { Thread::set_name(thread, c"bootstrap".as_ptr()) };

    // SAFETY: the host object and device port are live, and the fresh task's
    // space receives one send right each.
    unsafe {
        let host_port = make_send((*host::realhost()).host_priv_self);
        BOOT_HOST_PORT
            .store(insert_send_right(task, host_port), Ordering::Release);
        let device_port = make_send(glue::master_device_port);
        BOOT_DEVICE_PORT
            .store(insert_send_right(task, device_port), Ordering::Release);

        (*thread).saved.other = module.cast();
        (*thread).start(Some(user_bootstrap_compat));
    }
    // SAFETY: the thread is live and startable.
    let _ = unsafe { Thread::resume(thread) };
}

/// `bootstrap_create()` of `kern/bootstrap.c`: find the boot script in the
/// multiboot modules and run it.
///
/// # Safety
///
/// Runs once from the boot sequence, after `boot_info` and the IPC layers
/// are live.
pub(crate) unsafe fn create() {
    // SAFETY: `boot_info` is the C global `i386at_init()` filled; the mirror
    // is packed, so every read is a copy.
    let flags = unsafe { crate::arch::i386::model_dep::boot_info.flags };
    // SAFETY: as above.
    let mods_count =
        unsafe { crate::arch::i386::model_dep::boot_info.mods_count };
    // SAFETY: as above.
    let mods_addr =
        unsafe { crate::arch::i386::model_dep::boot_info.mods_addr };
    let mods = kv_ptr_mut::<MultibootRawModule>(phystokv(address(mods_addr)));
    if flags & MULTIBOOT_MODS == 0 || mods_count == 0 {
        // SAFETY: `Panic()` does not return.
        unsafe {
            glue::Panic(
                c"kern/bootstrap.c".as_ptr(),
                line!() as c_int,
                c"bootstrap_create".as_ptr(),
                c"No bootstrap code loaded with the kernel!".as_ptr(),
            )
        };
    }

    // SAFETY: the first module's command line is mapped.
    let first_string =
        kv_ptr::<c_char>(phystokv(address(unsafe { (*mods).string })));
    // SAFETY: the string is NUL-terminated.
    let first_bytes = unsafe { CStr::from_ptr(first_string) }.to_bytes();
    let mut compat = mods_count == 1;
    if compat {
        compat = match first_bytes.iter().position(|&byte| byte == b' ') {
            Some(pos) => first_bytes[pos + 1..]
                .iter()
                .all(|&byte| byte == b' ' || byte == b'\n'),
            None => true,
        };
    }

    if compat {
        // SAFETY: `printf` takes the format and one string.
        unsafe {
            glue::printf(
                c"Loading single multiboot module in compat mode: %s\n"
                    .as_ptr(),
                first_string,
            )
        };
        // SAFETY: the first module is live.
        unsafe { exec_compat(mods) };
    } else {
        // The C leaked these send rights; the port does too.
        let host_priv = unsafe { (*host::realhost()).host_priv_self };
        if !unsafe {
            boot_script::set_variable(
                c"host-port".as_ptr(),
                boot_script::VAL_PORT,
                host_priv.expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable host-port".as_ptr(),
                )
            }
        }
        if !unsafe {
            boot_script::set_variable(
                c"device-port".as_ptr(),
                boot_script::VAL_PORT,
                glue::master_device_port.expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable device-port".as_ptr(),
                )
            }
        }
        // SAFETY: the kernel task is live.
        let itk_self = unsafe { (*crate::kern::task::kernel_task).itk_self };
        if !unsafe {
            boot_script::set_variable(
                c"kernel-task".as_ptr(),
                boot_script::VAL_PORT,
                itk_self.expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable kernel-task".as_ptr(),
                )
            }
        }
        if !unsafe {
            boot_script::set_variable(
                c"kernel-command-line".as_ptr(),
                boot_script::VAL_STR,
                crate::arch::i386::model_dep::kernel_cmdline
                    .expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable kernel-command-line"
                        .as_ptr(),
                )
            }
        }

        // SAFETY: the boot command line is a live NUL-terminated string.
        let cmdline = unsafe {
            CStr::from_ptr(crate::arch::i386::model_dep::kernel_cmdline)
        }
        .to_bytes();
        let mut flag_buf = [0u8; 1024];
        let mut root_buf = [0u8; 1024];
        get_compat_strings(&mut flag_buf, &mut root_buf, cmdline);
        if !unsafe {
            boot_script::set_variable(
                c"boot-args".as_ptr(),
                boot_script::VAL_STR,
                flag_buf.as_ptr().expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable boot-args".as_ptr(),
                )
            }
        }
        if !unsafe {
            boot_script::set_variable(
                c"root-device".as_ptr(),
                boot_script::VAL_STR,
                root_buf.as_ptr().expose_provenance() as c_long,
            )
        } {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"cannot set boot-script variable root-device".as_ptr(),
                )
            }
        }

        // Turn each `FOO=BAR` word in the command line into a boot script
        // variable `${FOO}` with value BAR.  The symbol table keeps pointers
        // into this copy, so it is freed only after `boot_script::exec()`.
        let cmdline_copy = unsafe {
            CStr::from_ptr(crate::arch::i386::model_dep::kernel_cmdline)
        }
        .to_bytes_with_nul();
        let Some(buf) = kalloc(cmdline_copy.len()) else {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"out of memory for the boot script".as_ptr(),
                )
            }
        };
        // SAFETY: the allocation holds `cmdline_copy.len()` bytes.
        unsafe {
            ptr::copy_nonoverlapping(
                cmdline_copy.as_ptr(),
                buf.as_ptr(),
                cmdline_copy.len(),
            )
        };
        let mut words = buf.as_ptr().cast::<c_char>();
        loop {
            // SAFETY: `words` starts at the copy and the NUL keeps the walk
            // inside it.
            let word = unsafe {
                crate::utils::string::strsep(&mut words, c" \t".as_ptr())
            };
            if word.is_null() {
                break;
            }
            // SAFETY: `word` is NUL-terminated inside the copy.
            let eq = unsafe {
                crate::utils::string::strchr(word, c_int::from(b'='))
            };
            if eq.is_null() {
                continue;
            }
            // SAFETY: the copy is writable.
            unsafe { *eq = 0 };
            let value = unsafe { eq.add(1) };
            if !unsafe {
                boot_script::set_variable(
                    word,
                    boot_script::VAL_STR,
                    value.expose_provenance() as c_long,
                )
            } {
                // SAFETY: `Panic()` does not return.
                unsafe {
                    glue::Panic(
                        c"kern/bootstrap.c".as_ptr(),
                        line!() as c_int,
                        c"bootstrap_create".as_ptr(),
                        c"cannot set a boot-script variable from the command line".as_ptr(),
                    )
                }
            }
        }

        let mut losers: c_int = 0;
        let mut i: u32 = 0;
        while i < mods_count {
            let module = unsafe { mods.add(usize::try_from(i).unwrap_or(0)) };
            // SAFETY: the module's command line is mapped.
            let line = kv_ptr_mut::<c_char>(phystokv(address(unsafe {
                (*module).string
            })));
            // SAFETY: `printf` takes the format, one integer and one string.
            unsafe {
                glue::printf(
                    c"module %d: %s\n".as_ptr(),
                    c_int::try_from(i).unwrap_or(0),
                    line,
                )
            };
            // SAFETY: the line is mapped and stays so until `exec()`.
            let result =
                unsafe { boot_script::parse_line(module.cast(), line) };
            if let Err(error) = result {
                // SAFETY: `printf` takes the format and one string.
                unsafe {
                    glue::printf(
                        c"\n\tERROR: %s".as_ptr(),
                        boot_script::error_string(error.code()),
                    )
                };
                losers += 1;
            }
            i += 1;
        }
        // SAFETY: `printf` takes the format and one integer.
        unsafe {
            glue::printf(
                c"%d multiboot modules\n".as_ptr(),
                c_int::try_from(i).unwrap_or(0),
            )
        };
        if losers != 0 {
            // SAFETY: `Panic()` does not return.
            unsafe {
                glue::Panic(
                    c"kern/bootstrap.c".as_ptr(),
                    line!() as c_int,
                    c"bootstrap_create".as_ptr(),
                    c"%d of %d boot script commands could not be parsed"
                        .as_ptr(),
                    losers,
                    c_int::try_from(mods_count).unwrap_or(0),
                )
            }
        }
        // SAFETY: every line parsed is still mapped.
        match unsafe { boot_script::exec() } {
            Ok(()) => (),
            Err(error) => {
                // SAFETY: `Panic()` does not return.
                unsafe {
                    glue::Panic(
                        c"kern/bootstrap.c".as_ptr(),
                        line!() as c_int,
                        c"bootstrap_create".as_ptr(),
                        c"ERROR in executing boot script: %s".as_ptr(),
                        boot_script::error_string(error.code()),
                    )
                }
            }
        }
        // SAFETY: `exec()` freed the symbol table, the last holder of
        // pointers into the copy `kalloc()` made.
        unsafe { kfree(buf, cmdline_copy.len()) };
    }

    let mut n: u32 = 0;
    while n < mods_count {
        let module = unsafe { mods.add(usize::try_from(n).unwrap_or(0)) };
        // SAFETY: the module record is mapped.
        let (start, end) = unsafe {
            (address((*module).mod_start), address((*module).mod_end))
        };
        // SAFETY: the range is the module the boot loader left.
        unsafe { free_bootstrap_pages(start, end) };
        n += 1;
    }
}
