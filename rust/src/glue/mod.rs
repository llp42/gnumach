// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls, and the interface records both halves
//! share.
//!
//! C *macros* cannot come through here; when Rust needs one, it gets a
//! small C shim function beside the header that defines it, and that
//! shim is declared below like any other C function.
//!
//! [`mig`] holds the conversions between the Rust error codes and the
//! result codes the C side passes, and [`time_value`] the time records
//! of <mach/time_value.h>.

pub mod mig;
pub mod time_value;

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::lock::SimpleLock;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::NUMQUEUES;
use crate::kern::thread::Thread;
use crate::kern::timer::{Timer, TimerSave};
use crate::vm::types::{Pmap, VmObject, VmPage, VmProt};
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_void};

// The raw pointers below are to `#[repr(C)]` mirrors.  `QueueEntry`
// ends in the zero-sized `PhantomPinned` marker, which the FFI lint
// treats as poison even behind a pointer; the C side passes the same
// pointers, and every layout is asserted in its own module.
#[expect(improper_ctypes)]
unsafe extern "C" {
    pub fn Panic(
        file: *const c_char,
        line: c_int,
        fun: *const c_char,
        s: *const c_char,
        ...
    ) -> !;

    // <kern/printf.h>
    pub fn printf(fmt: *const c_char, ...) -> c_int;

    // <kern/mach_clock.h>
    pub fn timeout(
        fcn: Option<unsafe extern "C" fn(*mut c_void)>,
        param: *mut c_void,
        interval: c_int,
    ) -> *mut c_void;

    // <kern/machine.c>
    pub fn cpu_shutdown();

    // <i386at/kd.h>, the screen block moves in kdasm.S
    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

    // <kern/sched_prim.h>
    pub fn wakeup(channel: VmOffset);
    pub fn assert_wait(event: *mut c_void, interruptible: c_int);
    pub fn thread_block(continuation: Option<unsafe extern "C" fn()>);
    // The C half of the scheduler still owns these; the Rust port
    // calls them.
    pub fn update_priority(thread: *mut Thread);
    pub fn stack_free(thread: *mut Thread);

    // <kern/syscall_subr.h>: the priority-depression timeout, stored
    // in `thread.depress_timer.fcn`.
    pub fn thread_depress_timeout(thread: *mut c_void);

    // <kern/mach_clock.h>: the wait timeout, set under the thread
    // lock.  The handle is opaque here; its layout lives in
    // rust/src/kern/mach_clock.rs, which is GPL-derived.
    pub fn set_timeout(t: *mut c_void, interval: c_uint);
    pub fn reset_timeout(t: *mut c_void) -> c_int;

    // <kern/ast.h>: `cause_ast_check()` is a function; the `ast_on()`
    // family lives in rust/src/kern/ast.rs.
    pub fn cause_ast_check(processor: *mut Processor);

    // <kern/sched_prim.c>: `unsigned sched_tick`, the second counter
    // priorities age against.
    pub static mut sched_tick: c_uint;

    // <kern/sched_prim.c>: the event hash table.  `wait_queue_init()`
    // stays C and builds these; the Rust port hashes exactly as
    // `wait_hash()` does.  `wait_lock` was `static` in C and is
    // exported for the port.
    pub static mut wait_queue: [QueueEntry; NUMQUEUES];
    pub static mut wait_lock: [SimpleLock; NUMQUEUES];

    // <kern/thread.c>: the module state `thread_init()` builds and the
    // C half goes on using.
    pub static mut thread_cache: c_void;
    pub static mut thread_stack_cache: c_void;
    pub static mut thread_template: Thread;
    pub static mut reaper_queue: QueueEntry;
    pub static mut reaper_lock: SimpleLock;
    pub static mut stack_lock_data: SimpleLock;
    pub static mut stack_usage_lock: SimpleLock;
    // <kern/sched_prim.h>: where a fresh thread starts execution.
    pub fn thread_bootstrap_return();
    // <i386/i386/pcb.h>
    pub fn pcb_module_init();

    // <kern/timer.h>: the coherency slow path of the timer-delta
    // protocol, which the Rust `TimerSave::delta()` calls.  The C
    // prototype takes no const, but the routine only reads the timer.
    pub fn timer_delta(timer: *const Timer, save: *mut TimerSave) -> c_uint;

    // <kern/sched_prim.c>: the shim for `processor_set.sched_load`,
    // whose offset depends on the configure-time NCPUS.  It dies when
    // kern/processor.c moves.
    pub fn thread_glue_pset_sched_load(pset: *mut ProcessorSet) -> c_long;

    // <kern/sched_prim.c>: `int min_quantum`, the maximum context
    // switch rate, declared in <kern/sched.h>.  `pset_init` seeds the
    // set's quantum fields from it.
    pub static mut min_quantum: c_int;

    // <kern/processor_glue.c>: the NCPUS-sized tail of a processor set,
    // `machine_quantum` through `sched_load`.  It dies when NCPUS is
    // visible to Rust and the tail can be mirrored.
    pub fn processor_glue_pset_tail_init(
        pset: *mut ProcessorSet,
        quantum: c_int,
    );

    // <device/ds_routines.h>, the request passed as an opaque handle:
    // `struct io_req` itself belongs to its driver.
    pub fn iodone(ior: *mut c_void);
    pub fn device_read_alloc(ior: *mut c_void, size: usize) -> c_int;
    pub fn ds_read_done(ior: *mut c_void) -> c_int;

    // <machine/spl.h>: asm functions, `SPLKD` is a macro over `spltty`.
    pub fn splhi() -> c_int;
    pub fn splsched() -> c_int;
    pub fn spltty() -> c_int;
    pub fn splsoftclock() -> c_int;
    pub fn splx(level: c_int) -> c_int;

    // <i386at/com.h>
    pub fn comgetc(unit: c_int) -> c_int;

    // <device/tty.h> and <device/cirbuf.h>
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
    pub fn tty_queue_completion(queue: *mut c_void);
    pub fn getc(buf: *mut c_void) -> c_int;

    // Shims in i386/i386at/kd_glue.c, for the tty lock macros, the
    // line discipline switch, `ttlowat[]` and `phystokv()`.
    pub fn kd_simple_lock_irq(lock: *mut c_void) -> c_int;
    pub fn kd_simple_unlock_irq(s: c_int, lock: *mut c_void);
    pub fn kd_simple_lock(lock: *mut c_void);
    pub fn kd_simple_unlock(lock: *mut c_void);
    pub fn kd_ldisc_read(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_write(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_rint(line: c_int, c: c_uint, tp: *mut c_void);
    pub fn kd_ttlowat(speed: c_int) -> c_short;

    // <kern/mach_clock.h> and <i386/i386at/model_dep.c>
    pub static hz: c_int;
    pub static rebootflag: c_int;

    // Shims for the C macros Rust cannot call: see i386/i386/pio_glue.c.
    pub fn pio_inb(port: u16) -> u8;
    pub fn pio_inw(port: u16) -> u16;
    pub fn pio_inl(port: u16) -> u32;
    pub fn pio_outb(port: u16, value: u8);
    pub fn pio_outw(port: u16, value: u16);
    pub fn pio_outl(port: u16, value: u32);

    // Shims for `mask_irq'/'unmask_irq' (static inline under APIC) and
    // for the NINTR-sized `ivect'/`iunit' arrays: see i386/i386/irq.c.
    pub fn irq_mask(irq: c_uint);
    pub fn irq_unmask(irq: c_uint);
    pub fn irq_set_handler(
        irq: c_int,
        handler: Option<unsafe extern "C" fn(c_int)>,
    );
    pub fn irq_get_handler(irq: c_int) -> Option<unsafe extern "C" fn(c_int)>;
    pub fn irq_set_unit(irq: c_int, unit: c_int);
    pub fn irq_get_unit(irq: c_int) -> c_int;

    // Shims for the NCOM-sized `cominfo' array: see i386/i386at/com.c.
    pub fn com_base_addr(unit: c_int) -> VmOffset;
    pub fn com_irq(unit: c_int) -> c_int;

    // <i386at/biosmem.h>, used by the /dev/mem mmap hook.
    pub fn biosmem_addr_available(addr: VmOffset) -> c_int;

    // <ipc/ipc_thread_glue.c>: the ith_next/ith_prev pair of a thread,
    // as one `struct ipc_thread_links *`.
    pub fn ipc_thread_glue_links(thread: *mut c_void) -> *mut c_void;

    // <kern/slab.h>.  `kmem_cache_alloc` returns the object address as
    // the C code does; the caller turns it into a pointer.
    // `kmem_cache_init` builds a cache in caller storage, which the map
    // module still keeps in vm/vm_map_glue.c until kern/slab.c moves.
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

    // <kern/kalloc.h>: the page-list copyin's continuation argument
    // block, allocated for the continuation and freed after it runs.
    pub fn kalloc(size: VmSize) -> VmOffset;
    pub fn kfree(data: VmOffset, size: VmSize);

    // <ipc/ipc_port.h>: the send-right operations of the region proxy.
    // Ports stay opaque pointers until ipc/ipc_port.c moves.
    pub fn ipc_port_copy_send(port: *mut c_void) -> *mut c_void;
    pub fn ipc_port_release_send(port: *mut c_void);

    // The three caches of the map module and the submap placeholder,
    // which vm/vm_map_glue.c defines until kern/slab.c and
    // vm/vm_object.c move.
    pub static mut vm_map_cache: c_void;
    pub static mut vm_map_entry_cache: c_void;
    pub static mut vm_map_copy_cache: c_void;
    pub static mut vm_submap_object: *mut VmObject;

    // The three caches of the external-page bookkeeping, which
    // vm/vm_external_glue.c defines until kern/slab.c moves.
    pub static mut vm_external_cache: c_void;
    pub static mut vm_object_small_existence_map_cache: c_void;
    pub static mut vm_object_large_existence_map_cache: c_void;

    // <vm/vm_kern.c>.  The map is passed as an opaque handle here:
    // `VmMap` is `!Unpin` (it embeds a list), and the C signature only
    // needs the address.
    pub fn projected_buffer_collect(map: *mut c_void) -> c_int;

    // <vm/pmap.h> and <i386/intel/pmap.h>.
    pub fn pmap_destroy(pmap: *mut Pmap);
    pub static kernel_pmap: *mut Pmap;

    // <vm/vm_page.h>.  `VM_PAGE_WAIT` is a macro over `vm_page_wait`.
    pub fn vm_page_mem_size() -> VmSize;
    pub fn vm_page_grab(flags: c_uint) -> *mut VmPage;
    pub fn vm_page_copy(src: *mut VmPage, dst: *mut VmPage);
    pub fn vm_page_wait(continuation: Option<unsafe extern "C" fn()>);
    pub fn vm_page_more_fictitious();
    // The page queue lock must be held for `replace`, `wire` and
    // `activate`, as the page-list copyout does.
    pub fn vm_page_replace(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_wire(page: *mut VmPage);
    pub fn vm_page_activate(page: *mut VmPage);

    // <vm/pmap.h>: make a pmap range pageable, used when a wired copy
    // is entered.
    pub fn pmap_pageable(
        pmap: *mut Pmap,
        start: VmOffset,
        end: VmOffset,
        pageable: c_int,
    );

    // Shims in vm/vm_map_glue.c: the thread privilege bump the map
    // lock performs through `current_thread()`, and the machine-dependent
    // `pmap_attribute` macro.  Both die when their owners move.
    pub fn vm_map_glue_privilege_inc();
    pub fn vm_map_glue_privilege_dec();
    pub fn vm_map_glue_pmap_attribute(
        pmap: *mut Pmap,
        address: VmOffset,
        size: VmSize,
        attribute: c_uint,
        value: *mut c_int,
    ) -> c_int;
    pub fn vm_map_glue_thread_wakeup(event: *mut c_void);
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
    pub fn vm_map_glue_page_free(page: *mut VmPage);
    pub fn vm_map_glue_page_steal(page: *mut VmPage);
    pub fn vm_map_glue_page_protect(page: *mut VmPage, protection: c_int);
    pub fn vm_map_glue_page_set_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_clear_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_set_dirty(page: *mut VmPage);
    pub fn vm_map_glue_page_wakeup_done(page: *mut VmPage);
    pub fn vm_map_glue_page_activate_if_idle(page: *mut VmPage);
    pub fn vm_map_glue_page_wire_count(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_offset(page: *mut VmPage) -> VmOffset;
    pub fn vm_map_glue_page_queue_lock();
    pub fn vm_map_glue_page_queue_unlock();
    pub fn vm_map_glue_pmap_enter(
        pmap: *mut Pmap,
        addr: VmOffset,
        page: *mut VmPage,
        protection: c_int,
        wired: c_int,
    );

    // Shims in vm/vm_map_glue.c: the pager of a `struct vm_object` and
    // the task fields `vm_region_create_proxy` reads.  The object shim
    // dies with vm/vm_object.c, the task shims with kern/task.c.
    pub fn vm_map_glue_object_pager(object: *mut VmObject) -> *mut c_void;
    pub fn vm_map_glue_task_map(task: *mut c_void) -> *mut c_void;
    pub fn vm_map_glue_task_space(task: *mut c_void) -> *mut c_void;
    pub fn vm_map_glue_memory_object_create_proxy(
        space: *mut c_void,
        max_protection: c_int,
        object: *mut c_void,
        offset: VmOffset,
        start: VmOffset,
        len: VmSize,
        port: *mut *mut c_void,
    ) -> c_int;

    // <vm/vm_page.h>.
    pub fn vm_page_lookup(
        object: *mut VmObject,
        offset: VmOffset,
    ) -> *mut VmPage;

    // <vm/vm_object.h> and <vm/pmap.h>: the object and pmap operations
    // the deletion path reaches.
    pub fn vm_object_reference(object: *mut VmObject);
    pub fn vm_object_deallocate(object: *mut VmObject);
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
    pub fn vm_fault_wire(map: *mut c_void, entry: *mut c_void);

    // <vm/vm_fault.h>: fault a page in for the page-list copyin while
    // the map is unlocked.  The object lock and paging reference the
    // caller passes are consumed; the continuation is the scheduler's,
    // and the copyin passes none.
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

    // <vm/vm_fault.h>: copy pages between objects for the overwrite.
    // The size is in/out and the version is the caller's map-version
    // snapshot.
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

    // <vm/vm_object.h>.
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
    // The naked send right naming an object's pager, which `vm_region`
    // hands out, and the pager creation the region proxy performs on an
    // internal object.
    pub fn vm_object_name(object: *mut VmObject) -> *mut c_void;
    pub fn vm_object_pager_create(object: *mut VmObject);

    // <vm/pmap.h>: the physical-map operations of the fork.  `pmap_copy`
    // is a macro here, so it comes through the vm_map_glue.c shim.
    pub fn pmap_create(size: VmSize) -> *mut Pmap;
    pub fn vm_map_glue_pmap_copy(
        dst: *mut Pmap,
        src: *mut Pmap,
        dst_addr: VmOffset,
        len: VmSize,
        src_addr: VmOffset,
    );

    // The VM bootstrap, which rust/src/vm/vm_init.rs calls in the
    // order the packages depend on.
    // <vm/vm_page.h>: the resident-page table hands the physical
    // range it accounted for back through two out-parameters, so the
    // caller owns both slots.
    pub fn vm_page_bootstrap(startp: *mut VmOffset, endp: *mut VmOffset);
    pub fn vm_page_module_init();
    pub fn vm_page_info_all();

    // <kern/slab.h>.
    pub fn slab_bootstrap();
    pub fn slab_init();

    // <vm/vm_object.h>.
    pub fn vm_object_bootstrap();
    pub fn vm_object_init();

    // <vm/vm_kern.h>.
    pub fn kmem_init(start: VmOffset, end: VmOffset);

    // <vm/pmap.h>.
    pub fn pmap_init();

    // <kern/kalloc.h>.
    pub fn kalloc_init();

    // <vm/vm_fault.h>.
    pub fn vm_fault_init();

    // <vm/memory_object.h> and <vm/memory_object_proxy.h>: the
    // default memory manager and the proxy port, up with the rest.
    pub fn memory_manager_default_init();
    pub fn memory_object_proxy_init();
}
