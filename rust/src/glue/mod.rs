// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls, and the interface records both halves share.

pub mod mig;
pub mod time_value;

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::Timeout;
use crate::kern::machine::MachineSlot;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::QueueEntry;
use crate::kern::sched::RunQueue;
use crate::kern::sched_prim::NUMQUEUES;
use crate::kern::thread::{Continuation, StackResume, Thread};
use crate::vm::types::{Pmap, VmObject, VmPage, VmProt};
use crate::vm::vm_map::{VmMap, VmMapEntry};
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_void};
use core::mem::offset_of;

/// `NSPEEDS` of <device/tty_status.h>: how many baud-rate slots `ttlowat[]`
/// and `tthiwat[]` are indexed by.
pub const NSPEEDS: usize = 18;

/// `struct ldisc_switch` of <device/tty.h>: the entry points one line
/// discipline provides.
#[repr(C)]
pub struct LdiscSwitch {
    pub l_read:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    pub l_write:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    /// `l_rint`: feed one received character to the discipline.
    pub l_rint: Option<unsafe extern "C" fn(c_uint, *mut c_void)>,
    /// `l_modem`: report a modem carrier change.
    pub l_modem: Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>,
    /// `l_start`: restart stalled output.
    pub l_start: Option<unsafe extern "C" fn(*mut c_void)>,
}

