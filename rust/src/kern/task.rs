// SPDX-License-Identifier: CMU-Mach
// Derived from kern/task.c and kern/task.h:
//   Copyright (c) 1993-1988 Carnegie Mellon University.
// Derived from include/mach/vm_statistics.h:
//   Copyright (c) 1993-1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task module's cores, which `kern/task.c` used to define and
//! `kern/task.h` declares, and the `struct task` mirror of `kern/task.h`.

use crate::arch::i386::machine_task::{MachineTask, machine_task_module_init};
use crate::arch::i386::percpu::{cpu_number, current_thread};
use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::PAGE_SIZE;
use crate::glue;
use crate::glue::time_value::{
    RpcTimeValue, TIME_NANOS_MAX, TimeValue, TimeValue64,
};
use crate::ipc::ipc_space;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::ast::{AST_BLOCK, ast_on};
use crate::kern::ipc_tt::{
    convert_task_to_port, convert_thread_to_port, ipc_task_disable,
    ipc_task_enable, ipc_task_init, ipc_task_terminate, ipc_thread_disable,
    ipc_thread_terminate,
};
use crate::kern::lock::SimpleLock;
use crate::kern::mach_clock::read_time_stamp;
use crate::kern::processor::{ProcessorSet, pset_deallocate, pset_reference};
use crate::kern::queue::{
    QueueEntry, queue_empty, queue_end, queue_enter_tail, queue_first,
    queue_init, queue_next, queue_remove_generic,
};
use crate::kern::sched::invalid_pri;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, compute_priority, thread_wakeup_prim,
};
use crate::kern::slab::{CacheInitFlags, KmemCache, kalloc, kfree};
use crate::kern::syscall_emulation::eml_init;
use crate::kern::thread::Thread;
use crate::kern::timer::thread_read_times;
use crate::kern::types::KernError;
use crate::vm::types::Pmap;
use crate::vm::vm_map::{VmMap, round_page, trunc_page};
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{
    self, NonNull, addr_of, addr_of_mut, null_mut, with_exposed_provenance_mut,
};

/// `TASK_PORT_REGISTER_MAX` of <mach/mach_param.h>: the registered send
/// rights a task holds.
pub(crate) const TASK_PORT_REGISTER_MAX: usize = 4;

/// `TASK_NAME_SIZE` of <kern/task.h>.
const TASK_NAME_SIZE: usize = 32;

/// `BASEPRI_USER` of <kern/sched.h>: a fresh user task's priority.
const BASEPRI_USER: c_int = 25;

/// `TASK_BASIC_INFO`, `TASK_EVENTS_INFO` and `TASK_THREAD_TIMES_INFO` of
/// <mach/task_info.h>.
const TASK_BASIC_INFO: c_int = 1;
const TASK_EVENTS_INFO: c_int = 2;
const TASK_THREAD_TIMES_INFO: c_int = 3;

/// `VM_MIN_USER_ADDRESS` and `VM_MAX_USER_ADDRESS` of <i386/vm_param.h>:
/// the bounds of a fresh user map.  The `--enable-user32` third value is out
/// of scope for the Rust half.
const VM_MIN_USER_ADDRESS: VmOffset = 0;
#[cfg(target_arch = "x86_64")]
const VM_MAX_USER_ADDRESS: VmOffset = 0x8000_0000_0000;
#[cfg(target_arch = "x86")]
const VM_MAX_USER_ADDRESS: VmOffset = 0xc000_0000;

/// `IKOT_NONE`, `IKOT_HOST` and `IKOT_HOST_PRIV` of <kern/ipc_kobject.h>.
const IKOT_NONE: c_uint = 0;
const IKOT_HOST: c_uint = 3;
const IKOT_HOST_PRIV: c_uint = 4;

/// `IP_DEAD` of <ipc/ipc_port.h>: the one non-null pointer `IP_VALID()`
/// rejects.
const IP_DEAD: usize = usize::MAX;

/// `TASK_ACTIVE`, `TASK_MAY_ASSIGN` and `TASK_ESSENTIAL`: the three
/// single-bit fields `struct task` packs into one `unsigned char`.
const TASK_ACTIVE: u8 = 1 << 0;
const TASK_MAY_ASSIGN: u8 = 1 << 1;
const TASK_ESSENTIAL: u8 = 1 << 2;

/// `struct task` of <kern/task.h>: the task record itself.  The C packs the
/// three boolean flags into one `unsigned char`: `active` in bit 0,
/// `may_assign` in bit 1 and `essential` in bit 2.
#[repr(C)]
pub struct Task {
    /// `lock`: the task lock.
    pub lock: SimpleLock,
    pub ref_count: c_int,
    /// `assign_active`: waiting for `may_assign`.
    pub assign_active: u8,
    /// `active`, `may_assign` and `essential`, in that bit order.
    pub flags: u8,
    /// `map`: the address space.  `vm_map_t`, opaque here.
    pub map: *mut c_void,
    /// `pset_tasks`: link in the assigned processor set's task queue.
    pub pset_tasks: QueueEntry,
    pub suspend_count: c_int,
    /// `thread_list`: the task's thread queue head.
    pub thread_list: QueueEntry,
    pub thread_count: c_int,
    pub processor_set: *mut ProcessorSet,
    pub user_stop_count: c_int,
    pub priority: c_int,
    pub max_priority: c_int,
    pub total_user_time: TimeValue64,
    pub total_system_time: TimeValue64,
    pub creation_time: TimeValue64,
    /// `itk_lock_data`: protects the registered-port fields below.
    pub itk_lock_data: SimpleLock,
    pub itk_self: *mut c_void,
    pub itk_sself: *mut c_void,
    pub itk_exception: *mut c_void,
    pub itk_bootstrap: *mut c_void,
    pub itk_registered: [*mut c_void; TASK_PORT_REGISTER_MAX],
    pub itk_space: *mut c_void,
    pub eml_dispatch: *mut c_void,
    pub machine: MachineTask,
    pub faults: c_ulong,
    pub zero_fills: c_ulong,
    pub reactivations: c_ulong,
    pub pageins: c_ulong,
    pub cow_faults: c_ulong,
    pub messages_sent: c_ulong,
    pub messages_received: c_ulong,
    pub name: [c_char; TASK_NAME_SIZE],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Task>() == 336);
    assert!(align_of::<Task>() == 8);
    assert!(offset_of!(Task, lock) == 0);
    assert!(offset_of!(Task, ref_count) == 4);
    assert!(offset_of!(Task, assign_active) == 8);
    assert!(offset_of!(Task, flags) == 9);
    assert!(offset_of!(Task, map) == 16);
    assert!(offset_of!(Task, pset_tasks) == 24);
    assert!(offset_of!(Task, suspend_count) == 40);
    assert!(offset_of!(Task, thread_list) == 48);
    assert!(offset_of!(Task, thread_count) == 64);
    assert!(offset_of!(Task, processor_set) == 72);
    assert!(offset_of!(Task, user_stop_count) == 80);
    assert!(offset_of!(Task, priority) == 84);
    assert!(offset_of!(Task, max_priority) == 88);
    assert!(offset_of!(Task, total_user_time) == 96);
    assert!(offset_of!(Task, total_system_time) == 112);
    assert!(offset_of!(Task, creation_time) == 128);
    assert!(offset_of!(Task, itk_lock_data) == 144);
    assert!(offset_of!(Task, itk_self) == 152);
    assert!(offset_of!(Task, itk_sself) == 160);
    assert!(offset_of!(Task, itk_exception) == 168);
    assert!(offset_of!(Task, itk_bootstrap) == 176);
    assert!(offset_of!(Task, itk_registered) == 184);
    assert!(offset_of!(Task, itk_space) == 216);
    assert!(offset_of!(Task, eml_dispatch) == 224);
    assert!(offset_of!(Task, machine) == 232);
    assert!(offset_of!(Task, faults) == 248);
    assert!(offset_of!(Task, zero_fills) == 256);
    assert!(offset_of!(Task, reactivations) == 264);
    assert!(offset_of!(Task, pageins) == 272);
    assert!(offset_of!(Task, cow_faults) == 280);
    assert!(offset_of!(Task, messages_sent) == 288);
    assert!(offset_of!(Task, messages_received) == 296);
    assert!(offset_of!(Task, name) == 304);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Task>() == 220);
    assert!(align_of::<Task>() == 4);
    assert!(offset_of!(Task, lock) == 0);
    assert!(offset_of!(Task, ref_count) == 4);
    assert!(offset_of!(Task, assign_active) == 8);
    assert!(offset_of!(Task, flags) == 9);
    assert!(offset_of!(Task, map) == 12);
    assert!(offset_of!(Task, pset_tasks) == 16);
    assert!(offset_of!(Task, suspend_count) == 24);
    assert!(offset_of!(Task, thread_list) == 28);
    assert!(offset_of!(Task, thread_count) == 36);
    assert!(offset_of!(Task, processor_set) == 40);
    assert!(offset_of!(Task, user_stop_count) == 44);
    assert!(offset_of!(Task, priority) == 48);
    assert!(offset_of!(Task, max_priority) == 52);
    assert!(offset_of!(Task, total_user_time) == 56);
    assert!(offset_of!(Task, total_system_time) == 72);
    assert!(offset_of!(Task, creation_time) == 88);
    assert!(offset_of!(Task, itk_lock_data) == 104);
    assert!(offset_of!(Task, itk_self) == 108);
    assert!(offset_of!(Task, itk_sself) == 112);
    assert!(offset_of!(Task, itk_exception) == 116);
    assert!(offset_of!(Task, itk_bootstrap) == 120);
    assert!(offset_of!(Task, itk_registered) == 124);
    assert!(offset_of!(Task, itk_space) == 140);
    assert!(offset_of!(Task, eml_dispatch) == 144);
    assert!(offset_of!(Task, machine) == 148);
    assert!(offset_of!(Task, faults) == 160);
    assert!(offset_of!(Task, zero_fills) == 164);
    assert!(offset_of!(Task, reactivations) == 168);
    assert!(offset_of!(Task, pageins) == 172);
    assert!(offset_of!(Task, cow_faults) == 176);
    assert!(offset_of!(Task, messages_sent) == 180);
    assert!(offset_of!(Task, messages_received) == 184);
    assert!(offset_of!(Task, name) == 188);
};

