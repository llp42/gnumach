// SPDX-License-Identifier: CMU-Mach
// Derived from kern/startup.c and kern/startup.h:
//   Copyright (c) 1991,1990,1989,1988 Carnegie Mellon University
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Kernel startup, which `kern/startup.c` used to define and `kern/startup.h`
//! declares.

use crate::arch::i386::model_dep::{self, kernel_cmdline};
use crate::arch::i386::pcb;
use crate::arch::i386::percpu::{
    cpu_number, current_thread, percpu_at, processor_ptr, set_active_thread,
};
use crate::arch::i386::pmap;
use crate::device::device_init;
use crate::glue;
use crate::ipc::ipc_init;
use crate::kern::mach_clock::{self, record_time_stamp};
use crate::kern::mach_factor;
use crate::kern::machine;
use crate::kern::processor::master_cpu;
use crate::kern::rdxtree_ffi;
use crate::kern::sched_prim;
use crate::kern::task::kernel_task;
use crate::kern::thread::{TH_RUN, TH_UNINT, Thread};
use crate::kern::thread_ffi;
use crate::kern::thread_swap;
use crate::kern::timer;
use crate::vm::vm_init;
use crate::vm::vm_page;
use crate::vm::vm_pageout_ffi;
use core::ffi::{c_char, c_int};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

/// `KERNEL_MAJOR_VERSION` of <mach/version.h>.
const KERNEL_MAJOR_VERSION: c_int = 4;
/// `KERNEL_MINOR_VERSION` of <mach/version.h>.
const KERNEL_MINOR_VERSION: c_int = 0;

/// `reboot_on_panic` of kern/startup.c: whether the panic path reboots or
/// halts.  The C `Panic()` reads the same symbol.
#[unsafe(export_name = "reboot_on_panic")]
static REBOOT_ON_PANIC: AtomicU32 = AtomicU32::new(1);

/// `setup_main()` in C: start the kernel from the boot processor.
///
/// # Safety
///
/// Runs once, on the interrupt stack of the boot processor, before any other
/// CPU or thread exists.
pub(crate) unsafe fn setup_main() {
    if unsafe { command_line_has_halt() } {
        // The store runs before any other CPU starts, and the C `Panic()`
        // read the same word without synchronization.
        REBOOT_ON_PANIC.store(0, Ordering::Relaxed);
    }

    crate::kern::debug::panic_init();

    // SAFETY: the boot sequence calls each initializer exactly once, in the
    // order the C used.
    unsafe {
        sched_prim::sched_init();
        vm_init::vm_mem_bootstrap();
        rdxtree_ffi::rdxtree_cache_init();
        ipc_init::ipc_bootstrap();
        vm_init::vm_mem_init();
        ipc_init::ipc_init();

        pmap::activate_kernel(master_cpu());
        timer::init_timers();
        mach_clock::init_timeout();
        model_dep::machine_init();
        mach_clock::mapable_time_init();
    }

    let info = machine::info();
    // SAFETY: no other CPU is running, and `machine_info` is the boot
    // record.
    unsafe {
        let memsize = vm_page::mem_size();
        (*info).max_cpus = crate::config::NCPUS as c_int;
        (*info).memory_size = memsize;
        if (*info).memory_size < memsize {
            (*info).memory_size = usize::MAX;
        }
        (*info).avail_cpus = 0;
        (*info).major_version = KERNEL_MAJOR_VERSION;
        (*info).minor_version = KERNEL_MINOR_VERSION;
    }

    // SAFETY: as above; the C ran the subsystem initializers in this order.
    unsafe {
        crate::kern::task_ffi::task_init();
        thread_ffi::thread_init();
        thread_swap::swapper_init();
        crate::kern::processor_ffi::pset_sys_init();

        sched_prim::recompute_priorities(ptr::null_mut());
        mach_factor::compute();
        crate::kern::gsync_ffi::gsync_setup();

        let mut startup_thread: *mut Thread = ptr::null_mut();
        let _ =
            thread_ffi::thread_create(kernel_task, &raw mut startup_thread);
        let _ =
            thread_ffi::thread_set_name(startup_thread, c"startup".as_ptr());
        thread_ffi::thread_start(startup_thread, Some(start_kernel_threads));
        thread_swap::thread_doswapin(startup_thread);

        (*startup_thread).set_state((*startup_thread).state() | TH_RUN);
        let _ = thread_ffi::thread_resume(startup_thread);

        cpu_launch_first_thread(startup_thread);
    }
}

/// The `strstr(kernel_cmdline, "-H ")` test of `setup_main()`.
///
/// # Safety
///
/// `kernel_cmdline` must point at the live NUL-terminated boot command line.
unsafe fn command_line_has_halt() -> bool {
    // SAFETY: the caller promises the live string; the boot set it before
    // the virtual-memory system came up.
    let line = unsafe { core::ffi::CStr::from_ptr(kernel_cmdline) };
    line.to_bytes().windows(3).any(|window| window == b"-H ")
}

