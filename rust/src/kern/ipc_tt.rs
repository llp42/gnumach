// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_tt.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The task- and thread-related IPC operations, which `kern/ipc_tt.c` used to
//! define and `kern/ipc_tt.h` declares.

use crate::arch::i386::percpu::current_thread;
use crate::arch::types::VmOffset;
use crate::glue;
use crate::ipc::ipc_thread::ipc_thread_links_init;
use crate::ipc::{IpcPort, IpcSpace};
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::{self, TASK_PORT_REGISTER_MAX, Task, current_task};
use crate::kern::thread::{IpcKmsgQueue, Thread};
use crate::kern::types::KernError;
use crate::vm::vm_map::VmMap;
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr::{self, NonNull, with_exposed_provenance_mut};

/// `IKOT_THREAD` and `IKOT_TASK` of <kern/ipc_kobject.h>.
const IKOT_THREAD: c_uint = 1;
const IKOT_TASK: c_uint = 2;
/// `IKOT_NONE` of <kern/ipc_kobject.h>: the type of a port bound to no kernel
/// object.
const IKOT_NONE: c_uint = 0;
/// `IKO_NULL` of <kern/ipc_kobject.h>: the value that clears a port's
/// `ip_kobject`.
const IKO_NULL: VmOffset = 0;

/// `TASK_KERNEL_PORT`, `TASK_EXCEPTION_PORT` and `TASK_BOOTSTRAP_PORT` of
/// <mach/task_special_ports.h>: the `which` values `task_*_special_port()`
/// accepts.
const TASK_KERNEL_PORT: c_int = 1;
const TASK_EXCEPTION_PORT: c_int = 3;
const TASK_BOOTSTRAP_PORT: c_int = 4;

/// `THREAD_KERNEL_PORT` and `THREAD_EXCEPTION_PORT` of
/// <mach/thread_special_ports.h>.
const THREAD_KERNEL_PORT: c_int = 1;
const THREAD_EXCEPTION_PORT: c_int = 3;

/// `MACH_PORT_NULL` of <mach/port.h>.
const MACH_PORT_NULL: c_uint = 0;

/// The `which` domain of `task_get_special_port()` and
/// `task_set_special_port()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskSpecialPort {
    /// `TASK_KERNEL_PORT`.
    Kernel,
    /// `TASK_EXCEPTION_PORT`.
    Exception,
    /// `TASK_BOOTSTRAP_PORT`.
    Bootstrap,
}

impl TaskSpecialPort {
    /// The port a C `which` names, or `None` for anything else.
    pub(crate) const fn from_int(which: c_int) -> Option<Self> {
        match which {
            TASK_KERNEL_PORT => Some(Self::Kernel),
            TASK_EXCEPTION_PORT => Some(Self::Exception),
            TASK_BOOTSTRAP_PORT => Some(Self::Bootstrap),
            _ => None,
        }
    }
}

/// The `which` domain of `thread_get_special_port()` and
/// `thread_set_special_port()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadSpecialPort {
    /// `THREAD_KERNEL_PORT`.
    Kernel,
    /// `THREAD_EXCEPTION_PORT`.
    Exception,
}

impl ThreadSpecialPort {
    /// The port a C `which` names, or `None` for anything else.
    pub(crate) const fn from_int(which: c_int) -> Option<Self> {
        match which {
            THREAD_KERNEL_PORT => Some(Self::Kernel),
            THREAD_EXCEPTION_PORT => Some(Self::Exception),
            _ => None,
        }
    }
}

/// `ipc_space_create()` of <ipc/ipc_space.h>.
fn create_space() -> Result<*mut c_void, KernError> {
    let mut space: *mut c_void = ptr::null_mut();

    // SAFETY: the out-pointer is this function's live local, written only on
    // success.
    let code = unsafe { glue::ipc_space_create(&mut space) };

    match u8::try_from(code) {
        Ok(code) => KernError::from_u8(code).map(|()| space),
        Err(_) => Err(KernError::Failure),
    }
}

/// The C `panic()` of `ipc_task_init()` and `ipc_thread_init()`.
fn init_panic(fun: &'static CStr) -> ! {
    // SAFETY: `Panic` does not return; the file is the C file the call sat
    // in, and the message is the C's own function-name tag.
    unsafe {
        glue::Panic(
            c"kern/ipc_tt.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            fun.as_ptr(),
            fun.as_ptr(),
        )
    }
}