impl Task {
    pub(crate) fn active(&self) -> bool {
        self.flags & TASK_ACTIVE != 0
    }

    pub(crate) fn set_active(&mut self, active: bool) {
        if active {
            self.flags |= TASK_ACTIVE;
        } else {
            self.flags &= !TASK_ACTIVE;
        }
    }

    fn may_assign(&self) -> bool {
        self.flags & TASK_MAY_ASSIGN != 0
    }

    fn set_may_assign(&mut self, may_assign: bool) {
        if may_assign {
            self.flags |= TASK_MAY_ASSIGN;
        } else {
            self.flags &= !TASK_MAY_ASSIGN;
        }
    }

    fn set_essential(&mut self, essential: bool) {
        if essential {
            self.flags |= TASK_ESSENTIAL;
        } else {
            self.flags &= !TASK_ESSENTIAL;
        }
    }
}

/// `struct task_basic_info` of <mach/task_info.h>.
#[repr(C)]
struct TaskBasicInfo {
    suspend_count: c_int,
    base_priority: c_int,
    virtual_size: usize,
    resident_size: usize,
    user_time: RpcTimeValue,
    system_time: RpcTimeValue,
    creation_time: RpcTimeValue,
    user_time64: TimeValue64,
    system_time64: TimeValue64,
    creation_time64: TimeValue64,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<TaskBasicInfo>() == 120);
    assert!(offset_of!(TaskBasicInfo, suspend_count) == 0);
    assert!(offset_of!(TaskBasicInfo, base_priority) == 4);
    assert!(offset_of!(TaskBasicInfo, virtual_size) == 8);
    assert!(offset_of!(TaskBasicInfo, resident_size) == 16);
    assert!(offset_of!(TaskBasicInfo, user_time) == 24);
    assert!(offset_of!(TaskBasicInfo, system_time) == 40);
    assert!(offset_of!(TaskBasicInfo, creation_time) == 56);
    assert!(offset_of!(TaskBasicInfo, user_time64) == 72);
    assert!(offset_of!(TaskBasicInfo, system_time64) == 88);
    assert!(offset_of!(TaskBasicInfo, creation_time64) == 104);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<TaskBasicInfo>() == 88);
    assert!(offset_of!(TaskBasicInfo, suspend_count) == 0);
    assert!(offset_of!(TaskBasicInfo, base_priority) == 4);
    assert!(offset_of!(TaskBasicInfo, virtual_size) == 8);
    assert!(offset_of!(TaskBasicInfo, resident_size) == 12);
    assert!(offset_of!(TaskBasicInfo, user_time) == 16);
    assert!(offset_of!(TaskBasicInfo, system_time) == 24);
    assert!(offset_of!(TaskBasicInfo, creation_time) == 32);
    assert!(offset_of!(TaskBasicInfo, user_time64) == 40);
    assert!(offset_of!(TaskBasicInfo, system_time64) == 56);
    assert!(offset_of!(TaskBasicInfo, creation_time64) == 72);
};

/// `struct task_events_info` of <mach/task_info.h>.
#[repr(C)]
struct TaskEventsInfo {
    faults: c_ulong,
    zero_fills: c_ulong,
    reactivations: c_ulong,
    pageins: c_ulong,
    cow_faults: c_ulong,
    messages_sent: c_ulong,
    messages_received: c_ulong,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<TaskEventsInfo>() == 56);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<TaskEventsInfo>() == 28);

/// `struct task_thread_times_info` of <mach/task_info.h>.
#[repr(C)]
struct TaskThreadTimesInfo {
    user_time: RpcTimeValue,
    system_time: RpcTimeValue,
    user_time64: TimeValue64,
    system_time64: TimeValue64,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<TaskThreadTimesInfo>() == 64);
    assert!(offset_of!(TaskThreadTimesInfo, user_time64) == 32);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<TaskThreadTimesInfo>() == 48);
    assert!(offset_of!(TaskThreadTimesInfo, user_time64) == 16);
};

/// The C's `sizeof(struct T) / sizeof(integer_t)`, the `TASK_*_COUNT` of
/// <mach/task_info.h>.  Every count is a few dozen, so the narrowing cannot
/// lose a bit.
const fn info_count(size: usize) -> c_uint {
    (size / size_of::<c_int>()) as c_uint
}

const TASK_BASIC_INFO_COUNT: c_uint = info_count(size_of::<TaskBasicInfo>());
const TASK_BASIC_INFO_LEGACY_COUNT: usize =
    offset_of!(TaskBasicInfo, user_time64) / size_of::<c_int>();
const TASK_EVENTS_INFO_COUNT: c_uint = info_count(size_of::<TaskEventsInfo>());
const TASK_THREAD_TIMES_INFO_COUNT: c_uint =
    info_count(size_of::<TaskThreadTimesInfo>());
const TASK_THREAD_TIMES_INFO_LEGACY_COUNT: usize =
    offset_of!(TaskThreadTimesInfo, user_time64) / size_of::<c_int>();

/// `struct pmap_statistics` of <mach/vm_statistics.h>.
#[repr(C)]
struct PmapStatistics {
    resident_count: c_int,
    wired_count: c_int,
}

