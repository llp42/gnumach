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
use crate::kern::mach_clock::Timeout;
use crate::kern::machine::MachineSlot;
use crate::kern::processor::{Processor, ProcessorSet};
use crate::kern::queue::QueueEntry;
use crate::kern::sched::RunQueue;
use crate::kern::sched_prim::NUMQUEUES;
use crate::kern::thread::{Continuation, StackResume, Thread};
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

    // <device/cons.h>: the polled console character `safe_gets()`
    // reads.  `device/cons.c` defines it.
    pub fn cngetc() -> c_int;

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

    // <kern/mach_clock.c>: the difference between the boot-time clock
    // and the real-time clock, which `clock_boottime_update()`
    // maintains and `record_time_stamp()` adds back.  Rust reads it
    // where the C `read_time_stamp()` did.
    pub static mut clock_boottime_offset: time_value::TimeValue64;

    // <kern/mach_clock.c>: the gradual-adjustment state, read and
    // written under `splclock()` by the clock interrupt and
    // `host_adjust_time64()`.  The two C `unsigned` globals are the
    // `c_uint` pair.
    pub static mut timedelta: c_int;
    pub static mut tickdelta: c_int;
    pub static mut tickadj: c_uint;
    pub static mut bigadj: c_uint;

    // <kern/mach_host.server.h>: the 64-bit wall-clock setter that
    // `host_set_time()` forwards to; it stays C with the rest of the
    // host clock entries.
    pub fn host_set_time64(
        host: *mut c_void,
        new_time: time_value::TimeValue64,
    ) -> c_int;

    // <kern/mach_clock.c>: the page the user side reads the clock
    // through, which `update_mapped_time()` writes.  Rust reads only
    // the pointer, never the page it names; `timemmap()` in
    // rust/src/arch/i386/model_dep.rs turns the address into a page
    // frame, and the `mapped_time_value_t` mirror gives the pointer
    // its type.
    pub static mtime: *mut time_value::MappedTimeValue;

    // <kern/machine.c>
    pub fn cpu_shutdown();
    // <kern/machine.h>: the action thread's body.  It drains the
    // action queue in a loop and never returns, because its wait
    // re-enters the routine itself.
    pub fn action_thread_continue() -> !;

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
    // <kern/sched.h>: take `th` off its run queue and answer the queue
    // it was on, or the C `RUN_QUEUE_NULL` when it was on none.
    // `kern/sched_prim.c` still defines it; `set_pri()` calls it.
    pub fn rem_runq(th: *mut Thread) -> *mut RunQueue;
    // <kern/sched_prim.h>: the machine-dependent return to user mode
    // after a kernel call; it never returns.
    pub fn thread_exception_return() -> !;
    // <i386/i386/pcb.h>: attach a stack to a thread, installing the
    // continuation a swapped-in thread resumes through.  The stack
    // allocation and the free list are Rust now, in
    // rust/src/kern/thread.rs.
    pub fn stack_attach(
        thread: *mut Thread,
        stack: VmOffset,
        continuation: StackResume,
    );
    // <kern/thread.h>: fold the usage a finalized stack recorded into
    // the global maximum.  The `stack_init()` half is Rust now, in
    // rust/src/kern/thread.rs; both do nothing unless
    // `stack_check_usage` is set.
    pub fn stack_finalize(stack: VmOffset);

    // <i386/i386/pcb.h>: write the syscall return register, and apply a
    // `thread_state_t` array of `count` `natural_t` words to a thread's
    // saved machine state.  `thread_set_self_state()` is their trap
    // caller.
    pub fn thread_set_syscall_return(thread: *mut Thread, retval: c_int);
    pub fn thread_setstatus(
        thread: *mut Thread,
        flavor: c_int,
        tstate: *mut c_uint,
        count: c_uint,
    ) -> c_int;
    // <i386/i386/pcb.h>: read a thread's saved machine state into a
    // `thread_state_t` array of `*count` `natural_t` words, narrowing
    // the count to the state the flavor fills.  The Rust
    // `thread_get_state()` calls it, as the setter above serves the
    // Rust `thread_set_state()`.
    pub fn thread_getstatus(
        thread: *mut Thread,
        flavor: c_int,
        tstate: *mut c_uint,
        count: *mut c_uint,
    ) -> c_int;

    // <i386/i386/locore.h>: copy `cn` bytes from a user address to a
    // kernel one, returning nonzero when the copy faults.  The C
    // `size_t` is `usize` on both targets.
    pub fn copyin(
        userbuf: *const c_void,
        kernelbuf: *mut c_void,
        cn: usize,
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

    // <kern/task.c>: the creation and teardown the task entries call,
    // and the notification port `task_create_kernel()` reads when a
    // task is created.  `struct task` is opaque here: it embeds an
    // `ipc_space`, a `vm_map` and the emulation vector, so only its
    // address is named.
    pub fn task_create_kernel(
        parent_task: *mut c_void,
        inherit_memory: c_int,
        child_task: *mut *mut c_void,
    ) -> c_int;
    pub fn task_terminate(task: *mut c_void) -> c_int;
    pub fn task_deallocate(task: *mut c_void);
    pub static mut new_task_notification: *mut c_void;

    // <kern/thread.h>: halt the current thread, resuming it through
    // `continuation` if it is released again.  The exception path
    // passes `thread_exception_return`, whose `!` makes it a
    // continuation that never comes back.
    pub fn thread_halt_self(continuation: Continuation);

    // <kern/thread.h>: the thread-lifecycle entries kern/thread.c still
    // defines, which the Rust `thread_force_terminate()` and
    // `thread_abort()` call.
    pub fn thread_freeze(thread: *mut Thread);
    pub fn thread_doassign(
        thread: *mut Thread,
        new_pset: *mut ProcessorSet,
        release_freeze: c_int,
    );
    pub fn thread_halt(thread: *mut Thread, must_halt: c_int) -> c_int;
    pub fn thread_deallocate(thread: *mut Thread);
    // <kern/thread.h>: wait for a thread to reach the stopped state a
    // `thread_hold()` arranged.  kern/thread.c defines it; the Rust
    // `thread_get_state()` and `thread_set_state()` call it, and
    // `must_halt` is the C boolean.
    pub fn thread_dowait(thread: *mut Thread, must_halt: c_int) -> c_int;

    // <kern/ipc_tt.h>: shut down a thread's IPC state; it stays C with
    // the rest of kern/ipc_tt.c for now.
    pub fn ipc_thread_terminate(thread: *mut Thread);

    // <kern/eventcount.h>: break a thread out of an event-count wait;
    // kern/eventcount.c still defines it.
    pub fn evc_notify_abort(thread: *mut Thread);

    // <kern/mach_clock.h>: cancel a wait timeout, under the thread
    // lock.  The handle is opaque here; its layout lives in
    // rust/src/kern/mach_clock.rs, which is GPL-derived.
    pub fn reset_timeout(t: *mut c_void) -> c_int;

    // <kern/mach_clock.h>: arm a private timer element to fire after
    // `interval` ticks.  The caller holds the lock protecting `t`.
    pub fn set_timeout(t: *mut Timeout, interval: c_uint);

    // <i386/i386/smp.h>: the remote-AST IPI that
    // rust/src/arch/i386/ast_check.rs's `cause_ast_check()` sends, and
    // the remote pmap-TLB flush IPI that
    // rust/src/arch/i386/mp_desc.rs's `interrupt_processor()` sends.
    pub fn smp_remote_ast(logical_id: c_uint);
    pub fn smp_pmap_update(logical_id: c_uint);

    // <kern/sched_prim.c>: `unsigned sched_tick`, the second counter
    // priorities age against, and the private timer
    // `recompute_priorities()` re-arms plus the scheduler thread it
    // wakes.  `sched_init()` still writes the timer's function field,
    // so the globals stay C.
    pub static mut sched_tick: c_uint;
    pub static mut recompute_priorities_timer: Timeout;
    pub static mut sched_thread_id: *mut Thread;

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
    // <kern/thread.c>: the free list of kernel stacks, its length and
    // the patchable limit `stack_collect()` trims it to.  Reached only
    // at splsched, under `stack_lock_data`; rust/src/kern/thread.rs
    // moves the entries.
    pub static mut stack_free_list: VmOffset;
    pub static mut stack_free_count: c_uint;
    pub static mut stack_free_limit: c_uint;
    // <kern/thread.c>: nonzero when a fresh kernel stack is filled
    // with the usage marker.  A debugger may set it, so the Rust
    // `stack_init()` reads the mutable global.
    pub static mut stack_check_usage: c_int;
    // <kern/sched_prim.h>: where a fresh thread starts execution.
    pub fn thread_bootstrap_return();
    // <i386/i386/pcb.h>
    pub fn pcb_module_init();
    // <i386/i386/pcb.h>: the two halves of `load_context()`, which
    // rust/src/arch/i386/pcb.rs now defines.  `switch_ktss` loads the
    // thread's TSS and its per-thread GDT entries; `Load_context` is
    // the assembly that switches stacks and resumes the thread, so it
    // never returns.  `struct pcb` has no Rust mirror, so the pcb is
    // passed as the opaque pointer `Thread.pcb` stores.
    pub fn switch_ktss(pcb: *mut c_void);
    pub fn Load_context(new: *mut Thread) -> !;

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

    // <kern/machine.h>: the machine-dependent shutdown of a processor,
    // still C on both architectures.
    pub fn processor_shutdown(processor: *mut Processor) -> c_int;

    // <device/ds_routines.h>, the request passed as an opaque handle:
    // `struct io_req` itself belongs to its driver.
    pub fn iodone(ior: *mut c_void);
    pub fn device_read_alloc(ior: *mut c_void, size: usize) -> c_int;
    pub fn ds_read_done(ior: *mut c_void) -> c_int;

    // <device/ds_routines.h>, <device/net_io.h> and <device/chario.h>:
    // the device layer's initializers, which the Rust
    // `device_service_create()` runs in the C order.  `kernel_thread`
    // is <kern/thread.h>, and both start routines take no arguments and
    // never return.  `ds_device_open` is the open handler that the Rust
    // `ds_device_open_new` forwards to, and it stays C.
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

    // <device/device_port.h> and <device/device_init.c>: the master
    // device port, written once by rust/src/device/device_init.rs and
    // read by the C open path.  <kern/task.h> and <ipc/ipc_space.h>:
    // the kernel's task and IPC space, read by that same creation.
    pub static mut master_device_port: *mut c_void;
    pub static kernel_task: *mut c_void;
    pub static ipc_space_kernel: *mut c_void;

    // <machine/spl.h>: asm functions, `SPLKD` is a macro over `spltty`.
    pub fn spl0() -> c_int;
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

    // <device/tty.h>: the line-discipline switch and the output
    // low-water marks, both built by device/chario.c's initializers
    // and never written afterwards.  `linesw[]` holds the single
    // character discipline this kernel has.
    pub static linesw: [LdiscSwitch; 1];
    pub static ttlowat: [c_short; NSPEEDS];

    // <kern/mach_clock.h> and <i386/i386at/model_dep.c>
    pub static hz: c_int;
    pub static rebootflag: c_int;
    // <kern/mach_clock.h>: the microseconds per tick, initialized from
    // `MICROSECONDS_IN_ONE_SECOND / HZ` and never written afterwards;
    // the Rust `thread_policy()` reads it.
    pub static tick: c_int;

    // <i386/i386/irq.h>: the APIC-or-PIC selection, read by
    // rust/src/device/intr.rs.  `i386/i386at/ioapic.c` defines it in
    // the APIC build and `i386/i386/pic.c` in the PIC build.
    pub static pic_mode: c_int;

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

    // <i386/i386/apic.h>: the local-APIC page `apic_lapic_init()`
    // publishes, the APIC-ID-to-kernel-ID table, and the mask
    // `fix_apic_id_mask()` settles at boot.  `struct ApicLocalUnit` has
    // no Rust mirror, so only the page's address is named here.
    pub static mut lapic: *mut c_void;
    pub static mut cpu_id_lut: [c_int; 256];
    pub static mut apic_id_mask: u8;

    // i386/i386at/acpi_parse_apic.c and <i386/i386/apic.h>: the mapped
    // HPET register window, which ACPI sets up, and the period
    // `hpet_init()` derives from it.
    pub static mut hpet_addr: *mut u32;
    pub static mut hpet_period_nsec: u32;

    // <i386at/biosmem.h>, used by the /dev/mem mmap hook.
    pub fn biosmem_addr_available(addr: VmOffset) -> c_int;

    // <i386at/biosmem.h>: allocate contiguous physical pages during
    // bootstrap, for the page tables and the early copies.  The C
    // parameter is an `unsigned int` and the result an
    // `unsigned long`, which the two `core::ffi` types mirror.
    pub fn biosmem_bootalloc(nr_pages: c_uint) -> c_ulong;

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

    // <i386/i386/fpu.h>: load a thread's saved state into the FPU.
    // `fp_load()` stays C with the rest of the FPU state handling, and
    // `fpnoextflt()` in rust/src/arch/i386/fpu.rs calls it.
    pub fn fp_load(thread: *mut Thread);

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

    // <ipc/ipc_port.h>: the second release and the send-once
    // notification `ipc_object_destroy` dispatches to, plus the port
    // initializer the allocators call with the object the C side
    // returns to Rust.  Ports and spaces stay opaque.
    pub fn ipc_port_release_receive(port: *mut c_void);
    pub fn ipc_port_init(port: *mut c_void, space: *mut c_void, name: c_uint);
    // <ipc/ipc_notify.h>.
    pub fn ipc_notify_send_once(port: *mut c_void);

    // <kern/ipc_host.h>: the processor-to-name-port conversion behind
    // `processor_set_processors()`.  The routine stays C with the rest
    // of the file's conversions, which read `struct ipc_port` fields.
    pub fn convert_processor_name_to_port(
        processor: *mut Processor,
    ) -> *mut c_void;

    // <kern/ipc_kobject.h>: bind a special port to the kernel object it
    // represents.  `ipc_kobject_t` is a `vm_offset_t`, so the object is
    // passed as its address.
    pub fn ipc_kobject_set(
        port: *mut c_void,
        kobject: VmOffset,
        type_: c_uint,
    );

    // <ipc/ipc_port.h>: the special-port allocator and deallocator
    // behind the `ipc_port_alloc_kernel()` and
    // `ipc_port_dealloc_kernel()` macros, which cannot cross FFI.  The
    // space is opaque here, as the port operations above spell it.
    pub fn ipc_port_alloc_special(space: *mut c_void) -> *mut c_void;
    pub fn ipc_port_dealloc_special(port: *mut c_void, space: *mut c_void);

    // <ipc/ipc_space.h>: the reply space `ipc_init()` builds, which the
    // `ipc_port_alloc_reply()`/`ipc_port_dealloc_reply()` macros name.
    pub static ipc_space_reply: *mut c_void;

    // <kern/ipc_tt.h>: allocate a reply port in the current task's
    // space, or `MACH_PORT_NULL` when it fails.
    pub fn mach_reply_port() -> c_uint;

    // <ipc/ipc_object.h>: the rights operations behind the three
    // mach_port server routines.  `ipc_object_copyin_type` is Rust now
    // and is called directly; these three stay C.
    pub fn ipc_object_rename(
        space: *mut c_void,
        old_name: c_uint,
        new_name: c_uint,
    ) -> c_int;

    // <ipc/ipc_object.h>: the object allocators behind `ipc_port_alloc`
    // and `ipc_port_alloc_name`.  `otype` is the `ipc_object_type_t`
    // the caller picks and `type_` the `mach_port_type_t` it wants the
    // entry to carry; both are `unsigned int` in the C.
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

    // <vm/vm_kern.h> and <ipc/ipc_init.c>: the IPC kernel submap the
    // C allocation routines carve out of `kernel_map`, and the host
    // bootstrap `ipc_init()` runs after it.  `ipc_kernel_map` and
    // `kernel_map` are opaque addresses, and `kmem_submap` takes its
    // bounds back through two out-parameters, exactly as the C does.
    pub static mut ipc_kernel_map: *mut c_void;
    pub static ipc_kernel_map_size: VmSize;
    pub fn kmem_submap(
        map: *mut c_void,
        parent: *mut c_void,
        minp: *mut VmOffset,
        maxp: *mut VmOffset,
        size: VmSize,
    );
    pub fn ipc_host_init();

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
    // <i386/intel/pmap.h>: the page-table-entry lookup the
    // `kvtophys()` port in rust/src/arch/i386/phys.rs walks.  It
    // returns null when the address has no pte, and the entry it
    // points at is a `phys_addr_t`, which `VmOffset` mirrors.
    pub fn pmap_pte(pmap: *mut Pmap, addr: VmOffset) -> *mut VmOffset;
    // <vm/pmap.h>: the virtual-to-physical lookup the `/dev/time` mmap
    // hook uses on the mapped time page.  The C's `phys_addr_t` is
    // `unsigned long` in both configured builds, which `VmOffset`
    // mirrors; the i386 `--enable-pae` configuration widens it to
    // 64 bits and is a known gap.
    pub fn pmap_extract(pmap: *mut Pmap, address: VmOffset) -> VmOffset;

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

    // <vm/vm_page.h>: the page queue lock and the real
    // `vm_page_free`, which the C `VM_PAGE_FREE` macro wraps as
    // lock, free, unlock.
    pub static mut vm_page_queue_lock: SimpleLock;
    pub fn vm_page_free(page: *mut VmPage);

    // Shims in vm/vm_map_glue.c: the object and page fields the
    // map's C edges still read.  They die when `struct vm_object`
    // and `struct vm_page` move.
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

    // Shims in vm/vm_map_glue.c: the pager of a `struct vm_object` and
    // the task fields `vm_region_create_proxy` reads.  The object shim
    // dies with vm/vm_object.c, the task shims with kern/task.c.
    pub fn vm_map_glue_object_pager(object: *mut VmObject) -> *mut c_void;
    pub fn vm_map_glue_task_map(task: *mut c_void) -> *mut c_void;
    pub fn vm_map_glue_task_space(task: *mut c_void) -> *mut c_void;

    // <vm/memory_object_proxy.c>: the `rpc_vm_*` arguments this
    // interface takes are pointer-sized on every supported build, so
    // the native `vm_offset_t`/`vm_size_t` types cross directly.
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

    // <vm/pmap.h>: the physical-map operations of the fork.
    pub fn pmap_create(size: VmSize) -> *mut Pmap;

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