/// The `if (IP_VALID(port)) ipc_port_release_send(port);` the C repeats.
///
/// # Safety
///
/// A non-null, non-dead `port` must hold one send right.
unsafe fn release_send_if_valid(port: *mut c_void) {
    if let Some(port) = IpcPort::valid(port) {
        // SAFETY: the caller's contract; `valid()` is `IP_VALID()`'s test
        // and the right is the one `port` holds.
        unsafe { glue::ipc_port_release_send(port.as_ptr()) };
    }
}

/// `ipc_task_init()` in C.
///
/// # Safety
///
/// `task` must be a fresh task whose IPC fields this call is the first to
/// write, and `parent` must be null or a live task whose own
/// `ipc_task_init()` already ran; the caller must hold no locks, as the
/// allocation may block.
pub(crate) unsafe fn ipc_task_init(task: *mut Task, parent: *mut Task) {
    // SAFETY: the caller promises the fresh task, so nothing reads the space
    // while `ipc_space_create` builds it.
    let Ok(space) = create_space() else {
        init_panic(c"ipc_task_init")
    };

    // SAFETY: `ipc_space_kernel` is live for the life of the kernel; this is
    // the C's `ipc_port_alloc_kernel()`.
    let Some(kport) = IpcPort::new(unsafe {
        glue::ipc_port_alloc_special(glue::ipc_space_kernel)
    }) else {
        init_panic(c"ipc_task_init")
    };

    // SAFETY: the caller promises a fresh task, and the port was just
    // created live; the parent's lock covers the fields inherited below.
    unsafe {
        (*task).itk_lock_data.init();
        (*task).itk_self = kport.as_ptr();
        (*task).itk_sself = glue::ipc_port_make_send(kport.as_ptr());
        (*task).itk_space = space;

        if parent.is_null() {
            (*task).itk_exception = ptr::null_mut();
            (*task).itk_bootstrap = ptr::null_mut();
            (*task).itk_registered = [ptr::null_mut(); TASK_PORT_REGISTER_MAX];
        } else {
            (*parent).itk_lock_data.lock();
            for (slot, parent_port) in (*task)
                .itk_registered
                .iter_mut()
                .zip((*parent).itk_registered.iter())
            {
                *slot = glue::ipc_port_copy_send(*parent_port);
            }
            (*task).itk_exception =
                glue::ipc_port_copy_send((*parent).itk_exception);
            (*task).itk_bootstrap =
                glue::ipc_port_copy_send((*parent).itk_bootstrap);
            (*parent).itk_lock_data.unlock();
        }
    }
}

/// `ipc_task_enable()` in C.
///
/// # Safety
///
/// `task` must point at a live task whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
pub(crate) unsafe fn ipc_task_enable(task: *mut Task) {
    // SAFETY: the caller's contract; the task lock covers `itk_self`, and
    // `ipc_kobject_set` takes the port lock itself.
    unsafe {
        (*task).itk_lock_data.lock();
        let kport = (*task).itk_self;
        if !kport.is_null() {
            glue::ipc_kobject_set(kport, task.addr(), IKOT_TASK);
        }
        (*task).itk_lock_data.unlock();
    }
}

/// `ipc_task_disable()` in C.
///
/// # Safety
///
/// `task` must point at a live task whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
pub(crate) unsafe fn ipc_task_disable(task: *mut Task) {
    // SAFETY: the caller's contract; as [`ipc_task_enable()`], with the
    // clearing values the C passed.
    unsafe {
        (*task).itk_lock_data.lock();
        let kport = (*task).itk_self;
        if !kport.is_null() {
            glue::ipc_kobject_set(kport, IKO_NULL, IKOT_NONE);
        }
        (*task).itk_lock_data.unlock();
    }
}