/// The `struct pmap` prefix through `stats`, whose `resident_count` the C
/// `pmap_resident_count()` macro of <i386/intel/pmap.h> reads.
#[repr(C)]
struct PmapPrefix {
    /// `dirbase` on i386, `pdpbase` with PAE and `l4base` on x86_64: a
    /// pointer in every configuration.
    _page_table: *mut c_void,
    ref_count: c_int,
    lock: SimpleLock,
    stats: PmapStatistics,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(PmapPrefix, ref_count) == 8);
    assert!(offset_of!(PmapPrefix, lock) == 12);
    assert!(offset_of!(PmapPrefix, stats) == 16);
    assert!(offset_of!(PmapStatistics, resident_count) == 0);
    assert!(offset_of!(PmapStatistics, wired_count) == 4);
    assert!(size_of::<PmapStatistics>() == 8);
};
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(offset_of!(PmapPrefix, ref_count) == 4);
    assert!(offset_of!(PmapPrefix, lock) == 8);
    assert!(offset_of!(PmapPrefix, stats) == 12);
    assert!(offset_of!(PmapStatistics, resident_count) == 0);
    assert!(offset_of!(PmapStatistics, wired_count) == 4);
    assert!(size_of::<PmapStatistics>() == 8);
};

/// `kernel_task` of <kern/task.h>: the kernel's own task, the first created.
#[unsafe(no_mangle)]
pub static mut kernel_task: *mut Task = null_mut();

/// `task_cache` of kern/task.c: the `struct task` slab cache.
#[unsafe(export_name = "task_cache")]
static mut TASK_CACHE: KmemCache = KmemCache::zeroed();

/// `new_task_notification` of kern/task.c: the port new-task notifications
/// go to, or null.
#[unsafe(no_mangle)]
pub static mut new_task_notification: *mut c_void = null_mut();

/// `task_collect_allowed` of kern/task.c: whether the collector may run.
#[unsafe(export_name = "task_collect_allowed")]
static mut TASK_COLLECT_ALLOWED: c_int = 1;

/// `task_collect_last_tick` and `task_collect_max_rate` of kern/task.c: the
/// last tick the collector ran and the minimum interval, in ticks.
#[unsafe(export_name = "task_collect_last_tick")]
static mut TASK_COLLECT_LAST_TICK: c_uint = 0;
#[unsafe(export_name = "task_collect_max_rate")]
static mut TASK_COLLECT_MAX_RATE: c_uint = 0;

/// `current_task()` of <kern/thread.h>: the running thread's task.
///
/// # Safety
///
/// Must be called from a thread context: every running CPU has a live
/// current thread with a live task.
pub(crate) unsafe fn current_task() -> *mut Task {
    // SAFETY: the caller's contract.
    unsafe { (*current_thread()).task }
}

/// `pmap_resident_count()` of <i386/intel/pmap.h>: the pages the pmap has
/// resident.
///
/// # Safety
///
/// `pmap` must point at a live `struct pmap`.
unsafe fn resident_count(pmap: *mut Pmap) -> c_int {
    // SAFETY: the caller promises the live pmap; `PmapPrefix` mirrors the
    // fields of `struct pmap` up to `stats`.
    unsafe { (*pmap.cast::<PmapPrefix>()).stats.resident_count }
}

/// The `time_value64_add()` macro of <mach/time_value.h>.
pub(crate) fn add_time64(result: &mut TimeValue64, addend: TimeValue64) {
    result.seconds = result.seconds.wrapping_add(addend.seconds);
    result.nanoseconds = result.nanoseconds.wrapping_add(addend.nanoseconds);
    if result.nanoseconds >= TIME_NANOS_MAX {
        result.nanoseconds = result.nanoseconds.wrapping_sub(TIME_NANOS_MAX);
        result.seconds = result.seconds.wrapping_add(1);
    }
}

/// How [`create_kernel_task()`] chooses the new task's map: the C's
/// `child_task == &kernel_task` test and `inherit_memory` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapSource {
    /// The kernel task's `kernel_map`, already built.
    Kernel,
    /// A fork of the parent's map.
    Inherit,
    /// A fresh user map, limited like the parent's when there is one.
    Fresh,
}

/// `task_init()` of kern/task.c.
///
/// # Safety
///
/// Runs once, from the boot sequence, after the slab and IPC packages are
/// initialized and before any other task exists.
pub(crate) unsafe fn init() {
    // SAFETY: the caller runs this once before anything allocates from the
    // cache, so nothing else sees the storage while the slab layer builds
    // it.
    unsafe {
        (*addr_of_mut!(TASK_CACHE)).init(
            b"task",
            size_of::<Task>(),
            0,
            None,
            CacheInitFlags::EMPTY,
        );
    }
    eml_init();
    // SAFETY: as above: this runs once, before any task exists and so before
    // anything can allocate from the I/O-permission cache.
    unsafe { machine_task_module_init() };

    // SAFETY: the cache is live and the caller holds no locks.
    let task =
        match unsafe { create_kernel_task(null_mut(), MapSource::Kernel) } {
            Ok(task) => task,
            // The C ignored the failure and dereferenced the null `kernel_task`
            // on its next line; a shortage this early is fatal either way.
            Err(_) => unsafe {
                glue::Panic(
                    c"kern/task.c".as_ptr(),
                    line!() as c_int,
                    c"task_init".as_ptr(),
                    c"task_init: cannot create the kernel task".as_ptr(),
                )
            },
        };
    // SAFETY: this is the only writer, and it runs once.
    unsafe { kernel_task = task };

    // SAFETY: the new kernel task is live and the name is static.
    let _ = unsafe { set_name(task, b"gnumach") };
    // SAFETY: the kernel task's map is `kernel_map`, live since the VM
    // bootstrap, and its name points at the task's own name buffer.
    unsafe {
        let map = glue::kernel_map.cast::<VmMap>();
        (*map).name = addr_of!((*task).name).cast::<c_char>();
    }
}

