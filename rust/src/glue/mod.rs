// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls, and the interface records both halves
//! share.
//!
//! C *macros* cannot come through here, and no shim may be written for
//! one: the thing that defines the macro is ported first, or the real
//! symbol the macro expands to is declared below like any other C
//! function.
//!
//! [`mig`] holds the conversions between the Rust error codes and the
//! result codes the C side passes, and [`time_value`] the time records
//! of <mach/time_value.h>.

pub mod mig;
pub mod time_value;

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::lock::SimpleLock;
use crate::kern::machine::MachineSlot;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::QueueEntry;
use crate::kern::sched_prim::NUMQUEUES;
use crate::kern::thread::Thread;
use crate::vm::types::{Pmap, VmObject, VmPage, VmProt};
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_void};
use core::mem::offset_of;

/// `NSPEEDS` of <device/tty_status.h>: how many baud-rate slots
/// `ttlowat[]` and `tthiwat[]` are indexed by.
pub const NSPEEDS: usize = 18;

/// `struct ldisc_switch` of <device/tty.h>: the entry points one line
/// discipline provides.
///
/// The tty and request pointers are `*mut c_void`, as the rest of
/// this module's <device/tty.h> declarations spell them, because the
/// `Tty` mirror belongs to the kd driver rather than to the
/// interface.  `l_modem` and `l_start` are carried so the record has
/// the C layout; only the C side calls them.
#[repr(C)]
pub struct LdiscSwitch {
    /// `l_read`: hand a read request to the discipline.
    pub l_read:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    /// `l_write`: hand a write request to the discipline.
    pub l_write:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    /// `l_rint`: feed one received character to the discipline.
    pub l_rint: Option<unsafe extern "C" fn(c_uint, *mut c_void)>,
    /// `l_modem`: report a modem carrier change.
    pub l_modem: Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>,
    /// `l_start`: restart stalled output.
    pub l_start: Option<unsafe extern "C" fn(*mut c_void)>,
}