/// `ipc_task_terminate()` in C.
///
/// # Safety
///
/// `task` must be a live, suspended task, or the current thread's own task,
/// and the caller must hold no locks.
pub(crate) unsafe fn ipc_task_terminate(task: *mut Task) {
    // SAFETY: the caller's contract; the lock detaches `itk_self` so a second
    // teardown returns without touching the port.
    let kport = unsafe {
        (*task).itk_lock_data.lock();
        let kport = (*task).itk_self;
        if kport.is_null() {
            (*task).itk_lock_data.unlock();
            return;
        }
        (*task).itk_self = ptr::null_mut();
        (*task).itk_lock_data.unlock();
        kport
    };

    // SAFETY: the caller promises a live task in teardown: the naked send
    // rights are the ones it holds, its space is live, and `kport` is the
    // receive right the lock above detached.
    unsafe {
        release_send_if_valid((*task).itk_sself);
        release_send_if_valid((*task).itk_exception);
        release_send_if_valid((*task).itk_bootstrap);
        for port in (*task).itk_registered.iter() {
            release_send_if_valid(*port);
        }
        glue::ipc_space_destroy((*task).itk_space);
        glue::ipc_port_dealloc_special(kport, glue::ipc_space_kernel);
    }
}

/// `ipc_thread_init()` in C.
///
/// # Safety
///
/// `thread` must be a fresh thread whose IPC fields this call is the first to
/// write, and the caller must hold no locks: the allocation may block.
pub(crate) unsafe fn ipc_thread_init(thread: *mut Thread) {
    // SAFETY: `ipc_space_kernel` is live for the life of the kernel; this is
    // the C's `ipc_port_alloc_kernel()`.
    let Some(kport) = IpcPort::new(unsafe {
        glue::ipc_port_alloc_special(glue::ipc_space_kernel)
    }) else {
        init_panic(c"ipc_thread_init")
    };

    // SAFETY: the caller promises a fresh thread, and the port was just
    // created live.
    unsafe {
        ipc_thread_links_init(thread.cast());
        (*thread).ith_messages = IpcKmsgQueue {
            base: ptr::null_mut(),
        };
        (*thread).ith_lock_data.init();
        (*thread).ith_self = kport.as_ptr();
        (*thread).ith_sself = glue::ipc_port_make_send(kport.as_ptr());
        (*thread).ith_exception = ptr::null_mut();
        (*thread).ith_mig_reply = MACH_PORT_NULL;
        (*thread).ith_rpc_reply = ptr::null_mut();
    }
}

/// `ipc_thread_enable()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread whose IPC state is initialized and
/// not terminated, and the caller must hold no locks.
pub(crate) unsafe fn ipc_thread_enable(thread: *mut Thread) {
    // SAFETY: the caller's contract; the thread lock covers `ith_self`, and
    // `ipc_kobject_set` takes the port lock itself.
    unsafe {
        (*thread).ith_lock_data.lock();
        let kport = (*thread).ith_self;
        if !kport.is_null() {
            glue::ipc_kobject_set(kport, thread.addr(), IKOT_THREAD);
        }
        (*thread).ith_lock_data.unlock();
    }
}

/// `ipc_thread_disable()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread whose IPC state is initialized and
/// not terminated, and the caller must hold no locks.
pub(crate) unsafe fn ipc_thread_disable(thread: *mut Thread) {
    // SAFETY: the caller's contract; as [`ipc_thread_enable()`], with the
    // clearing values the C passed.
    unsafe {
        (*thread).ith_lock_data.lock();
        let kport = (*thread).ith_self;
        if !kport.is_null() {
            glue::ipc_kobject_set(kport, IKO_NULL, IKOT_NONE);
        }
        (*thread).ith_lock_data.unlock();
    }
}

/// `ipc_thread_terminate()` in C.
///
/// # Safety
///
/// `thread` must be a live, suspended thread, or the current thread, and the
/// caller must hold no locks.
pub(crate) unsafe fn ipc_thread_terminate(thread: *mut Thread) {
    // SAFETY: the caller's contract; the lock detaches `ith_self` so a second
    // teardown returns without touching the port.
    let kport = unsafe {
        (*thread).ith_lock_data.lock();
        let kport = (*thread).ith_self;
        if kport.is_null() {
            (*thread).ith_lock_data.unlock();
            return;
        }
        (*thread).ith_self = ptr::null_mut();
        (*thread).ith_lock_data.unlock();
        kport
    };

    // SAFETY: the caller promises a live thread in teardown: the naked send
    // rights are the ones it holds, and `kport` is the receive right the
    // lock above detached.
    unsafe {
        release_send_if_valid((*thread).ith_sself);
        release_send_if_valid((*thread).ith_exception);
        glue::ipc_port_dealloc_special(kport, glue::ipc_space_kernel);
    }
}