/// `task_create_kernel()` of kern/task.c.
///
/// # Safety
///
/// `parent` must be null or point at a live task whose map outlives the
/// call, and the caller must hold no locks: creation may block on the memory
/// it takes.
pub(crate) unsafe fn create_kernel_task(
    parent: *mut Task,
    source: MapSource,
) -> Result<*mut Task, KernError> {
    // SAFETY: the caller runs after `task_init()`, so the cache is live.
    let Some(buf) = (unsafe { (*addr_of_mut!(TASK_CACHE)).alloc() }) else {
        return Err(KernError::ResourceShortage);
    };
    let task = buf.as_ptr().cast::<Task>();

    // SAFETY: the task is fresh, unshared storage and every field is written
    // before anything reads it.
    unsafe {
        addr_of_mut!((*task).ref_count).write(2);
    }

    let map = match source {
        // SAFETY: `kernel_map` is live from the VM bootstrap on.
        MapSource::Kernel => unsafe { glue::kernel_map }.cast::<VmMap>(),
        MapSource::Inherit => {
            // SAFETY: the caller promises a live parent with a map.
            let parent_map = unsafe {
                NonNull::new_unchecked((*parent).map.cast::<VmMap>())
            };
            VmMap::fork(parent_map).map_or(null_mut(), NonNull::as_ptr)
        }
        MapSource::Fresh => {
            // SAFETY: the caller holds no locks, and the pmap layer is up.
            let pmap = unsafe { glue::pmap_create(0) };
            if pmap.is_null() {
                null_mut()
            } else {
                let created = VmMap::create(
                    pmap,
                    round_page(VM_MIN_USER_ADDRESS),
                    trunc_page(VM_MAX_USER_ADDRESS),
                );
                match created {
                    Some(map) => {
                        if !parent.is_null() {
                            // SAFETY: the caller promises a live parent and
                            // holds no locks, as the read lock needs.
                            unsafe {
                                let parent_map = NonNull::new_unchecked(
                                    (*parent).map.cast::<VmMap>(),
                                );
                                (*parent_map.as_ptr()).lock.read();
                                VmMap::copy_limits(map, parent_map);
                                (*parent_map.as_ptr()).lock.done();
                            }
                        }
                        map.as_ptr()
                    }
                    None => {
                        // SAFETY: the pmap came from `pmap_create()` just
                        // above.
                        unsafe { glue::pmap_destroy(pmap) };
                        null_mut()
                    }
                }
            }
        }
    };

    if map.is_null() {
        // SAFETY: the task is the allocation just made, with no other
        // holder.
        unsafe {
            (*addr_of_mut!(TASK_CACHE)).free(buf);
        }
        return Err(KernError::ResourceShortage);
    }

    // SAFETY: the task is unshared storage and the map is live; each field
    // is written once, before any read.
    unsafe {
        addr_of_mut!((*task).map).write(map.cast());
        if source != MapSource::Kernel {
            // `vm_map_set_name()` of <vm/vm_map.h>, the C's inline.
            (*map).name = addr_of!((*task).name).cast();
        }
        addr_of_mut!((*task).flags).write(TASK_ACTIVE);
        addr_of_mut!((*task).assign_active).write(0);
        (*task).lock.init();
        queue_init(addr_of_mut!((*task).thread_list));
        addr_of_mut!((*task).suspend_count).write(0);
        addr_of_mut!((*task).user_stop_count).write(0);
        addr_of_mut!((*task).thread_count).write(0);
        addr_of_mut!((*task).faults).write(0);
        addr_of_mut!((*task).zero_fills).write(0);
        addr_of_mut!((*task).reactivations).write(0);
        addr_of_mut!((*task).pageins).write(0);
        addr_of_mut!((*task).cow_faults).write(0);
        addr_of_mut!((*task).messages_sent).write(0);
        addr_of_mut!((*task).messages_received).write(0);
    }

    // SAFETY: the new task is live and unshared; the parent, when there is
    // one, is live as the caller promised.  All four calls only read the
    // parent and initialize the task's own fields.
    unsafe {
        glue::eml_task_reference(task, parent);
        ipc_task_init(task, parent);
        glue::machine_task_init(task);

        addr_of_mut!((*task).total_user_time).write(TimeValue64::default());
        addr_of_mut!((*task).total_system_time).write(TimeValue64::default());
        glue::record_time_stamp(addr_of_mut!((*task).creation_time));
    }

    let pset;
    if !parent.is_null() {
        // SAFETY: the caller promises a live parent; its lock protects the
        // processor-set field and the priorities below, and is held across
        // the reference so the set cannot go away.
        unsafe {
            (*parent).lock.lock();
            let parent_pset = (*parent).processor_set;
            pset = if (*parent_pset).active != 0 {
                parent_pset
            } else {
                // `default_pset` is the C global live for the life of the
                // kernel.
                addr_of_mut!(glue::default_pset).cast::<ProcessorSet>()
            };
            pset_reference(pset);
            addr_of_mut!((*task).priority).write((*parent).priority);
            addr_of_mut!((*task).max_priority).write((*parent).max_priority);
            (*parent).lock.unlock();
        }
    } else {
        // `default_pset` is the C global live for the life of the kernel.
        pset = addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
        // SAFETY: `default_pset` is the live default set.
        unsafe { pset_reference(pset) };
        // SAFETY: the new task is unshared; the C raised the priority to the
        // set's own when that is higher than `BASEPRI_USER`.
        unsafe {
            addr_of_mut!((*task).priority).write(BASEPRI_USER);
            let max_priority = (*pset).max_priority;
            addr_of_mut!((*task).max_priority).write(max_priority);
            if max_priority > BASEPRI_USER {
                addr_of_mut!((*task).priority).write(max_priority);
            }
        }
    }

    // SAFETY: the set is live and referenced; the C took its lock around the
    // queue insert.
    unsafe {
        (*pset).lock.lock();
        glue::pset_add_task(pset, task);
        (*pset).lock.unlock();

        addr_of_mut!((*task).flags).write(TASK_ACTIVE | TASK_MAY_ASSIGN);
    }

    // SAFETY: both name buffers are live; `snprintf` writes at most its size
    // argument, and the task's own name is the destination.
    unsafe {
        let name = addr_of_mut!((*task).name).cast::<c_char>();
        if parent.is_null() {
            glue::snprintf(name, TASK_NAME_SIZE, c"%p".as_ptr(), task);
        } else {
            // The C's `(int)(sizeof name - 3)`; 32 always fits an `int`.
            let precision = TASK_NAME_SIZE as c_int - 3;
            glue::snprintf(
                name,
                TASK_NAME_SIZE,
                c"(%.*s)".as_ptr(),
                precision,
                addr_of!((*parent).name).cast::<c_char>(),
            );
        }
    }

    // SAFETY: the notification global is the C's `ipc_port_t`; both
    // conversions and the references follow the C body.  `reference()`
    // accepts a null parent.
    unsafe {
        if !new_task_notification.is_null() {
            reference(task);
            reference(parent);
            glue::mach_notify_new_task(
                new_task_notification,
                convert_task_to_port(task).map_or(null_mut(), IpcPort::as_ptr),
                if parent.is_null() {
                    null_mut()
                } else {
                    convert_task_to_port(parent)
                        .map_or(null_mut(), IpcPort::as_ptr)
                },
            );
        }
        ipc_task_enable(task);
    }

    Ok(task)
}

/// `task_deallocate()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task the caller holds a reference
/// to, and the caller must hold no locks: the cleanup may block.
pub(crate) unsafe fn deallocate(task: *mut Task) {
    if task.is_null() {
        return;
    }

    // SAFETY: the caller promises a live task and its reference; the lock
    // protects the count.
    let count = unsafe {
        (*task).lock.lock();
        let count = (*task).ref_count.wrapping_sub(1);
        (*task).ref_count = count;
        (*task).lock.unlock();
        count
    };
    if count != 0 {
        return;
    }

    // SAFETY: this is the last reference, so the machine data and emulation
    // vector belong to this call.
    unsafe {
        glue::machine_task_terminate(task);
        glue::eml_task_deallocate(task);
    }

    // SAFETY: a live task's processor-set field was set by `pset_add_task()`.
    let pset = unsafe { (*task).processor_set };
    // SAFETY: the set is live; its lock serializes the removal.
    unsafe {
        (*pset).lock.lock();
        glue::pset_remove_task(pset, task);
        (*pset).lock.unlock();
    }
    // SAFETY: the set is live and the reference taken at creation moves
    // here.
    unsafe { pset_deallocate(pset) };

    // SAFETY: the task's map and IPC space are live, and each holds one of
    // the task's own references.
    unsafe {
        if let Some(map) = NonNull::new((*task).map.cast::<VmMap>()) {
            VmMap::deallocate(map);
        }
        ipc_space::release(IpcSpace::from_raw((*task).itk_space));
    }

    // SAFETY: the task came from the cache and nothing references it now.
    unsafe {
        (*addr_of_mut!(TASK_CACHE))
            .free(NonNull::new_unchecked(task.cast::<u8>()));
    }
}

/// `task_reference()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task.
pub(crate) unsafe fn reference(task: *mut Task) {
    if task.is_null() {
        return;
    }

    // SAFETY: the caller promises a live task; the lock protects the count.
    unsafe {
        (*task).lock.lock();
        (*task).ref_count = (*task).ref_count.wrapping_add(1);
        (*task).lock.unlock();
    }
}

