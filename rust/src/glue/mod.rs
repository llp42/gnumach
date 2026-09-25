// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls, and the interface records both halves share.

pub mod mig;
pub mod time_value;

use crate::arch::i386::debug_i386::MachTrap;
use crate::arch::i386::idt::IdtInitEntry;
use crate::arch::i386::irq::{IrqDev, UserIntr};
use crate::arch::i386::model_dep::GdtDescrTmp;
use crate::arch::i386::trap::Recovery;
use crate::arch::types::{VmOffset, VmSize};
use crate::device::ds_routines::DevOps;
use crate::ipc::MachMsgHeader;
use crate::kern::lock::SimpleLock;
use crate::kern::processor::Processor;
use crate::kern::queue::QueueEntry;
use crate::kern::thread::{Continuation, Thread};
use crate::vm::types::{Pmap, VmObject, VmPage, VmProt};
use crate::vm::vm_map::{VmMap, VmMapEntry};
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_ushort, c_void};

// The raw pointers below are to `#[repr(C)]` mirrors.
#[expect(improper_ctypes)]
unsafe extern "C" {
    pub fn Panic(
        file: *const c_char,
        line: c_int,
        fun: *const c_char,
        s: *const c_char,
        ...
    ) -> !;

    pub fn printf(fmt: *const c_char, ...) -> c_int;

    pub fn SoftDebugger(message: *const c_char);

    pub fn snprintf(
        str: *mut c_char,
        size: usize,
        format: *const c_char,
        ...
    ) -> c_int;

    pub fn cngetc() -> c_int;

    pub fn cpu_shutdown();
    pub fn action_thread_continue() -> !;

    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

    pub fn thread_exception_return();
    pub fn thread_handoff(
        self_: *mut Thread,
        continuation: crate::kern::thread::Continuation,
        receiver: *mut Thread,
    ) -> c_int;

    pub fn Thread_continue();

    pub fn Switch_context(
        old: *mut Thread,
        continuation: Continuation,
        new: *mut Thread,
    ) -> *mut Thread;

    pub fn copyin(
        userbuf: *const c_void,
        kernelbuf: *mut c_void,
        cn: usize,
    ) -> c_int;

    pub fn copyout(
        kernelbuf: *const c_void,
        userbuf: *mut c_void,
        cn: usize,
    ) -> c_int;

    /// `copyinmsg()` of `i386/i386/locore.S`: the i386 kernel's message
    /// copy.  The LP64 kernel gets the same symbol from
    /// `src/ipc/copy_user_ffi.rs`.
    #[cfg(target_pointer_width = "32")]
    pub fn copyinmsg(
        userbuf: *const c_void,
        kernelbuf: *mut c_void,
        cn: usize,
        kn: usize,
    ) -> c_int;

    /// `thread_syscall_return()` of <kern/sched_prim.h>: the machine's
    /// syscall-return path, which never comes back to its caller.
    pub fn thread_syscall_return(retval: c_int) -> !;

    /// `syscall` and `syscall64` of `i386/i386/locore.S` and
    /// `x86_64/locore.S`: the entry points `i386/i386/ldt.c` installs.
    pub fn syscall() -> c_int;
    pub fn syscall64() -> c_int;

    /// `inst_fetch()` of `i386/i386/locore.S` and `x86_64/locore.S`: fetch
    /// one instruction byte with the recovery tables' fault handling.
    pub fn inst_fetch(eip: c_int, cs: c_int) -> c_int;

    /// The `copyin`/`copyout` recovery tables of the architecture's
    /// `locore.S`; the `_end` objects mark their ends.
    pub static mut recover_table: Recovery;
    pub static mut recover_table_end: Recovery;
    pub static mut retry_table: Recovery;
    pub static mut retry_table_end: Recovery;

    pub fn mach_notify_new_task(
        notify: *mut c_void,
        task: *mut c_void,
        parent: *mut c_void,
    ) -> c_int;

    pub fn thread_bootstrap_return();
    pub fn Load_context(new: *mut Thread) -> !;

    /// `switch_to_shutdown_context()` of `i386/i386/cswitch.S` and
    /// `x86_64/cswitch.S`: switch to the shutdown stack and call `routine`.
    pub fn switch_to_shutdown_context(
        thread: *mut Thread,
        routine: Option<unsafe extern "C" fn(*mut Processor)>,
        processor: *mut Processor,
    );

    /// `halt_cpu()` of `i386/i386at/model_dep.c`: stop this CPU for good.
    pub fn halt_cpu() -> !;

    /// `compute_mach_factor()` of `kern/mach_factor.c`, which is still C.
    pub fn compute_mach_factor();

    /// `call_continuation()` of `i386/i386/locore.S`, which never returns.
    pub fn call_continuation(continuation: Continuation) -> !;

    /// `avenrun` and `mach_factor` of kern/mach_factor.c: the three load
    /// averages `host_info()` reports.
    pub static avenrun: [c_long; 3];
    pub static mach_factor: [c_long; 3];

    pub static cpu_features: [c_uint; 2];

    /// The load image bounds `pmap_bootstrap()` maps read-only.
    pub static _start: c_char;
    pub static etext: c_char;
    pub static _end: c_char;

    /// `apboot` and `apbootend` of `i386/i386/cpuboot.S`: the AP boot code
    /// `start_other_cpus()` copies to `apboot_addr`.
    pub static apboot: c_char;
    pub static apbootend: c_char;

    /// `gdt_descr_tmp` and `apboot_jmp_offset` of `i386/i386/cpuboot.S`: the
    /// realmode GDT pointer and far jump `machine_init()` relocates.
    pub static mut gdt_descr_tmp: GdtDescrTmp;
    pub static mut apboot_jmp_offset: u32;

    /// `version[]` of the generated version object: the kernel's release
    /// string, printed by `c_boot_entry()`.
    pub static version: c_char;

    /// `mach_trap_table` of `kern/syscall_sw.c`: one entry per syscall, which
    /// `syscall_trace_print()` indexes.
    pub static mach_trap_table: MachTrap;

    /// `idt_inittab[]` of `i386/i386/idt_inittab.S` and
    /// `x86_64/idt_inittab.S`: the generated gate table `idt_fill()` walks.
    pub static mut idt_inittab: IdtInitEntry;

    /// `int_entry_table[]` of `i386/i386/locore.S` and `x86_64/locore.S`:
    /// the generated interrupt entry points `int_fill()` installs.
    pub static int_entry_table: VmOffset;

    /// `return_to_iret` of `i386/i386/locore.S` and `x86_64/locore.S`: the
    /// label `hardclock()` compares an interrupt's return address against.
    pub static return_to_iret: c_char;

    /// `cninit()` of `device/cons.c`: find and initialize the console.
    pub fn cninit();

    /// `discover_x86_cpu_type()` of `i386/i386/locore.S`.
    pub fn discover_x86_cpu_type() -> c_int;

    /// `intr_thread()` of `device/intr.c`, the interrupt service thread.
    pub fn intr_thread();
    pub static mut master_device_port: *mut c_void;

    /// `dev_name_lookup()` of `device/dev_name.c`, which is still C.
    pub fn dev_name_lookup(
        name: *const c_char,
        ops: *mut *mut DevOps,
        unit: *mut c_int,
    ) -> c_int;

    /// `r_memory_object_data_error()` of the MIG `memory_object_reply`
    /// user stubs.
    pub fn r_memory_object_data_error(
        memory_control: *mut c_void,
        offset: VmOffset,
        size: VmSize,
        error_value: c_int,
    ) -> c_int;

    /// `r_memory_object_ready()` of the MIG `memory_object_reply` user
    /// stubs.
    pub fn r_memory_object_ready(
        memory_control: *mut c_void,
        may_cache: c_int,
        copy_strategy: c_int,
    ) -> c_int;
    pub fn insert_intr_entry(
        dev: *mut IrqDev,
        id: c_int,
        receive_port: *mut c_void,
    ) -> *mut UserIntr;
    pub fn install_user_intr_handler(
        dev: *mut IrqDev,
        id: c_int,
        flags: c_ulong,
        entry: *mut UserIntr,
    ) -> c_int;
    pub fn irq_acknowledge(receive_port: *mut c_void) -> c_int;
    pub fn ds_device_open_reply(
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        return_code: c_int,
        device_port: *mut c_void,
    ) -> c_int;
    pub fn ds_device_write_reply(
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        return_code: c_int,
        bytes_written: c_int,
    ) -> c_int;
    pub fn ds_device_write_reply_inband(
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        return_code: c_int,
        bytes_written: c_int,
    ) -> c_int;
    pub fn ds_device_read_reply(
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        return_code: c_int,
        data: *mut c_char,
        data_count: c_uint,
    ) -> c_int;
    pub fn ds_device_read_reply_inband(
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        return_code: c_int,
        data: *mut c_char,
        data_count: c_uint,
    ) -> c_int;

    pub fn spl0() -> c_int;
    pub fn splimp() -> c_int;
    pub fn splhi() -> c_int;
    pub fn splsched() -> c_int;
    pub fn spltty() -> c_int;
    pub fn splsoftclock() -> c_int;
    pub fn splclock() -> c_int;
    pub fn splhigh() -> c_int;
    pub fn splvm() -> c_int;
    pub fn splx(level: c_int) -> c_int;
    pub fn sploff() -> c_ulong;
    pub fn splon(n: c_ulong);

    /// `main_intr_queue` of <device/intr.h>: the queue `irqtab` points at.
    pub static mut main_intr_queue: QueueEntry;

    pub fn configure_bus_master(
        name: *const c_char,
        virt: VmOffset,
        phys: VmOffset,
        adpt_no: c_int,
        bus_name: *const c_char,
    ) -> c_int;

    pub fn configure_bus_device(
        name: *const c_char,
        virt: VmOffset,
        phys: VmOffset,
        adpt_no: c_int,
        bus_name: *const c_char,
    ) -> c_int;

    /// `setsoftclock()` of <i386/spl.h>: raise the softclock interrupt.
    pub fn setsoftclock();

    /// `thread_quantum_update()` of <kern/priority.h>: charge the quantum.
    pub fn thread_quantum_update(
        mycpu: c_int,
        thread: *mut Thread,
        nticks: c_int,
        state: c_int,
    );

    pub fn ipc_kobject_server(kmsg: *mut c_void) -> *mut c_void;

    pub fn ipc_kobject_destroy(port: *mut c_void);
    pub fn ipc_kobject_set_locked(
        port: *mut c_void,
        kobject: VmOffset,
        type_: c_uint,
    );

    pub fn ipc_kobject_set(
        port: *mut c_void,
        kobject: VmOffset,
        type_: c_uint,
    );

    /// The `*_server_routines[]` tables the generated `*.server.h` headers
    /// declare, one per MIG subsystem `ipc_kobject_server()` dispatches to.
    /// Each is declared as its first element, as the C header declares the
    /// array.
    pub(crate) static mut mach_server_routines: MigRoutine;
    pub(crate) static mut mach_port_server_routines: MigRoutine;
    pub(crate) static mut mach_host_server_routines: MigRoutine;
    pub(crate) static mut device_server_routines: MigRoutine;
    pub(crate) static mut device_pager_server_routines: MigRoutine;
    pub(crate) static mut mach_debug_server_routines: MigRoutine;
    pub(crate) static mut mach4_server_routines: MigRoutine;
    pub(crate) static mut gnumach_server_routines: MigRoutine;
    pub(crate) static mut experimental_server_routines: MigRoutine;
    pub(crate) static mut mach_i386_server_routines: MigRoutine;
    pub fn pmap_destroy(pmap: *mut Pmap);
    pub fn pmap_collect(pmap: *mut Pmap);
    pub fn pmap_reference(pmap: *mut Pmap);
    pub static kernel_pmap: *mut Pmap;
    pub fn pmap_extract(pmap: *mut Pmap, address: VmOffset) -> VmOffset;
    pub fn pmap_map_bd(
        virt: VmOffset,
        start: VmOffset,
        end: VmOffset,
        prot: VmProt,
    ) -> VmOffset;
    pub fn pmap_virtual_space(startp: *mut VmOffset, endp: *mut VmOffset);
    pub fn pmap_enter(
        pmap: *mut Pmap,
        va: VmOffset,
        pa: VmOffset,
        protection: VmProt,
        wired: c_int,
    );

    pub fn vm_page_mem_size() -> VmSize;
    pub fn vm_page_bootalloc(size: VmSize) -> VmOffset;
    pub fn vm_page_check(page: *const VmPage);
    pub fn vm_page_queues_remove(page: *mut VmPage);
    pub fn vm_page_alloc_pa(
        order: c_uint,
        selector: c_uint,
        type_: c_ushort,
    ) -> *mut VmPage;
    pub fn vm_page_free_pa(page: *mut VmPage, order: c_uint);
    pub fn vm_page_lookup_pa(pa: VmOffset) -> *mut VmPage;

    pub fn pmap_page_protect(pa: VmOffset, prot: c_int);
    pub fn pmap_clear_modify(pa: VmOffset);
    pub fn pmap_is_modified(pa: VmOffset) -> c_int;
    pub fn pmap_clear_reference(pa: VmOffset);
    pub fn pmap_is_referenced(pa: VmOffset) -> c_int;

    pub fn vm_object_collect(object: *mut VmObject);

    pub fn memory_manager_default_port(port: *mut c_void) -> c_int;
    pub static mut memory_manager_default: *mut c_void;
    pub fn memory_manager_default_reference() -> *mut c_void;
    /// `memory_object_data_request()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_data_request(
        memory_object: *mut c_void,
        memory_control: *mut c_void,
        offset: VmOffset,
        length: VmSize,
        desired_access: VmProt,
    ) -> c_int;

    /// `memory_object_data_unlock()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_data_unlock(
        memory_object: *mut c_void,
        memory_control: *mut c_void,
        offset: VmOffset,
        length: VmSize,
        desired_access: VmProt,
    ) -> c_int;

    /// `memory_object_data_return()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_data_return(
        memory_object: *mut c_void,
        memory_control: *mut c_void,
        offset: VmOffset,
        data: VmOffset,
        data_cnt: c_uint,
        dirty: c_int,
        kernel_copy: c_int,
    ) -> c_int;

    /// `memory_object_lock_completed()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_lock_completed(
        memory_object: *mut c_void,
        memory_object_poly: c_uint,
        memory_control: *mut c_void,
        offset: VmOffset,
        length: VmSize,
    ) -> c_int;

    /// `memory_object_supply_completed()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_supply_completed(
        memory_object: *mut c_void,
        memory_object_poly: c_uint,
        memory_control: *mut c_void,
        offset: VmOffset,
        length: VmSize,
        result: c_int,
        error_offset: VmOffset,
    ) -> c_int;

    /// `memory_object_change_completed()` of the MIG `memory_object_user`
    /// stubs.
    pub fn memory_object_change_completed(
        memory_object: *mut c_void,
        memory_object_poly: c_uint,
        may_cache: c_int,
        copy_strategy: c_int,
    ) -> c_int;

    pub fn memory_object_init(
        pager: *mut c_void,
        pager_request: *mut c_void,
        pager_name: *mut c_void,
        page_size: VmSize,
    ) -> c_int;
    pub fn memory_object_create(
        memory_object: *mut c_void,
        pager: *mut c_void,
        size: VmSize,
        pager_request: *mut c_void,
        pager_name: *mut c_void,
        page_size: VmSize,
    ) -> c_int;
    pub fn memory_object_copy(
        memory_object: *mut c_void,
        pager_request: *mut c_void,
        offset: VmOffset,
        size: VmSize,
        new_memory_object: *mut c_void,
    ) -> c_int;
    pub fn memory_object_terminate(
        pager: *mut c_void,
        pager_request: *mut c_void,
        pager_name: *mut c_void,
    ) -> c_int;

    pub static mut vm_page_fictitious_addr: VmOffset;
    pub fn vm_page_grab_fictitious() -> *mut VmPage;
    pub fn vm_page_insert(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_remove(page: *mut VmPage);
    pub fn vm_page_wait(continuation: Option<unsafe extern "C" fn()>);
    pub fn vm_page_more_fictitious();
    pub fn vm_page_replace(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_activate(page: *mut VmPage);

    pub static mut vm_page_queue_lock: SimpleLock;
    pub static mut vm_page_queue_free_lock: SimpleLock;
    pub fn vm_page_free(page: *mut VmPage);

    /// `memory_object_data_initialize()` of the MIG
    /// `memory_object_default` stubs.
    pub fn memory_object_data_initialize(
        memory_object: *mut c_void,
        memory_control: *mut c_void,
        offset: VmOffset,
        data: VmOffset,
        data_cnt: c_uint,
    ) -> c_int;

    pub fn vm_page_lookup(
        object: *mut VmObject,
        offset: VmOffset,
    ) -> *mut VmPage;

    pub fn vm_object_reference(object: *mut VmObject);
    pub fn vm_object_deallocate(object: *mut VmObject);
    pub fn memory_object_lock_request(
        object: *mut VmObject,
        offset: VmOffset,
        size: VmSize,
        should_return: c_int,
        should_flush: c_int,
        prot: VmProt,
        reply_to: *mut c_void,
        reply_to_type: c_uint,
    ) -> c_int;
    pub fn vm_object_page_remove(
        object: *mut VmObject,
        start: VmOffset,
        end: VmOffset,
    );
    pub fn vm_object_pmap_remove(
        object: *mut VmObject,
        start: VmOffset,
        end: VmOffset,
    );
    pub fn vm_object_coalesce(
        prev_object: *mut VmObject,
        next_object: *mut VmObject,
        prev_offset: VmOffset,
        next_offset: VmOffset,
        prev_size: VmSize,
        next_size: VmSize,
        new_object: *mut *mut VmObject,
        new_offset: *mut VmOffset,
    ) -> c_int;
    pub static kernel_object: *mut VmObject;
    pub static kernel_map: *mut c_void;
    pub fn kmem_alloc_wired(
        map: *mut VmMap,
        addrp: *mut VmOffset,
        size: VmSize,
    ) -> c_int;
    pub static kernel_virtual_start: VmOffset;
    pub static kernel_virtual_end: VmOffset;
    pub fn pmap_remove(pmap: *mut Pmap, start: VmOffset, end: VmOffset);
    pub fn pmap_protect(
        pmap: *mut Pmap,
        start: VmOffset,
        end: VmOffset,
        prot: c_int,
    );
    pub fn pmap_zero_page(pa: VmOffset);
    pub fn pmap_copy_page(src: VmOffset, dst: VmOffset);
    pub fn vm_fault_unwire(map: *mut c_void, entry: *mut c_void);
    pub fn vm_fault(
        map: *mut VmMap,
        va: VmOffset,
        protection: VmProt,
        change_wiring: c_int,
        resume: c_int,
        continuation: Option<unsafe extern "C" fn(c_int)>,
    ) -> c_int;
    pub fn vm_fault_wire_fast(
        map: *mut VmMap,
        va: VmOffset,
        entry: *mut VmMapEntry,
    ) -> c_int;

    pub fn vm_fault_page(
        first_object: *mut VmObject,
        first_offset: VmOffset,
        fault_type: VmProt,
        must_be_resident: c_int,
        interruptible: c_int,
        protection: *mut VmProt,
        result_page: *mut *mut VmPage,
        top_page: *mut *mut VmPage,
        resume: c_int,
        continuation: Option<unsafe extern "C" fn()>,
    ) -> c_int;

    pub fn vm_fault_cleanup(object: *mut VmObject, top_page: *mut VmPage);

    pub fn vm_fault_copy(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        src_size: *mut VmSize,
        dst_object: *mut VmObject,
        dst_offset: VmOffset,
        dst_map: *mut c_void,
        dst_version: *mut c_void,
        interruptible: c_int,
    ) -> c_int;

    pub fn vm_object_shadow(
        object: *mut *mut VmObject,
        offset: *mut VmOffset,
        length: VmSize,
    );
    pub fn vm_object_allocate(size: VmSize) -> *mut VmObject;
    pub fn vm_object_collapse(object: *mut VmObject);
    pub fn vm_object_copy_slowly(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        size: VmSize,
        interruptible: c_int,
        result_object: *mut *mut VmObject,
    ) -> c_int;
    pub fn vm_object_copy_strategically(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        size: VmSize,
        dst_object: *mut *mut VmObject,
        dst_offset: *mut VmOffset,
        dst_needs_copy: *mut c_int,
    ) -> c_int;
    pub fn vm_object_copy_temporary(
        object: *mut *mut VmObject,
        offset: *mut VmOffset,
        src_needs_copy: *mut c_int,
        dst_needs_copy: *mut c_int,
    ) -> c_int;
    pub fn vm_object_pmap_protect(
        object: *mut VmObject,
        offset: VmOffset,
        size: VmSize,
        pmap: *mut Pmap,
        start: VmOffset,
        protection: c_int,
    );
    pub fn vm_object_name(object: *mut VmObject) -> *mut c_void;
    pub fn vm_object_pager_create(object: *mut VmObject);

    pub fn pmap_create(size: VmSize) -> *mut Pmap;

    pub fn vm_page_bootstrap(startp: *mut VmOffset, endp: *mut VmOffset);
    pub fn vm_page_info_all();

    pub fn vm_object_bootstrap();
    pub fn vm_object_init();

    pub fn pmap_init();

    pub fn vm_fault_init();

    pub fn memory_manager_default_init();
}

/// `mig_routine_t` of <mach/mig.h>: one generated MIG server entry point.
pub(crate) type MigRoutine =
    Option<unsafe extern "C" fn(*mut MachMsgHeader, *mut MachMsgHeader)>;