// Five function pointers, in the order <device/tty.h> declares them.
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
    // <kern/mach_clock.h>: the wall-clock time, which `kern/mach_clock.c`
    // defines and `inittodr()` sets at boot.  `time_value64_t` is the
    // `TimeValue64` mirror.
    pub static mut time: time_value::TimeValue64;

    // <kern/machine.c>
    pub fn cpu_shutdown();

    // <i386/i386/model_dep.h>: halt every CPU, or reboot when `reboot`
    // is nonzero.  It is defined in i386/i386at/model_dep.c and marked
    // `noreturn`, so the Rust declaration diverges too.
    pub fn halt_all_cpus(reboot: c_int) -> !;

    // <i386at/kd.h>, the screen block moves in kdasm.S
    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

    // <kern/sched_prim.h>
    pub fn assert_wait(event: *mut c_void, interruptible: c_int);
    pub fn thread_block(continuation: Option<unsafe extern "C" fn()>);
    // The C half of the scheduler still owns these; the Rust port
    // calls them.
    pub fn update_priority(thread: *mut Thread);
    pub fn stack_free(thread: *mut Thread);
    // <kern/sched_prim.h>: allocate a kernel stack for a swapped-out
    // thread and resume it through the given continuation.  The
    // continuation is `thread_continue` below, a C symbol the swapin
    // path passes by address.
    pub fn stack_alloc(
        thread: *mut Thread,
        resume: Option<unsafe extern "C" fn(*mut Thread)>,
    ) -> c_int;
    pub fn thread_continue(thread: *mut Thread);

    // <kern/syscall_subr.h>: the priority-depression timeout, stored
    // in `thread.depress_timer.fcn`.
    pub fn thread_depress_timeout(thread: *mut c_void);

    // <kern/thread.h>: the scheduling-policy setters the processor-set
    // updates call on each member thread.  They stay C for now, and
    // the port calls them exactly as kern/processor.c did.
    pub fn thread_policy(
        thread: *mut Thread,
        policy: c_int,
        data: c_int,
    ) -> c_int;
    pub fn thread_max_priority(
        thread: *mut Thread,
        pset: *mut ProcessorSet,
        max_priority: c_int,
    ) -> c_int;

    // <kern/thread.h> and <kern/task.h>: the processor-set assignment
    // routines the `*_assign_default` entries forward to.  Both are
    // the `#if MACH_HOST` half of their file and stay C; the port
    // calls them with the arguments the C default entries supplied.
    pub fn thread_assign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
    ) -> c_int;
    pub fn task_assign(
        task: *mut c_void,
        new_pset: *mut ProcessorSet,
        assign_threads: c_int,
    ) -> c_int;

    // <kern/thread.h>: reserve the thread's current kernel stack, so
    // `stack_alloc_try()` on it always succeeds.
    pub fn stack_privilege(thread: *mut Thread);

    // <kern/mach_clock.h>: cancel a wait timeout, under the thread
    // lock.  The handle is opaque here; its layout lives in
    // rust/src/kern/mach_clock.rs, which is GPL-derived.
    pub fn reset_timeout(t: *mut c_void) -> c_int;

    // <i386/i386/smp.h>: the remote-AST IPI that
    // rust/src/arch/i386/ast_check.rs's `cause_ast_check()` sends.
    pub fn smp_remote_ast(logical_id: c_uint);

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

    // <kern/processor_glue.c>: the same tail's head, `machine_quantum`,
    // which `quantum_set()` indexes.  It dies when NCPUS is visible to
    // Rust and the tail can be mirrored.
    pub fn processor_glue_pset_machine_quantum(
        pset: *mut ProcessorSet,
    ) -> *mut c_int;

    // <kern/processor_glue.c>: the tail's `mach_factor` and
    // `load_average`, which the basic-information flavor reads.  They
    // die when NCPUS is visible to Rust and the tail can be mirrored.
    pub fn processor_glue_pset_mach_factor(pset: *mut ProcessorSet) -> c_long;
    pub fn processor_glue_pset_load_average(pset: *mut ProcessorSet)
    -> c_long;

    // <kern/processor.c>: the global processor-set list, its count and
    // its lock, which the C half goes on using.  `queue_head_t` is the
    // `QueueEntry` mirror.
    pub static mut all_psets: QueueEntry;
    pub static mut all_psets_lock: SimpleLock;
    pub static mut all_psets_count: c_int;

    // <kern/processor.c>: the processor the machine boots on, which
    // `pset_sys_bootstrap()` points at the master slot.
    pub static mut master_processor: *mut Processor;

    // <kern/machine.c>: the machine table.  The C declares
    // `struct machine_slot machine_slot[NCPUS]`, and `NCPUS` is a C
    // constant, so this names the first element and the Rust side
    // strides it with `addr_of_mut!`.
    pub static mut machine_slot: MachineSlot;

    // <kern/processor.c>: address-only.  The full `struct
    // processor_set` is longer than the Rust mirror and `struct
    // kmem_cache` has no mirror, so only the addresses are named.
    pub static mut default_pset: c_void;
    pub static mut pset_cache: c_void;

    // <kern/host.c>: address-only like the two above.  `host_data_t`
    // has no Rust mirror, so only the address is named.
    pub static mut realhost: c_void;

    // <version.c>: `const char version[]`, the package name and
    // version the build stamps in.  The array has no size in C, so
    // this names its first byte and the Rust side reads from there to
    // the terminator.
    pub static version: c_char;

    // <kern/machine.h>: the machine-dependent shutdown of a processor,
    // still C on both architectures.
    pub fn processor_shutdown(processor: *mut Processor) -> c_int;

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
    pub fn splclock() -> c_int;
    pub fn splhigh() -> c_int;
    pub fn splx(level: c_int) -> c_int;
    // <i386/i386/spl.h>: the flags pair, defined per arch in
    // i386/i386/spl.S or x86_64/spl.S.  `sploff()` disables interrupts
    // and returns the flags word, and `splon()` restores the word it
    // returned.
    pub fn sploff() -> c_ulong;
    pub fn splon(n: c_ulong);

    // <i386at/com.h>
    pub fn comgetc(unit: c_int) -> c_int;

    // <device/tty.h>
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

    // <device/tty.h>: the line-discipline switch and the output
    // low-water marks, both built by device/chario.c's initializers
    // and never written afterwards.  `linesw[]` holds the single
    // character discipline this kernel has.
    pub static linesw: [LdiscSwitch; 1];
    pub static ttlowat: [c_short; NSPEEDS];

    // <kern/mach_clock.h> and <i386/i386at/model_dep.c>
    pub static hz: c_int;
    pub static rebootflag: c_int;

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

    // <i386/i386/fpu.c>: the FPU save-area cache that
    // `fpu_module_init()` builds and `fp_free()` returns objects to.
    // It is a `struct kmem_cache` with no Rust mirror, so only the
    // address is named; this declaration dies when kern/slab.c moves.
    pub static mut ifps_cache: c_void;

    // <i386/i386/machine_task.c>: the cache of I/O permission bitmaps
    // that rust/src/arch/i386/machine_task.rs's
    // `machine_task_module_init()` builds and that the rest of that C
    // file still allocates from.  It is a `struct kmem_cache` with no
    // Rust mirror, so only the address is named; this declaration dies
    // when kern/slab.c moves.
    pub static mut machine_task_iopb_cache: c_void;

    // <kern/kalloc.h>: the page-list copyin's continuation argument
    // block, allocated for the continuation and freed after it runs.
    pub fn kalloc(size: VmSize) -> VmOffset;
    pub fn kfree(data: VmOffset, size: VmSize);

    // <ipc/ipc_port.h>: the send-right operations of the region proxy.
    // Ports stay opaque pointers until ipc/ipc_port.c moves.
    pub fn ipc_port_copy_send(port: *mut c_void) -> *mut c_void;
    pub fn ipc_port_release_send(port: *mut c_void);

    // <ipc/ipc_object.h>: the rights operations behind the three
    // mach_port server routines.  `ipc_object_copyin_type` is Rust now
    // and is called directly; these three stay C.
    pub fn ipc_object_rename(
        space: *mut c_void,
        old_name: c_uint,
        new_name: c_uint,
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

    // <ipc/ipc_object.c>: the receive-right lookup that the C macro
    // `ipc_port_translate_receive` expands to.  A macro cannot cross
    // FFI, so the underlying symbol is declared and
    // `MACH_PORT_RIGHT_RECEIVE` is supplied by the caller.
    pub fn ipc_object_translate(
        space: *mut c_void,
        name: c_uint,
        right: c_uint,
        objectp: *mut *mut c_void,
    ) -> c_int;

    // <ipc/ipc_port.h>: the two notification registrations behind
    // `mach_port_request_notification()`.  Both consume the port lock
    // `ipc_object_translate` returned; the previous send-once right
    // comes back through the out-pointer.
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

    // <ipc/ipc_right.h>: the dead-name registration, which owns its
    // own space and port locking and writes `previousp` on success
    // only.
    pub fn ipc_right_dnrequest(
        space: *mut c_void,
        name: c_uint,
        immediate: c_int,
        notify: *mut c_void,
        previousp: *mut *mut c_void,
    ) -> c_int;

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