/// `task_terminate()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks: the routine blocks and deallocates.
pub(crate) unsafe fn terminate(task: *mut Task) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task.
    let list = unsafe { addr_of_mut!((*task).thread_list) };
    // SAFETY: the caller runs on a live thread with a live task.
    let cur_task = unsafe { current_task() };
    let cur_thread = current_thread();

    if task == cur_task {
        // SAFETY: the caller promises a live task, and the current thread is
        // live; the C's lock order is kept.
        unsafe {
            (*task).lock.lock();
            if !(*task).active() {
                (*task).lock.unlock();
                return Err(KernError::Failure);
            }
            let s = glue::splsched();
            (*cur_thread).lock.lock();
            if !(*cur_thread).active() {
                (*cur_thread).lock.unlock();
                glue::splx(s);
                (*task).lock.unlock();
                let _ = Thread::terminate(cur_thread);
                return Err(KernError::Failure);
            }
            hold_locked(task);
            (*task).set_active(false);
            queue_remove_generic(
                list,
                cur_thread.cast::<c_void>(),
                offset_of!(Thread, thread_list),
            );
            (*cur_thread).lock.unlock();
            glue::splx(s);
            (*task).lock.unlock();

            // The current thread must be left alone to terminate the task.
            ipc_thread_disable(cur_thread);
            ipc_thread_terminate(cur_thread);
        }
    } else {
        // SAFETY: the caller promises a live task; the current thread and
        // task are live too.
        unsafe {
            if task.addr() < cur_task.addr() {
                (*task).lock.lock();
                (*cur_task).lock.lock();
            } else {
                (*cur_task).lock.lock();
                (*task).lock.lock();
            }

            let s = glue::splsched();
            (*cur_thread).lock.lock();
            if !(*cur_task).active() || !(*cur_thread).active() {
                (*cur_thread).lock.unlock();
                glue::splx(s);
                (*task).lock.unlock();
                (*cur_task).lock.unlock();
                let _ = Thread::terminate(cur_thread);
                return Err(KernError::Failure);
            }
            (*cur_thread).lock.unlock();
            glue::splx(s);
            (*cur_task).lock.unlock();

            if !(*task).active() {
                (*task).lock.unlock();
                return Err(KernError::Failure);
            }
            hold_locked(task);
            (*task).set_active(false);
            (*task).lock.unlock();
        }
    }

    // SAFETY: the caller promises a live task, and the C ignored the wait's
    // result as this does.
    let _ = unsafe { dowait(task, true) };

    // SAFETY: the caller promises a live task, and its IPC state is up.
    unsafe { ipc_task_disable(task) };

    // SAFETY: the task is live and unlocked here, as the C's loop needs;
    // each removed thread holds a reference while it is walked.
    unsafe {
        (*task).lock.lock();
        while queue_empty(list) == 0 {
            let mut thread = queue_first(list).cast::<Thread>();
            Thread::reference(thread);

            loop {
                let next = queue_next(addr_of_mut!((*thread).thread_list))
                    .cast::<Thread>();

                if queue_end(list, next.cast()) == 0 {
                    Thread::reference(next);
                }

                (*task).lock.unlock();
                Thread::force_terminate(thread);
                Thread::deallocate(thread);
                glue::thread_block(None);
                thread = next;
                (*task).lock.lock();
                if queue_end(list, thread.cast()) != 0 {
                    break;
                }
            }
        }
        (*task).lock.unlock();
    }

    // SAFETY: as above; the C tore the IPC state down here.
    unsafe { ipc_task_terminate(task) };

    // SAFETY: the caller promises the task's own reference moves here.
    unsafe { deallocate(task) };

    // SAFETY: the current thread's `task` field is live; when it names this
    // task, the thread still holds a reference, so the task was not freed
    // above.
    unsafe {
        if (*cur_thread).task == task {
            (*task).lock.lock();
            let s = glue::splsched();
            queue_enter_tail(
                list,
                cur_thread.cast::<c_void>(),
                offset_of!(Thread, thread_list),
            );
            glue::splx(s);
            (*task).lock.unlock();
            let _ = Thread::terminate(cur_thread);
        }
    }

    Ok(())
}

/// `task_hold_locked()` of kern/task.c.
///
/// # Safety
///
/// The caller must hold `task`'s lock, and `task` must be live.
pub(crate) unsafe fn hold_locked(task: *mut Task) {
    let cur_thread = current_thread();

    // SAFETY: the caller promises the locked, live task; the queue links are
    // stable under the lock and every entry is a live thread.
    unsafe {
        (*task).suspend_count = (*task).suspend_count.wrapping_add(1);

        let list = addr_of_mut!((*task).thread_list);
        let mut entry = queue_first(list);
        while queue_end(list, entry) == 0 {
            let thread = entry.cast::<Thread>();
            if thread != cur_thread {
                Thread::hold(thread);
            }
            entry = queue_next(addr_of_mut!((*thread).thread_list));
        }
    }
}

/// `task_hold()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks.
pub(crate) unsafe fn hold(task: *mut Task) -> Result<(), KernError> {
    // SAFETY: the caller promises a live task; the lock is taken here.
    unsafe {
        (*task).lock.lock();
        if !(*task).active() {
            (*task).lock.unlock();
            return Err(KernError::Failure);
        }
        hold_locked(task);
        (*task).lock.unlock();
    }
    Ok(())
}

/// `task_dowait()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks: the routine waits and may block.
pub(crate) unsafe fn dowait(
    task: *mut Task,
    must_wait: bool,
) -> Result<(), KernError> {
    let cur_thread = current_thread();
    // SAFETY: the caller promises a live task.
    let list = unsafe { addr_of_mut!((*task).thread_list) };
    let mut prev_thread: *mut Thread = null_mut();
    let mut result = Ok(());

    // SAFETY: the caller promises the live task; the lock protects the queue
    // and the walk holds a reference on the thread it is between.
    unsafe {
        (*task).lock.lock();
        let mut entry = queue_first(list);
        while queue_end(list, entry) == 0 {
            let thread = entry.cast::<Thread>();

            if !(*task).active() && !must_wait {
                result = Err(KernError::Failure);
                break;
            }

            if thread != cur_thread {
                Thread::reference(thread);
                (*task).lock.unlock();
                if !prev_thread.is_null() {
                    Thread::deallocate(prev_thread);
                }
                let _ = Thread::dowait(thread, true);
                prev_thread = thread;
                (*task).lock.lock();
            }

            entry = queue_next(addr_of_mut!((*thread).thread_list));
        }
        (*task).lock.unlock();

        if !prev_thread.is_null() {
            Thread::deallocate(prev_thread);
        }
    }

    result
}

/// `task_release()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks.
pub(crate) unsafe fn release(task: *mut Task) -> Result<(), KernError> {
    // SAFETY: the caller promises a live task; the lock protects the queue
    // and the stop count.
    unsafe {
        (*task).lock.lock();
        if !(*task).active() {
            (*task).lock.unlock();
            return Err(KernError::Failure);
        }

        (*task).suspend_count = (*task).suspend_count.wrapping_sub(1);

        let list = addr_of_mut!((*task).thread_list);
        let mut entry = queue_first(list);
        while queue_end(list, entry) == 0 {
            let thread = entry.cast::<Thread>();
            let next = queue_next(addr_of_mut!((*thread).thread_list));
            Thread::release(thread);
            entry = next;
        }
        (*task).lock.unlock();
    }

    Ok(())
}