/// `start_kernel_threads()` in C: create the kernel's service threads and the
/// bootstrap task.
///
/// # Safety
///
/// Runs once, in the startup thread, before the other CPUs are started.
pub(crate) unsafe extern "C" fn start_kernel_threads() {
    for i in 0..crate::config::NCPUS {
        // SAFETY: `i` indexes the `NCPUS` machine slots, which the probe
        // filled before this thread ran.
        if unsafe { (*machine::slot(i)).is_cpu } == 0 {
            continue;
        }

        // SAFETY: the kernel task is live, and the slot below is writable;
        // the C ignored a failure the same way.
        unsafe {
            let mut th: *mut Thread = ptr::null_mut();
            let _ = thread_ffi::thread_create(kernel_task, &raw mut th);

            let mut name = [0 as c_char; 10];
            let _ = glue::snprintf(
                name.as_mut_ptr(),
                name.len(),
                c"idle/%d".as_ptr(),
                i as c_int,
            );
            let _ = thread_ffi::thread_set_name(th, name.as_ptr());
            sched_prim::thread_bind(th, processor_ptr(i as c_int));
            thread_ffi::thread_start(
                th,
                Some(crate::kern::sched_prim_ffi::idle_thread),
            );
            thread_swap::thread_doswapin(th);
            let _ = thread_ffi::thread_resume(th);
        }
    }

    // SAFETY: the kernel task is live, and each continuation runs as its own
    // kernel thread, as the C started them.
    unsafe {
        let _ = thread_ffi::kernel_thread(
            kernel_task,
            c"reaper".as_ptr(),
            Some(thread_ffi::reaper_thread),
            ptr::null_mut(),
        );
        let _ = thread_ffi::kernel_thread(
            kernel_task,
            c"swapin".as_ptr(),
            Some(swapin_thread_continuation),
            ptr::null_mut(),
        );
        let _ = thread_ffi::kernel_thread(
            kernel_task,
            c"sched".as_ptr(),
            Some(crate::kern::sched_prim_ffi::sched_thread),
            ptr::null_mut(),
        );
        let _ = thread_ffi::kernel_thread(
            kernel_task,
            c"intr".as_ptr(),
            Some(crate::device::intr_ffi::intr_thread),
            ptr::null_mut(),
        );
        let _ = thread_ffi::kernel_thread(
            kernel_task,
            c"action".as_ptr(),
            Some(action_thread_continuation),
            ptr::null_mut(),
        );

        crate::arch::i386::mp_desc::start_other_cpus();
        device_init::device_service_create();
        record_time_stamp(&raw mut (*kernel_task).creation_time);
        crate::kern::bootstrap_ffi::bootstrap_create();
    }

    // SAFETY: the startup thread becomes the pageout daemon, which never
    // returns.
    unsafe {
        let _ = glue::spl0();
        let _ =
            thread_ffi::thread_set_name(current_thread(), c"pageout".as_ptr());
        vm_pageout_ffi::vm_pageout();
    }
}

/// `cpu_launch_first_thread()` in C: hand a CPU its first thread, never to
/// return.
///
/// # Safety
///
/// Runs on a CPU that is taking its first thread, with no thread of its own
/// yet.
pub(crate) unsafe extern "C" fn cpu_launch_first_thread(
    mut th: *mut Thread,
) -> ! {
    let mycpu = cpu_number();

    // SAFETY: the boot CPU and every AP reach this once, on their own
    // processor.
    unsafe {
        crate::kern::machine_ffi::cpu_up(mycpu);

        // The C `start_timer()` is an empty macro in <kern/timer.h>.

        glue::splhigh();

        if th.is_null() {
            th = sched_prim::choose_thread(processor_ptr(mycpu));
        }
        if th.is_null() {
            panic_no_thread();
        }

        pmap::activate_kernel(mycpu);
        set_active_thread(th);
        (*percpu_at(mycpu)).active_stack = (*th).kernel_stack;

        (*th).lock.lock();
        (*th).set_state((*th).state() & !TH_UNINT);
        (*th).lock.unlock();
        (*th).last_processor = processor_ptr(mycpu);

        // The C `timer_switch()` is an empty macro in <kern/timer.h>.

        let map = (*(*th).task).map.cast::<crate::vm::vm_map::VmMap>();
        pmap::activate_user((*map).pmap, mycpu);

        model_dep::startrtclock();
        pcb::load_context(th);
    }
}

/// The `swapin_thread()` continuation, which `kernel_thread()` starts.
unsafe extern "C" fn swapin_thread_continuation() {
    // SAFETY: the swapper thread runs once and never returns.
    unsafe { thread_swap::swapin_thread() }
}

/// The `action_thread()` continuation, which `kernel_thread()` starts.
unsafe extern "C" fn action_thread_continuation() {
    // SAFETY: the action thread runs once and never returns.
    unsafe { crate::kern::machine_ffi::action_thread() }
}

/// The `panic("cpu_launch_first_thread")` of the C.
fn panic_no_thread() -> ! {
    // SAFETY: `Panic` does not return; the tags are the C panic macro's.
    unsafe {
        glue::Panic(
            c"kern/startup.c".as_ptr(),
            line!() as c_int,
            c"cpu_launch_first_thread".as_ptr(),
            c"cpu_launch_first_thread".as_ptr(),
        )
    }
}
