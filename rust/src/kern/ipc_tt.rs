// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_tt.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The thread IPC enable and disable entries, which `kern/ipc_tt.c` used to
//! define and <kern/ipc_tt.h> declares.

use crate::arch::types::VmOffset;
use crate::glue;
use crate::kern::thread::Thread;
use core::ffi::c_uint;

/// `IKOT_THREAD` of <kern/ipc_kobject.h>: the object type of a thread's self
/// port.
const IKOT_THREAD: c_uint = 1;
/// `IKOT_NONE`: the type of a port bound to no kernel object.
const IKOT_NONE: c_uint = 0;
/// `IKO_NULL`: the value that clears a port's `ip_kobject`.
const IKO_NULL: VmOffset = 0;

/// `ipc_thread_enable()` in C.
///
/// # Safety
///
/// `thread` must point at a live thread whose IPC state `ipc_thread_init()`
/// built; nothing may be locked, as the C documented.
unsafe fn enable(thread: *mut Thread) {
    // SAFETY: the caller's contract; `ith_lock_data` is the lock the C held
    // over the `ith_self` read, and `ipc_kobject_set()` takes the port lock
    // itself.
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
/// `thread` must point at a live thread whose IPC state `ipc_thread_init()`
/// built; nothing may be locked, as the C documented.
unsafe fn disable(thread: *mut Thread) {
    // SAFETY: the caller's contract; as [`enable()`], with the clearing values
    // the C passed.
    unsafe {
        (*thread).ith_lock_data.lock();
        let kport = (*thread).ith_self;
        if !kport.is_null() {
            glue::ipc_kobject_set(kport, IKO_NULL, IKOT_NONE);
        }
        (*thread).ith_lock_data.unlock();
    }
}

/// `ipc_thread_enable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must point at a live thread whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_enable(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { enable(thread) };
}

/// `ipc_thread_disable()` of kern/ipc_tt.c.
///
/// # Safety
///
/// `thread` must point at a live thread whose IPC state is initialized and not
/// terminated, and the caller must hold no locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_thread_disable(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { disable(thread) };
}