/// `retrieve_task_self_fast()` in C.
///
/// # Safety
///
/// `task` must be a live task, and the caller must hold no locks.
pub(crate) unsafe fn retrieve_task_self_fast(
    task: *mut Task,
) -> Option<IpcPort> {
    // SAFETY: the caller's contract; the IPC lock covers both fields, and the
    // port lock covers the count bumped on the no-interposing path.
    unsafe {
        (*task).itk_lock_data.lock();

        let sself = (*task).itk_sself;
        let port = if ptr::eq(sself, (*task).itk_self) {
            // No interposing: the two fields name the same live port.
            let Some(port) = IpcPort::valid(sself) else {
                (*task).itk_lock_data.unlock();
                return None;
            };
            port.lock();
            port.increment_references();
            port.increment_srights();
            port.unlock();
            Some(port)
        } else {
            IpcPort::new(glue::ipc_port_copy_send(sself))
        };

        (*task).itk_lock_data.unlock();
        port
    }
}

/// `retrieve_thread_self_fast()` in C.
///
/// # Safety
///
/// `thread` must be a live thread, and the caller must hold no locks.
pub(crate) unsafe fn retrieve_thread_self_fast(
    thread: *mut Thread,
) -> Option<IpcPort> {
    // SAFETY: the caller's contract; the IPC lock covers both fields, and the
    // port lock covers the count bumped on the no-interposing path.
    unsafe {
        (*thread).ith_lock_data.lock();

        let sself = (*thread).ith_sself;
        let port = if ptr::eq(sself, (*thread).ith_self) {
            // No interposing: the two fields name the same live port.
            let Some(port) = IpcPort::valid(sself) else {
                (*thread).ith_lock_data.unlock();
                return None;
            };
            port.lock();
            port.increment_references();
            port.increment_srights();
            port.unlock();
            Some(port)
        } else {
            IpcPort::new(glue::ipc_port_copy_send(sself))
        };

        (*thread).ith_lock_data.unlock();
        port
    }
}

/// `mach_task_self()` in C, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context: the current task and its space are
/// live, and nothing may be locked.
pub(crate) unsafe fn mach_task_self() -> c_uint {
    // SAFETY: the caller's contract.
    let task = unsafe { current_task() };
    // SAFETY: as above; the task is live.
    let sright = unsafe { retrieve_task_self_fast(task) };

    // SAFETY: the current task's space is live, and `ipc_port_copyout_send`
    // handles a null or dead send right itself.
    unsafe {
        glue::ipc_port_copyout_send(
            sright.map_or(ptr::null_mut(), IpcPort::as_ptr),
            (*task).itk_space,
        )
    }
}

/// `mach_thread_self()` in C, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context: the current thread, its task and
/// that task's space are live, and nothing may be locked.
pub(crate) unsafe fn mach_thread_self() -> c_uint {
    let thread = current_thread();
    // SAFETY: the caller's contract; the current thread's task is live.
    let task = unsafe { (*thread).task };
    // SAFETY: as above; the current thread is live.
    let sright = unsafe { retrieve_thread_self_fast(thread) };

    // SAFETY: the task's space is live, and `ipc_port_copyout_send` handles
    // a null or dead send right itself.
    unsafe {
        glue::ipc_port_copyout_send(
            sright.map_or(ptr::null_mut(), IpcPort::as_ptr),
            (*task).itk_space,
        )
    }
}

/// `mach_reply_port()` in C, the mach trap.
///
/// # Safety
///
/// Must be called from a thread context: the current task and its space are
/// live, and nothing may be locked.
pub(crate) unsafe fn mach_reply_port() -> c_uint {
    // SAFETY: the caller's contract.
    let task = unsafe { current_task() };
    // SAFETY: as above; the task's space is live.
    let Some(space) = IpcSpace::new(unsafe { (*task).itk_space }) else {
        return MACH_PORT_NULL;
    };

    // SAFETY: the space is live, as `ipc_port_alloc` needs.
    match crate::ipc::ipc_port::alloc(space) {
        Ok((name, port)) => {
            // SAFETY: `ipc_port_alloc` returns the port live and locked, and
            // the C's `ip_unlock` released it.
            unsafe { port.unlock() };
            name
        }
        Err(_) => MACH_PORT_NULL,
    }
}