/// `task_threads()` of kern/task.c: the live threads of `task`, each
/// converted to a port name the caller owns.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks: the routine allocates.
pub(crate) unsafe fn threads(
    task: *mut Task,
) -> Result<(Option<NonNull<VmOffset>>, c_uint), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    let mut size: VmSize = 0;
    let mut addr: Option<NonNull<u8>> = None;
    let mut actual: c_uint;
    let mut size_needed: usize;

    loop {
        // SAFETY: the caller promises a live task; the lock is taken here,
        // and the break below leaves it held.
        unsafe {
            (*task).lock.lock();
            if !(*task).active() {
                (*task).lock.unlock();
                return Err(KernError::Failure);
            }

            // The C read the `int` count into an `unsigned int`; it is the
            // number of threads and never negative.
            actual = (*task).thread_count as c_uint;
            // `sizeof(mach_port_t)` is a `vm_offset_t` on the kernel side,
            // and the `unsigned int` count widens on both targets.
            size_needed = actual as usize * size_of::<VmOffset>();
            if size_needed <= size {
                break;
            }

            (*task).lock.unlock();
        }

        if let Some(old) = addr {
            // SAFETY: the old buffer is the live allocation of `size` bytes
            // made above.
            unsafe { kfree(old, size) };
        }
        size = size_needed;
        // SAFETY: `kalloc_init()` ran during the boot this MIG entry
        // follows.
        let Some(buf) = kalloc(size) else {
            return Err(KernError::ResourceShortage);
        };
        addr = Some(buf);
    }

    let Some(buf) = addr else {
        // The count was zero on the first look, so nothing was allocated.
        // SAFETY: the task lock was left held by the break above.
        unsafe { (*task).lock.unlock() };
        return Ok((None, 0));
    };

    if actual == 0 {
        // SAFETY: the task lock is still held by the break above, and the
        // buffer is the live allocation of `size` bytes.
        unsafe {
            (*task).lock.unlock();
            kfree(buf, size);
        }
        return Ok((None, 0));
    }

    let mut threads = buf.as_ptr().cast::<VmOffset>();
    // SAFETY: the task lock is held, so every queue entry is a live thread,
    // and the references taken here keep them alive.  An address is the same
    // width as the thread pointers the C stored.
    unsafe {
        let list = addr_of_mut!((*task).thread_list);
        let mut entry = queue_first(list);
        for i in 0..actual as usize {
            let thread = entry.cast::<Thread>();
            Thread::reference(thread);
            threads.add(i).write(thread.addr());
            entry = queue_next(addr_of_mut!((*thread).thread_list));
        }
        (*task).lock.unlock();
    }

    if size_needed < size {
        // `actual` is nonzero here, so the smaller size is too.
        // SAFETY: `kalloc_init()` ran during the boot.
        let Some(new) = kalloc(size_needed) else {
            // SAFETY: every slot holds a referenced thread, and the buffer
            // is the live allocation of `size` bytes.
            unsafe {
                for i in 0..actual as usize {
                    Thread::deallocate(with_exposed_provenance_mut(
                        threads.add(i).read(),
                    ));
                }
                kfree(buf, size);
            }
            return Err(KernError::ResourceShortage);
        };

        // SAFETY: both buffers are live and distinct, the copy fits the
        // smaller one, and the old allocation is released.
        unsafe {
            ptr::copy_nonoverlapping(buf.as_ptr(), new.as_ptr(), size_needed);
            kfree(buf, size);
        }
        threads = new.as_ptr().cast::<VmOffset>();
    }

    // SAFETY: every slot holds a referenced thread whose port conversion
    // hands the reference on; the buffer has room for all of them.
    unsafe {
        for i in 0..actual as usize {
            let port = convert_thread_to_port(with_exposed_provenance_mut(
                threads.add(i).read(),
            ));
            threads
                .add(i)
                .write(port.map_or(0, |port| port.as_ptr().addr()));
        }

        Ok((Some(NonNull::new_unchecked(threads)), actual))
    }
}

/// `task_suspend()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks.
pub(crate) unsafe fn suspend(task: *mut Task) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock protects the stop
    // count.
    let first_stop = unsafe {
        (*task).lock.lock();
        let first_stop = (*task).user_stop_count == 0;
        (*task).user_stop_count = (*task).user_stop_count.wrapping_add(1);
        (*task).lock.unlock();
        first_stop
    };

    if !first_stop {
        return Ok(());
    }

    // SAFETY: the caller's contract.
    unsafe { hold(task) }?;
    // SAFETY: as above.
    unsafe { dowait(task, false) }?;

    // SAFETY: the caller runs on a live thread.
    if unsafe { current_task() } == task {
        let thread = current_thread();
        // SAFETY: the current thread is live.
        unsafe { Thread::hold(thread) };
        // SAFETY: the C raised the level around `ast_on()`, whose slot write
        // takes care of itself.
        unsafe {
            let s = glue::splsched();
            ast_on(cpu_number(), AST_BLOCK);
            glue::splx(s);
        }
    }

    Ok(())
}

/// `task_resume()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks.
pub(crate) unsafe fn resume(task: *mut Task) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock protects the stop
    // count.
    let release_now = unsafe {
        (*task).lock.lock();
        if (*task).user_stop_count > 0 {
            (*task).user_stop_count = (*task).user_stop_count.wrapping_sub(1);
            let release_now = (*task).user_stop_count == 0;
            (*task).lock.unlock();
            release_now
        } else {
            (*task).lock.unlock();
            return Err(KernError::Failure);
        }
    };

    if release_now {
        // SAFETY: the caller's contract.
        unsafe { release(task) }
    } else {
        Ok(())
    }
}

/// `task_info()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and `out` must be writable
/// for the `count` `integer_t`s the caller passes.
pub(crate) unsafe fn info(
    task: *mut Task,
    flavor: c_int,
    out: *mut c_int,
    count: c_uint,
) -> Result<c_uint, KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    match flavor {
        TASK_BASIC_INFO => {
            // The count is a `natural_t`, and both targets widen it to `usize`.
            if (count as usize) < TASK_BASIC_INFO_LEGACY_COUNT {
                return Err(KernError::InvalidArgument);
            }

            let basic = out.cast::<TaskBasicInfo>();
            // SAFETY: `kernel_task` is live from `task_init()` on, and any
            // other live task's map is live.
            let map = if task == unsafe { kernel_task } {
                // SAFETY: `kernel_map` is live from the VM bootstrap on.
                unsafe { glue::kernel_map }.cast::<VmMap>()
            } else {
                // SAFETY: the caller promises a live task.
                unsafe { (*task).map }.cast::<VmMap>()
            };

            // SAFETY: the map is live, so its size field and pmap are too.
            let (virtual_size, resident) =
                unsafe { ((*map).size, resident_count((*map).pmap)) };
            // The C cast the non-negative page count to `rpc_vm_size_t`; the
            // count fits `usize`, and the product wraps in the C too.
            let resident_size = (resident as usize).wrapping_mul(PAGE_SIZE);
            // SAFETY: the caller promises the out buffer covers the legacy
            // count checked above.
            unsafe {
                addr_of_mut!((*basic).virtual_size).write(virtual_size);
                addr_of_mut!((*basic).resident_size).write(resident_size);
            }

            // SAFETY: the caller promises a live task; the lock covers the
            // statistics and the creation stamp.
            unsafe {
                (*task).lock.lock();
                addr_of_mut!((*basic).base_priority).write((*task).priority);
                addr_of_mut!((*basic).suspend_count)
                    .write((*task).user_stop_count);
                addr_of_mut!((*basic).user_time).write(RpcTimeValue::from(
                    TimeValue::from((*task).total_user_time),
                ));
                addr_of_mut!((*basic).system_time).write(RpcTimeValue::from(
                    TimeValue::from((*task).total_system_time),
                ));

                let mut creation_time64 = TimeValue64::default();
                read_time_stamp(
                    addr_of!((*task).creation_time),
                    addr_of_mut!(creation_time64),
                );
                addr_of_mut!((*basic).creation_time).write(
                    RpcTimeValue::from(TimeValue::from(creation_time64)),
                );

                if count == TASK_BASIC_INFO_COUNT {
                    addr_of_mut!((*basic).user_time64)
                        .write((*task).total_user_time);
                    addr_of_mut!((*basic).system_time64)
                        .write((*task).total_system_time);
                    addr_of_mut!((*basic).creation_time64)
                        .write(creation_time64);
                }
                (*task).lock.unlock();
            }

            if count > TASK_BASIC_INFO_COUNT {
                Ok(TASK_BASIC_INFO_COUNT)
            } else {
                Ok(count)
            }
        }
        TASK_EVENTS_INFO => {
            if count < TASK_EVENTS_INFO_COUNT {
                return Err(KernError::InvalidArgument);
            }

            let events = out.cast::<TaskEventsInfo>();
            // SAFETY: the caller promises a live task and its lock.
            unsafe {
                (*task).lock.lock();
                addr_of_mut!((*events).faults).write((*task).faults);
                addr_of_mut!((*events).zero_fills).write((*task).zero_fills);
                addr_of_mut!((*events).reactivations)
                    .write((*task).reactivations);
                addr_of_mut!((*events).pageins).write((*task).pageins);
                addr_of_mut!((*events).cow_faults).write((*task).cow_faults);
                addr_of_mut!((*events).messages_sent)
                    .write((*task).messages_sent);
                addr_of_mut!((*events).messages_received)
                    .write((*task).messages_received);
                (*task).lock.unlock();
            }

            Ok(TASK_EVENTS_INFO_COUNT)
        }
        TASK_THREAD_TIMES_INFO => {
            // The count is a `natural_t`, and both targets widen it to `usize`.
            if (count as usize) < TASK_THREAD_TIMES_INFO_LEGACY_COUNT {
                return Err(KernError::InvalidArgument);
            }

            let times = out.cast::<TaskThreadTimesInfo>();
            let mut acc_user = TimeValue64::default();
            let mut acc_system = TimeValue64::default();

            // SAFETY: the caller promises a live task, whose lock keeps the
            // thread queue stable; each thread's own lock covers its timers.
            unsafe {
                let list = addr_of_mut!((*task).thread_list);
                (*task).lock.lock();
                let mut entry = queue_first(list);
                while queue_end(list, entry) == 0 {
                    let thread = entry.cast::<Thread>();

                    let s = glue::splsched();
                    (*thread).lock.lock();
                    let mut user_time = TimeValue64::default();
                    let mut system_time = TimeValue64::default();
                    thread_read_times(
                        thread,
                        addr_of_mut!(user_time),
                        addr_of_mut!(system_time),
                    );
                    (*thread).lock.unlock();
                    glue::splx(s);

                    add_time64(&mut acc_user, user_time);
                    add_time64(&mut acc_system, system_time);

                    entry = queue_next(addr_of_mut!((*thread).thread_list));
                }
                (*task).lock.unlock();

                addr_of_mut!((*times).user_time)
                    .write(RpcTimeValue::from(TimeValue::from(acc_user)));
                addr_of_mut!((*times).system_time)
                    .write(RpcTimeValue::from(TimeValue::from(acc_system)));
                if count >= TASK_THREAD_TIMES_INFO_COUNT {
                    addr_of_mut!((*times).user_time64).write(acc_user);
                    addr_of_mut!((*times).system_time64).write(acc_system);
                }
            }

            if count > TASK_THREAD_TIMES_INFO_COUNT {
                Ok(TASK_THREAD_TIMES_INFO_COUNT)
            } else {
                Ok(count)
            }
        }
        _ => Err(KernError::InvalidArgument),
    }
}