const _: () = {
    const PTR: usize = size_of::<*const c_void>();
    assert!(size_of::<LdiscSwitch>() == 5 * PTR);
    assert!(align_of::<LdiscSwitch>() == align_of::<*const c_void>());
    assert!(offset_of!(LdiscSwitch, l_read) == 0);
    assert!(offset_of!(LdiscSwitch, l_write) == PTR);
    assert!(offset_of!(LdiscSwitch, l_rint) == 2 * PTR);
    assert!(offset_of!(LdiscSwitch, l_modem) == 3 * PTR);
    assert!(offset_of!(LdiscSwitch, l_start) == 4 * PTR);
};

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

    pub fn cngetc() -> c_int;

    pub fn timeout(
        fcn: Option<unsafe extern "C" fn(*mut c_void)>,
        param: *mut c_void,
        interval: c_int,
    ) -> *mut c_void;
    pub static mut time: time_value::TimeValue64;

    pub static mut clock_boottime_offset: time_value::TimeValue64;

    pub static mut timedelta: c_int;
    pub static mut tickdelta: c_int;
    pub static mut tickadj: c_uint;
    pub static mut bigadj: c_uint;

    pub fn host_set_time64(
        host: *mut c_void,
        new_time: time_value::TimeValue64,
    ) -> c_int;

    pub static mtime: *mut time_value::MappedTimeValue;

    pub fn cpu_shutdown();
    pub fn action_thread_continue() -> !;

    pub fn halt_all_cpus(reboot: c_int) -> !;

    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

    pub fn assert_wait(event: *mut c_void, interruptible: c_int);
    pub fn thread_block(continuation: Option<unsafe extern "C" fn()>);
    pub fn update_priority(thread: *mut Thread);
    pub fn rem_runq(th: *mut Thread) -> *mut RunQueue;
    pub fn thread_exception_return() -> !;
    pub fn stack_attach(
        thread: *mut Thread,
        stack: VmOffset,
        continuation: StackResume,
    );
    pub fn stack_finalize(stack: VmOffset);

    pub fn thread_set_syscall_return(thread: *mut Thread, retval: c_int);
    pub fn thread_setstatus(
        thread: *mut Thread,
        flavor: c_int,
        tstate: *mut c_uint,
        count: c_uint,
    ) -> c_int;
    pub fn thread_getstatus(
        thread: *mut Thread,
        flavor: c_int,
        tstate: *mut c_uint,
        count: *mut c_uint,
    ) -> c_int;

    pub fn copyin(
        userbuf: *const c_void,
        kernelbuf: *mut c_void,
        cn: usize,
    ) -> c_int;

    pub fn thread_assign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
    ) -> c_int;
    pub fn task_assign(
        task: *mut c_void,
        new_pset: *mut ProcessorSet,
        assign_threads: c_int,
    ) -> c_int;

    pub fn task_create_kernel(
        parent_task: *mut c_void,
        inherit_memory: c_int,
        child_task: *mut *mut c_void,
    ) -> c_int;
    pub fn task_terminate(task: *mut c_void) -> c_int;
    pub fn task_deallocate(task: *mut c_void);
    pub static mut new_task_notification: *mut c_void;

    pub fn thread_halt_self(continuation: Continuation);

    pub fn thread_freeze(thread: *mut Thread);
    pub fn thread_doassign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
        release_freeze: c_int,
    );
    pub fn thread_halt(thread: *mut Thread, must_halt: c_int) -> c_int;
    pub fn thread_deallocate(thread: *mut Thread);
    pub fn thread_dowait(thread: *mut Thread, must_halt: c_int) -> c_int;

    pub fn ipc_thread_terminate(thread: *mut Thread);

    pub fn evc_notify_abort(thread: *mut Thread);

    pub fn reset_timeout(t: *mut c_void) -> c_int;

    pub fn set_timeout(t: *mut Timeout, interval: c_uint);

    pub fn smp_remote_ast(logical_id: c_uint);
    pub fn smp_pmap_update(logical_id: c_uint);

    pub static mut sched_tick: c_uint;
    pub static mut recompute_priorities_timer: Timeout;
    pub static mut sched_thread_id: *mut Thread;

    pub static mut wait_queue: [QueueEntry; NUMQUEUES];
    pub static mut wait_lock: [SimpleLock; NUMQUEUES];

    pub static mut thread_cache: c_void;
    pub static mut thread_stack_cache: c_void;
    pub static mut thread_template: Thread;
    pub static mut reaper_queue: QueueEntry;
    pub static mut reaper_lock: SimpleLock;
    pub static mut stack_lock_data: SimpleLock;
    pub static mut stack_usage_lock: SimpleLock;
    pub static mut stack_free_list: VmOffset;
    pub static mut stack_free_count: c_uint;
    pub static mut stack_free_limit: c_uint;
    pub static mut stack_check_usage: c_int;
    pub fn thread_bootstrap_return();
    pub fn pcb_module_init();
    pub fn switch_ktss(pcb: *mut c_void);
    pub fn Load_context(new: *mut Thread) -> !;

    pub fn thread_glue_pset_sched_load(pset: *mut ProcessorSet) -> c_long;

    pub static mut min_quantum: c_int;

    pub fn processor_glue_pset_tail_init(
        pset: *mut ProcessorSet,
        quantum: c_int,
    );

    pub fn processor_glue_pset_machine_quantum(
        pset: *mut ProcessorSet,
    ) -> *mut c_int;

    pub fn processor_glue_pset_mach_factor(pset: *mut ProcessorSet) -> c_long;
    pub fn processor_glue_pset_load_average(pset: *mut ProcessorSet)
    -> c_long;

    pub static mut all_psets: QueueEntry;
    pub static mut all_psets_lock: SimpleLock;
    pub static mut all_psets_count: c_int;

    pub static mut master_processor: *mut Processor;

    pub static mut machine_slot: MachineSlot;

    pub static mut default_pset: c_void;
    pub static mut pset_cache: c_void;

    pub static mut realhost: c_void;

    pub fn processor_shutdown(processor: *mut Processor) -> c_int;

    pub fn iodone(ior: *mut c_void);
    pub fn device_read_alloc(ior: *mut c_void, size: usize) -> c_int;
    pub fn ds_read_done(ior: *mut c_void) -> c_int;

    pub fn mach_device_init();
    pub fn dev_lookup_init();
    pub fn net_io_init();
    pub fn device_pager_init();
    pub fn chario_init();
    pub fn io_done_thread();
    pub fn net_thread();
    pub fn kernel_thread(
        task: *mut c_void,
        name: *const c_char,
        start: Option<unsafe extern "C" fn()>,
        arg: *mut c_void,
    ) -> *mut c_void;
    pub fn ds_device_open(
        open_port: *mut c_void,
        reply_port: *mut c_void,
        reply_port_type: c_uint,
        mode: c_uint,
        name: *const c_char,
        devp: *mut *mut c_void,
    ) -> c_int;

    pub static mut master_device_port: *mut c_void;
    pub static kernel_task: *mut c_void;
    pub static ipc_space_kernel: *mut c_void;

    pub fn spl0() -> c_int;
    pub fn splhi() -> c_int;
    pub fn splsched() -> c_int;
    pub fn spltty() -> c_int;
    pub fn splsoftclock() -> c_int;
    pub fn splclock() -> c_int;
    pub fn splhigh() -> c_int;
    pub fn splx(level: c_int) -> c_int;
    pub fn sploff() -> c_ulong;
    pub fn splon(n: c_ulong);

    pub fn comgetc(unit: c_int) -> c_int;

    pub fn ttychars(tp: *mut c_void);
    pub fn char_open(
        dev: c_int,
        tp: *mut c_void,
        mode: c_int,
        ior: *mut c_void,
    ) -> c_int;
    pub fn ttyclose(tp: *mut c_void);
    pub fn tty_get_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: *mut u32,
    ) -> c_int;
    pub fn tty_set_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: u32,
    ) -> c_int;
    pub fn tty_portdeath(tp: *mut c_void, port: *mut c_void) -> c_int;

    pub static linesw: [LdiscSwitch; 1];
    pub static ttlowat: [c_short; NSPEEDS];

    pub static hz: c_int;
    pub static rebootflag: c_int;
    pub static tick: c_int;

    pub static pic_mode: c_int;

    pub fn irq_mask(irq: c_uint);
    pub fn irq_unmask(irq: c_uint);
    pub fn irq_set_handler(
        irq: c_int,
        handler: Option<unsafe extern "C" fn(c_int)>,
    );
    pub fn irq_get_handler(irq: c_int) -> Option<unsafe extern "C" fn(c_int)>;
    pub fn irq_set_unit(irq: c_int, unit: c_int);
    pub fn irq_get_unit(irq: c_int) -> c_int;

    pub fn com_base_addr(unit: c_int) -> VmOffset;
    pub fn com_irq(unit: c_int) -> c_int;

    pub static mut lapic: *mut c_void;
    pub static mut cpu_id_lut: [c_int; 256];
    pub static mut apic_id_mask: u8;

    pub static mut hpet_addr: *mut u32;
    pub static mut hpet_period_nsec: u32;

    pub fn biosmem_addr_available(addr: VmOffset) -> c_int;

    pub fn biosmem_bootalloc(nr_pages: c_uint) -> c_ulong;

    pub fn kmem_cache_alloc(cache: *mut c_void) -> VmOffset;
    pub fn kmem_cache_free(cache: *mut c_void, obj: VmOffset);
    pub fn kmem_cache_init(
        cache: *mut c_void,
        name: *const c_char,
        size: VmSize,
        align: VmSize,
        ctor: Option<unsafe extern "C" fn(*mut c_void)>,
        flags: c_int,
    );

    pub static mut ifps_cache: c_void;

    pub fn fp_load(thread: *mut Thread);

    pub static mut machine_task_iopb_cache: c_void;

    pub fn kalloc(size: VmSize) -> VmOffset;
    pub fn kfree(data: VmOffset, size: VmSize);

    pub fn ipc_port_copy_send(port: *mut c_void) -> *mut c_void;
    pub fn ipc_port_release_send(port: *mut c_void);

    pub fn ipc_port_release_receive(port: *mut c_void);
    pub fn ipc_port_init(port: *mut c_void, space: *mut c_void, name: c_uint);
    pub fn ipc_notify_send_once(port: *mut c_void);

    pub fn convert_processor_name_to_port(
        processor: *mut Processor,
    ) -> *mut c_void;

    pub fn ipc_kobject_set(
        port: *mut c_void,
        kobject: VmOffset,
        type_: c_uint,
    );

    pub fn ipc_port_alloc_special(space: *mut c_void) -> *mut c_void;
    pub fn ipc_port_dealloc_special(port: *mut c_void, space: *mut c_void);

    pub static ipc_space_reply: *mut c_void;

    pub fn mach_reply_port() -> c_uint;

    pub fn ipc_object_rename(
        space: *mut c_void,
        old_name: c_uint,
        new_name: c_uint,
    ) -> c_int;

    pub fn ipc_object_alloc(
        space: *mut c_void,
        otype: c_uint,
        type_: c_uint,
        urefs: c_uint,
        namep: *mut c_uint,
        objectp: *mut *mut c_void,
    ) -> c_int;
    pub fn ipc_object_alloc_name(
        space: *mut c_void,
        otype: c_uint,
        type_: c_uint,
        urefs: c_uint,
        name: c_uint,
        objectp: *mut *mut c_void,
    ) -> c_int;
    pub fn ipc_object_copyout_name(
        space: *mut c_void,
        object: *mut c_void,
        msgt_name: c_uint,
        overflow: c_int,
        name: c_uint,
    ) -> c_int;
    pub fn ipc_object_copyin(
        space: *mut c_void,
        name: c_uint,
        msgt_name: c_uint,
        objectp: *mut *mut c_void,
    ) -> c_int;

    pub fn ipc_object_translate(
        space: *mut c_void,
        name: c_uint,
        right: c_uint,
        objectp: *mut *mut c_void,
    ) -> c_int;

    pub static mut ipc_kernel_map: *mut c_void;
    pub static ipc_kernel_map_size: VmSize;
    pub fn ipc_host_init();

    pub fn ipc_port_pdrequest(
        port: *mut c_void,
        notify: *mut c_void,
        previousp: *mut *mut c_void,
    );
    pub fn ipc_port_nsrequest(
        port: *mut c_void,
        sync: c_uint,
        notify: *mut c_void,
        previousp: *mut *mut c_void,
    );

    pub fn ipc_right_dnrequest(
        space: *mut c_void,
        name: c_uint,
        immediate: c_int,
        notify: *mut c_void,
        previousp: *mut *mut c_void,
    ) -> c_int;

    pub static mut vm_map_cache: c_void;
    pub static mut vm_map_entry_cache: c_void;
    pub static mut vm_map_copy_cache: c_void;
    pub static mut vm_submap_object: *mut VmObject;

    pub static mut vm_external_cache: c_void;
    pub static mut vm_object_small_existence_map_cache: c_void;
    pub static mut vm_object_large_existence_map_cache: c_void;

    pub fn projected_buffer_deallocate(
        map: *mut VmMap,
        start: VmOffset,
        end: VmOffset,
    ) -> c_int;
    pub fn kmem_valloc(
        map: *mut VmMap,
        addrp: *mut VmOffset,
        size: VmSize,
    ) -> c_int;
    pub fn kmem_alloc_pages(
        object: *mut VmObject,
        offset: VmOffset,
        start: VmOffset,
        end: VmOffset,
        protection: VmProt,
        flags: c_uint,
    );

    pub fn pmap_destroy(pmap: *mut Pmap);
    pub fn pmap_reference(pmap: *mut Pmap);
    pub static kernel_pmap: *mut Pmap;
    pub fn pmap_pte(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset;
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
    pub fn vm_page_grab(flags: c_uint) -> *mut VmPage;
    pub fn vm_page_insert(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_remove(page: *mut VmPage);
    pub static mut virtual_space_start: VmOffset;
    pub static mut virtual_space_end: VmOffset;
    pub fn vm_page_copy(src: *mut VmPage, dst: *mut VmPage);
    pub fn vm_page_wait(continuation: Option<unsafe extern "C" fn()>);
    pub fn vm_page_more_fictitious();
    pub fn vm_page_replace(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_wire(page: *mut VmPage);
    pub fn vm_page_activate(page: *mut VmPage);

    pub static mut vm_page_queue_lock: SimpleLock;
    pub fn vm_page_free(page: *mut VmPage);

    pub fn vm_map_glue_object_lock(object: *mut VmObject);
    pub fn vm_map_glue_object_unlock(object: *mut VmObject);
    pub fn vm_map_glue_object_can_release(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_is_pristine_submap(
        object: *mut VmObject,
    ) -> c_int;
    pub fn vm_map_glue_object_needs_shadow(
        object: *mut VmObject,
        size: VmSize,
        needs_copy: c_int,
        is_shared: c_int,
    ) -> c_int;
    pub fn vm_map_glue_object_is_temporary(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_is_shadowed(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_use_shared_copy(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_make_shared(object: *mut VmObject);
    pub fn vm_map_glue_object_paging_begin(object: *mut VmObject);
    pub fn vm_map_glue_object_paging_end(object: *mut VmObject);
    pub fn vm_map_glue_object_can_coalesce(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_extend_size(object: *mut VmObject, size: VmSize);
    pub fn vm_map_glue_page_is_absent(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_tabled(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_busy(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_fictitious(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_error(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_precious(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_object(page: *mut VmPage) -> *mut VmObject;
    pub fn vm_map_glue_page_steal(page: *mut VmPage);
    pub fn vm_map_glue_page_protect(page: *mut VmPage, protection: c_int);
    pub fn vm_map_glue_page_set_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_clear_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_set_dirty(page: *mut VmPage);
    pub fn vm_map_glue_page_wakeup_done(page: *mut VmPage);
    pub fn vm_map_glue_page_activate_if_idle(page: *mut VmPage);
    pub fn vm_map_glue_page_wire_count(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_offset(page: *mut VmPage) -> VmOffset;
    pub fn vm_map_glue_pmap_enter(
        pmap: *mut Pmap,
        addr: VmOffset,
        page: *mut VmPage,
        protection: c_int,
        wired: c_int,
    );

    pub fn vm_map_glue_object_pager(object: *mut VmObject) -> *mut c_void;
    pub fn vm_map_glue_task_map(task: *mut c_void) -> *mut c_void;
    pub fn vm_map_glue_task_space(task: *mut c_void) -> *mut c_void;

    pub fn memory_object_create_proxy(
        task: *mut c_void,
        max_protection: c_int,
        object: *mut *mut c_void,
        object_count: c_uint,
        offset: *mut VmOffset,
        offset_count: c_uint,
        start: *mut VmOffset,
        start_count: c_uint,
        len: *mut VmSize,
        len_count: c_uint,
        proxy: *mut *mut c_void,
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
    pub static kernel_virtual_start: VmOffset;
    pub static kernel_virtual_end: VmOffset;
    pub fn pmap_remove(pmap: *mut Pmap, start: VmOffset, end: VmOffset);
    pub fn pmap_protect(
        pmap: *mut Pmap,
        start: VmOffset,
        end: VmOffset,
        prot: c_int,
    );
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
    pub fn vm_page_module_init();
    pub fn vm_page_info_all();

    pub fn slab_bootstrap();
    pub fn slab_init();

    pub fn vm_object_bootstrap();
    pub fn vm_object_init();

    pub fn pmap_init();

    pub fn kalloc_init();

    pub fn vm_fault_init();

    pub fn memory_manager_default_init();
    pub fn memory_object_proxy_init();
}