/// The `whichp` the C `switch` selected.
///
/// # Safety
///
/// `task` must be live and its IPC lock held.
unsafe fn task_port_field(
    task: *mut Task,
    which: TaskSpecialPort,
) -> *mut *mut c_void {
    // SAFETY: the caller promises a live task; each address is formed without
    // reading the field.
    unsafe {
        match which {
            TaskSpecialPort::Kernel => ptr::addr_of_mut!((*task).itk_sself),
            TaskSpecialPort::Exception => {
                ptr::addr_of_mut!((*task).itk_exception)
            }
            TaskSpecialPort::Bootstrap => {
                ptr::addr_of_mut!((*task).itk_bootstrap)
            }
        }
    }
}

/// The `whichp` the C `switch` selected.
///
/// # Safety
///
/// `thread` must be live and its IPC lock held.
unsafe fn thread_port_field(
    thread: *mut Thread,
    which: ThreadSpecialPort,
) -> *mut *mut c_void {
    // SAFETY: the caller promises a live thread; each address is formed
    // without reading the field.
    unsafe {
        match which {
            ThreadSpecialPort::Kernel => {
                ptr::addr_of_mut!((*thread).ith_sself)
            }
            ThreadSpecialPort::Exception => {
                ptr::addr_of_mut!((*thread).ith_exception)
            }
        }
    }
}

/// `task_get_special_port()` in C.
///
/// # Safety
///
/// `task` must be null or a live task, and the caller must hold no locks.
pub(crate) unsafe fn task_get_special_port(
    task: *mut Task,
    which: TaskSpecialPort,
) -> Result<Option<IpcPort>, KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task; the lock covers the field, and
    // `ipc_port_copy_send` handles a null or dead right itself.
    unsafe {
        (*task).itk_lock_data.lock();
        if (*task).itk_self.is_null() {
            (*task).itk_lock_data.unlock();
            return Err(KernError::Failure);
        }

        let port = IpcPort::new(glue::ipc_port_copy_send(*task_port_field(
            task, which,
        )));
        (*task).itk_lock_data.unlock();
        Ok(port)
    }
}

/// `task_set_special_port()` in C.
///
/// # Safety
///
/// `task` must be null or a live task, `port` must be a naked send right or
/// `IP_NULL`, and the caller must hold no locks; on success the right is
/// consumed.
pub(crate) unsafe fn task_set_special_port(
    task: *mut Task,
    which: TaskSpecialPort,
    port: *mut c_void,
) -> Result<(), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live task and the right; the lock covers
    // the field, and the release happens after the unlock as the C's did.
    unsafe {
        (*task).itk_lock_data.lock();
        if (*task).itk_self.is_null() {
            (*task).itk_lock_data.unlock();
            return Err(KernError::Failure);
        }

        let whichp = task_port_field(task, which);
        let old = *whichp;
        *whichp = port;
        (*task).itk_lock_data.unlock();

        release_send_if_valid(old);
    }
    Ok(())
}

/// `thread_get_special_port()` in C.
///
/// # Safety
///
/// `thread` must be null or a live thread, and the caller must hold no locks.
pub(crate) unsafe fn thread_get_special_port(
    thread: *mut Thread,
    which: ThreadSpecialPort,
) -> Result<Option<IpcPort>, KernError> {
    if thread.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live thread; the lock covers the field,
    // and `ipc_port_copy_send` handles a null or dead right itself.
    unsafe {
        (*thread).ith_lock_data.lock();
        if (*thread).ith_self.is_null() {
            (*thread).ith_lock_data.unlock();
            return Err(KernError::Failure);
        }

        let port = IpcPort::new(glue::ipc_port_copy_send(*thread_port_field(
            thread, which,
        )));
        (*thread).ith_lock_data.unlock();
        Ok(port)
    }
}