/// `task_assign()` of kern/task.c, the `MACH_HOST` arm both configured
/// builds take.
///
/// # Safety
///
/// `task` must be null or point at a live task, `new_pset` must be null or
/// point at a live processor set, and the caller must hold no locks: the
/// routine waits and may block.
pub(crate) unsafe fn assign(
    task: *mut Task,
    new_pset: *mut ProcessorSet,
    assign_threads: bool,
) -> Result<(), KernError> {
    if task.is_null() || new_pset.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock is taken and
    // dropped around the wait, as the C did.
    unsafe {
        (*task).lock.lock();
        while !(*task).may_assign() {
            (*task).assign_active = 1;
            assert_wait(
                addr_of_mut!((*task).assign_active).cast::<c_void>(),
                c_int::from(true),
            );
            (*task).lock.unlock();
            glue::thread_block(None);
            (*task).lock.lock();
        }

        if (*task).processor_set == new_pset {
            (*task).lock.unlock();
            return Ok(());
        }

        (*task).set_may_assign(false);
        (*task).lock.unlock();
    }

    // SAFETY: the task lock was dropped above, so the set field is stable
    // for the freeze; a live task's set is live.
    let pset = unsafe { (*task).processor_set };
    let mut new_pset = new_pset;

    // SAFETY: both sets are live; the C locks them in address order to avoid
    // deadlock and re-checks the new one under the locks.
    unsafe {
        loop {
            if pset.addr() < new_pset.addr() {
                (*pset).lock.lock();
                (*new_pset).lock.lock();
            } else {
                (*new_pset).lock.lock();
                (*pset).lock.lock();
            }

            if (*new_pset).active == 0 {
                (*pset).lock.unlock();
                (*new_pset).lock.unlock();
                new_pset =
                    addr_of_mut!(glue::default_pset).cast::<ProcessorSet>();
                continue;
            }

            pset_reference(new_pset);
            break;
        }

        (*task).lock.lock();
        glue::pset_remove_task(pset, task);
        glue::pset_add_task(new_pset, task);
        (*pset).lock.unlock();
        (*new_pset).lock.unlock();
    }

    if !assign_threads {
        // SAFETY: the task lock is held here.
        unsafe {
            (*task).set_may_assign(true);
            if (*task).assign_active != 0 {
                (*task).assign_active = 0;
                thread_wakeup_prim(
                    addr_of_mut!((*task).assign_active).cast::<c_void>(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            (*task).lock.unlock();
        }
        // SAFETY: the set is live and referenced.
        unsafe { pset_deallocate(pset) };
        return Ok(());
    }

    // SAFETY: the task lock is held, and the current thread is live.
    unsafe {
        if current_task() == task {
            (*task).lock.unlock();
            Thread::freeze(current_thread());
            (*task).lock.lock();
        }
    }

    // SAFETY: the task lock is held; every queue entry is a live thread, and
    // the reference keeps the one between iterations alive.
    let result = unsafe {
        let list = addr_of_mut!((*task).thread_list);
        let mut prev_thread: *mut Thread = null_mut();
        let mut result = Ok(());
        let mut entry = queue_first(list);
        while queue_end(list, entry) == 0 {
            let thread = entry.cast::<Thread>();

            if !(*task).active() {
                result = Err(KernError::Failure);
                break;
            }

            if thread != current_thread() {
                Thread::reference(thread);
                (*task).lock.unlock();
                if !prev_thread.is_null() {
                    Thread::deallocate(prev_thread);
                }
                let _ = Thread::assign(thread, new_pset);
                prev_thread = thread;
                (*task).lock.lock();
            }

            entry = queue_next(addr_of_mut!((*thread).thread_list));
        }

        (*task).set_may_assign(true);
        if (*task).assign_active != 0 {
            (*task).assign_active = 0;
            thread_wakeup_prim(
                addr_of_mut!((*task).assign_active).cast::<c_void>(),
                0,
                THREAD_AWAKENED,
            );
        }
        (*task).lock.unlock();

        if !prev_thread.is_null() {
            Thread::deallocate(prev_thread);
        }

        if current_task() == task {
            Thread::doassign(current_thread(), new_pset, true);
        }

        result
    };

    // SAFETY: the set is live and referenced.
    unsafe { pset_deallocate(pset) };
    result
}

/// `task_get_assignment()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task.
pub(crate) unsafe fn get_assignment(
    task: *mut Task,
) -> Result<*mut ProcessorSet, KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the C read both fields
    // unlocked, and `pset_reference()` takes the set's own lock.
    unsafe {
        if !(*task).active() {
            return Err(KernError::Failure);
        }
        let pset = (*task).processor_set;
        pset_reference(pset);
        Ok(pset)
    }
}

/// `task_priority()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task, and the caller must hold no
/// locks.
pub(crate) unsafe fn priority(
    task: *mut Task,
    priority: c_int,
    change_threads: bool,
) -> Result<(), KernError> {
    if task.is_null() || invalid_pri(priority) {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock is held across the
    // priority write and the thread walk.
    unsafe {
        (*task).lock.lock();
        if (*task).max_priority > priority {
            (*task).lock.unlock();
            return Err(KernError::NoAccess);
        }
        (*task).priority = priority;

        let mut result = Ok(());
        if change_threads {
            let list = addr_of_mut!((*task).thread_list);
            let mut entry = queue_first(list);
            while queue_end(list, entry) == 0 {
                let thread = entry.cast::<Thread>();
                if Thread::priority(thread, priority, false).is_err() {
                    result = Err(KernError::Failure);
                }
                entry = queue_next(addr_of_mut!((*thread).thread_list));
            }
        }
        (*task).lock.unlock();
        result
    }
}

/// `task_set_name()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task.
pub(crate) unsafe fn set_name(
    task: *mut Task,
    name: &[u8],
) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the name array is one field
    // and is written as a whole here.
    unsafe {
        let dst = core::slice::from_raw_parts_mut(
            addr_of_mut!((*task).name).cast::<u8>(),
            TASK_NAME_SIZE,
        );
        dst.fill(0);
        let copy = name.len().min(TASK_NAME_SIZE - 1);
        dst[..copy].copy_from_slice(&name[..copy]);
    }
    Ok(())
}

/// `task_set_essential()` of kern/task.c.
///
/// # Safety
///
/// `task` must be null or point at a live task.
pub(crate) unsafe fn set_essential(
    task: *mut Task,
    essential: bool,
) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task.
    unsafe { (*task).set_essential(essential) };
    Ok(())
}

/// `task_collect_scan()` of kern/task.c: walk every processor set's tasks
/// and let the machine layer release what it can.
///
/// # Safety
///
/// `kern/task.c`'s collector calls this with nothing locked, as the C did.
unsafe fn collect_scan() {
    let mut prev_task: *mut Task = null_mut();
    let mut prev_pset: *mut ProcessorSet = null_mut();

    // SAFETY: `all_psets` and its lock are the C globals; the walk keeps a
    // reference on both the set and the task between iterations.
    unsafe {
        let all_psets = addr_of_mut!(glue::all_psets);
        let all_psets_lock = addr_of_mut!(glue::all_psets_lock);

        (*all_psets_lock).lock();
        let mut pset_entry = queue_first(all_psets);
        while queue_end(all_psets, pset_entry) == 0 {
            let pset = pset_entry.cast::<ProcessorSet>();
            (*pset).lock.lock();

            let tasks = addr_of_mut!((*pset).tasks);
            let mut task_entry = queue_first(tasks);
            while queue_end(tasks, task_entry) == 0 {
                let task = task_entry.cast::<Task>();
                reference(task);
                pset_reference(pset);
                (*pset).lock.unlock();
                (*all_psets_lock).unlock();

                glue::machine_task_collect(task);
                glue::pmap_collect((*(*task).map.cast::<VmMap>()).pmap);

                if !prev_task.is_null() {
                    deallocate(prev_task);
                }
                prev_task = task;

                if !prev_pset.is_null() {
                    pset_deallocate(prev_pset);
                }
                prev_pset = pset;

                (*all_psets_lock).lock();
                (*pset).lock.lock();
                task_entry = queue_next(addr_of_mut!((*task).pset_tasks));
            }
            (*pset).lock.unlock();
            pset_entry = queue_next(addr_of_mut!((*pset).all_psets));
        }
        (*all_psets_lock).unlock();

        if !prev_task.is_null() {
            deallocate(prev_task);
        }
        if !prev_pset.is_null() {
            pset_deallocate(prev_pset);
        }
    }
}

/// `consider_task_collect()` of kern/task.c.
///
/// # Safety
///
/// The pageout daemon calls this with nothing locked, as the C did.
pub(crate) unsafe fn consider_collect() {
    // The C's `hz / 1` and the usual arithmetic conversions reinterpret the
    // signed tick rate as unsigned; `hz` is positive and set before the
    // pageout daemon can run.
    let hz = unsafe { glue::hz } as c_uint;
    let mut max_rate = unsafe { TASK_COLLECT_MAX_RATE };
    if max_rate == 0 {
        max_rate = hz;
        // SAFETY: this is the collector's own state, and it runs on one
        // thread.
        unsafe { TASK_COLLECT_MAX_RATE = max_rate };
    }

    let last_tick = unsafe { TASK_COLLECT_LAST_TICK };
    let deadline = last_tick.wrapping_add(max_rate / hz);
    if unsafe { TASK_COLLECT_ALLOWED } != 0
        && unsafe { glue::sched_tick } > deadline
    {
        // SAFETY: as above.
        unsafe {
            TASK_COLLECT_LAST_TICK = glue::sched_tick;
            collect_scan();
        }
    }
}

/// `thread_override_max_priority()` of kern/task.c.
///
/// # Safety
///
/// `thread` must point at a live thread, and the caller must hold the task
/// lock the C held.
unsafe fn override_max_priority(
    thread: *mut Thread,
    max_priority: c_int,
    set_priority: bool,
) {
    // SAFETY: `splsched()` is the real asm routine of <machine/spl.h>; the
    // thread lock is taken under it.
    let s = unsafe { glue::splsched() };
    // SAFETY: the caller promises the live thread; the lock protects every
    // field read and written below.
    unsafe {
        (*thread).lock.lock();

        (*thread).max_priority = max_priority;
        let pset = (*thread).processor_set;
        if (*pset).max_priority > max_priority {
            (*thread).max_priority = (*pset).max_priority;
        }
        if (*thread).max_priority > (*thread).priority || set_priority {
            (*thread).priority = (*thread).max_priority;
        }

        compute_priority(thread, c_int::from(true));

        (*thread).lock.unlock();
        glue::splx(s);
    }
}

/// `extract_host_type()` of kern/task.c: the kobject type of `port` when it
/// is a host or host-privilege port, `IKOT_NONE` otherwise.
///
/// # Safety
///
/// `port` must be null, `IP_DEAD` or a live port.
unsafe fn extract_host_type(port: *mut c_void) -> c_uint {
    if port.is_null() || port.addr() == IP_DEAD {
        return IKOT_NONE;
    }

    // SAFETY: the checks above are `IP_VALID()`'s, so the pointer is live.
    let port = unsafe { IpcPort::from_raw(port) };
    // SAFETY: the port is live; the lock is held over the fields the C read.
    let ikot = unsafe {
        port.lock();
        let ikot = if port.is_active() {
            port.kotype()
        } else {
            IKOT_NONE
        };
        port.unlock();
        ikot
    };

    if ikot == IKOT_HOST || ikot == IKOT_HOST_PRIV {
        ikot
    } else {
        IKOT_NONE
    }
}

/// `task_max_priority()` of kern/task.c.
///
/// # Safety
///
/// `host` must be the port MIG converted from the request, `task` must be
/// null or point at a live task, and the caller must hold no locks.
pub(crate) unsafe fn max_priority(
    host: *mut c_void,
    task: *mut Task,
    max_priority: c_int,
    set_priority: bool,
    change_threads: bool,
) -> Result<(), KernError> {
    // SAFETY: the caller promises the port.
    let ikot_host = unsafe { extract_host_type(host) };

    if ikot_host == IKOT_NONE || task.is_null() || invalid_pri(max_priority) {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock is held across the
    // priority write and the thread walk.
    unsafe {
        (*task).lock.lock();

        if max_priority < (*task).max_priority && ikot_host != IKOT_HOST_PRIV {
            (*task).lock.unlock();
            return Err(KernError::NoAccess);
        }

        (*task).max_priority = max_priority;
        if max_priority > (*task).priority || set_priority {
            (*task).priority = max_priority;
        }

        if change_threads {
            let list = addr_of_mut!((*task).thread_list);
            let mut entry = queue_first(list);
            while queue_end(list, entry) == 0 {
                let thread = entry.cast::<Thread>();
                override_max_priority(thread, max_priority, set_priority);
                entry = queue_next(addr_of_mut!((*thread).thread_list));
            }
        }

        (*task).lock.unlock();
    }

    Ok(())
}