/// `thread_set_special_port()` in C.
///
/// # Safety
///
/// `thread` must be null or a live thread, `port` must be a naked send right
/// or `IP_NULL`, and the caller must hold no locks; on success the right is
/// consumed.
pub(crate) unsafe fn thread_set_special_port(
    thread: *mut Thread,
    which: ThreadSpecialPort,
    port: *mut c_void,
) -> Result<(), KernError> {
    if thread.is_null() {
        return Err(KernError::InvalidArgument);
    }

    // SAFETY: the caller promises a live thread and the right; the lock
    // covers the field, and the release happens after the unlock as the C's
    // did.
    unsafe {
        (*thread).ith_lock_data.lock();
        if (*thread).ith_self.is_null() {
            (*thread).ith_lock_data.unlock();
            return Err(KernError::Failure);
        }

        let whichp = thread_port_field(thread, which);
        let old = *whichp;
        *whichp = port;
        (*thread).ith_lock_data.unlock();

        release_send_if_valid(old);
    }
    Ok(())
}

/// `mach_ports_register()` in C.
///
/// # Safety
///
/// `task` must be null or a live task; `ports` holds naked send rights the
/// caller gives up on success, and the caller must hold no locks.
pub(crate) unsafe fn ports_register(
    task: *mut Task,
    ports: &[VmOffset],
) -> Result<(), KernError> {
    if task.is_null() || ports.len() > TASK_PORT_REGISTER_MAX {
        return Err(KernError::InvalidArgument);
    }

    let mut new_ports = [ptr::null_mut(); TASK_PORT_REGISTER_MAX];
    for (slot, port) in new_ports.iter_mut().zip(ports.iter()) {
        *slot = with_exposed_provenance_mut(*port);
    }

    // SAFETY: the caller promises a live task; the lock covers the register
    // array, and the swap hands the old rights back for release.
    unsafe {
        (*task).itk_lock_data.lock();
        if (*task).itk_self.is_null() {
            (*task).itk_lock_data.unlock();
            return Err(KernError::InvalidArgument);
        }

        for (slot, new) in
            (*task).itk_registered.iter_mut().zip(new_ports.iter_mut())
        {
            core::mem::swap(slot, new);
        }
        (*task).itk_lock_data.unlock();
    }

    // SAFETY: every valid entry is a naked send right the C released after
    // the unlock.
    unsafe {
        for port in new_ports {
            release_send_if_valid(port);
        }
    }
    Ok(())
}

/// `mach_ports_lookup()` in C.
///
/// # Safety
///
/// `task` must be null or a live task, and the caller must hold no locks: the
/// routine allocates.
pub(crate) unsafe fn ports_lookup(
    task: *mut Task,
) -> Result<(NonNull<VmOffset>, c_uint), KernError> {
    if task.is_null() {
        return Err(KernError::InvalidArgument);
    }

    let size = TASK_PORT_REGISTER_MAX * size_of::<VmOffset>();
    // SAFETY: `kalloc_init()` ran during the boot this kernel call follows.
    let Some(memory) = kalloc(size) else {
        return Err(KernError::ResourceShortage);
    };

    // SAFETY: the caller promises a live task; the lock covers the register
    // array, and the buffer is the live allocation made above.
    unsafe {
        (*task).itk_lock_data.lock();
        if (*task).itk_self.is_null() {
            (*task).itk_lock_data.unlock();
            kfree(memory, size);
            return Err(KernError::InvalidArgument);
        }

        let ports = memory.as_ptr().cast::<VmOffset>();
        for (i, port) in (*task).itk_registered.iter().enumerate() {
            let clone = glue::ipc_port_copy_send(*port);
            ptr::write(ports.add(i), clone.addr());
        }
        (*task).itk_lock_data.unlock();
    }

    // SAFETY: `memory` points at the live slots just filled.
    let ports = unsafe { NonNull::new_unchecked(memory.as_ptr().cast()) };
    // `TASK_PORT_REGISTER_MAX` is four, so the narrowing cannot lose a bit.
    Ok((ports, TASK_PORT_REGISTER_MAX as c_uint))
}

/// `convert_port_to_task()` in C.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port.
pub(crate) unsafe fn convert_port_to_task(
    port: *mut c_void,
) -> Option<NonNull<Task>> {
    let port = IpcPort::valid(port)?;

    // SAFETY: `valid()` established the live port; the lock covers the
    // kobject fields, and the reference is the one the C took.
    unsafe {
        port.lock();
        let task = match NonNull::new(port.kobject().cast::<Task>()) {
            Some(task) if port.is_active() && port.kotype() == IKOT_TASK => {
                task::reference(task.as_ptr());
                Some(task)
            }
            _ => None,
        };
        port.unlock();
        task
    }
}

/// `convert_port_to_space()` in C.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port.
pub(crate) unsafe fn convert_port_to_space(
    port: *mut c_void,
) -> Option<IpcSpace> {
    let port = IpcPort::valid(port)?;

    // SAFETY: `valid()` established the live port; the lock covers the
    // kobject fields, and the reference is the one the C took.
    unsafe {
        port.lock();
        let space = if port.is_active() && port.kotype() == IKOT_TASK {
            let task = port.kobject().cast::<Task>();
            match NonNull::new((*task).itk_space) {
                Some(space) => {
                    glue::ipc_space_reference(space.as_ptr());
                    IpcSpace::new(space.as_ptr())
                }
                None => None,
            }
        } else {
            None
        };
        port.unlock();
        space
    }
}

/// `convert_port_to_map()` in C.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port.
pub(crate) unsafe fn convert_port_to_map(
    port: *mut c_void,
) -> Option<NonNull<VmMap>> {
    let port = IpcPort::valid(port)?;

    // SAFETY: `valid()` established the live port; the lock covers the
    // kobject fields, and a live task's map is set.
    unsafe {
        port.lock();
        let map = if port.is_active() && port.kotype() == IKOT_TASK {
            let task = port.kobject().cast::<Task>();
            NonNull::new((*task).map.cast::<VmMap>())
        } else {
            None
        };
        if let Some(map) = map {
            VmMap::reference(map);
        }
        port.unlock();
        map
    }
}

/// `convert_port_to_thread()` in C.
///
/// # Safety
///
/// A non-null, non-dead `port` must point at a live port.
pub(crate) unsafe fn convert_port_to_thread(
    port: *mut c_void,
) -> Option<NonNull<Thread>> {
    let port = IpcPort::valid(port)?;

    // SAFETY: `valid()` established the live port; the lock covers the
    // kobject fields, and the reference is the one the C took.
    unsafe {
        port.lock();
        let thread = match NonNull::new(port.kobject().cast::<Thread>()) {
            Some(thread)
                if port.is_active() && port.kotype() == IKOT_THREAD =>
            {
                Thread::reference(thread.as_ptr());
                Some(thread)
            }
            _ => None,
        };
        port.unlock();
        thread
    }
}

/// `convert_task_to_port()` in C.
///
/// # Safety
///
/// `task` must be a live task the caller holds a reference to; the routine
/// consumes it and may deallocate the task.
pub(crate) unsafe fn convert_task_to_port(task: *mut Task) -> Option<IpcPort> {
    // SAFETY: the caller's contract; the lock covers `itk_self`, and the C's
    // null test is the one the branch makes.
    let port = unsafe {
        (*task).itk_lock_data.lock();
        let port = if (*task).itk_self.is_null() {
            None
        } else {
            IpcPort::new(glue::ipc_port_make_send((*task).itk_self))
        };
        (*task).itk_lock_data.unlock();
        port
    };

    // SAFETY: the caller's reference is the one the C consumed.
    unsafe { task::deallocate(task) };
    port
}

/// `convert_thread_to_port()` in C.
///
/// # Safety
///
/// `thread` must be a live thread the caller holds a reference to; the
/// routine consumes it and may deallocate the thread.
pub(crate) unsafe fn convert_thread_to_port(
    thread: *mut Thread,
) -> Option<IpcPort> {
    // SAFETY: the caller's contract; the lock covers `ith_self`, and the C's
    // null test is the one the branch makes.
    let port = unsafe {
        (*thread).ith_lock_data.lock();
        let port = if (*thread).ith_self.is_null() {
            None
        } else {
            IpcPort::new(glue::ipc_port_make_send((*thread).ith_self))
        };
        (*thread).ith_lock_data.unlock();
        port
    };

    // SAFETY: the caller's reference is the one the C consumed.
    unsafe { Thread::deallocate(thread) };
    port
}

/// `space_deallocate()` in C: the `is_release()` of a space ref a
/// [`convert_port_to_space()`] produced.
///
/// # Safety
///
/// A non-null `space` must be a live space the caller holds a reference to.
pub(crate) unsafe fn space_deallocate(space: *mut c_void) {
    // SAFETY: the caller's contract; `ipc_space_release` is the C's
    // `is_release()`.
    if let Some(space) = IpcSpace::new(space) {
        unsafe { glue::ipc_space_release(space.as_ptr()) };
    }
}
