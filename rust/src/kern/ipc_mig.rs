// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_mig.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The MIG support entry points of <mach/mig_support.h> and the
//! kernel-side RPC stubs, which `kern/ipc_mig.c` used to define.
//!
//! [`mig_strncpy`] is the bounded copy MIG emits for a string
//! argument.  [`mig_put_reply_port`] and [`mig_dealloc_reply_port`]
//! are the client-side reply-port hooks: a kernel call never prompts
//! MIG to call either, so one does nothing and the other halts.
//! [`mach_msg_rpc_from_kernel`] has never been implemented in this
//! kernel and halts too.
//!
//! [`mach_msg_abort_rpc`] and [`mig_get_reply_port`] manage the reply
//! ports of a thread; [`mig_deallocate`] frees the map copy MIG handed
//! back; [`thread_set_self_state`] is the trap that writes the current
//! thread's machine state from a user buffer.
//!
//! The rest of `kern/ipc_mig.c` stays C: `mach_msg()` and the
//! `syscall_*` conversion entries reach into `struct ipc_space`,
//! `struct ipc_entry` and `struct ipc_kmsg` through lock macros that
//! inline the dereference into the caller, and none of those has a
//! Rust mirror yet.

use crate::arch::i386::percpu::current_thread;
use crate::arch::types::{VmOffset, VmSize};
use crate::glue;
use crate::kern::thread::Thread;
use crate::kern::types::KernError;
use crate::vm::vm_map::VmMapCopy;
use crate::vm::vm_map_ffi::vm_map_copy_discard;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr;
use core::slice;

/// `MACH_PORT_NULL` in <mach/port.h>: no port name.
const MACH_PORT_NULL: c_uint = 0;
/// `KERN_SUCCESS` in <mach/kern_return.h>.
const KERN_SUCCESS: c_int = 0;
/// The `natural_t` words the C scratch array of
/// [`thread_set_self_state`] held.
const MAX_SELF_STATE: usize = 150;

/// Copy `src` into `dest`, terminate what was written, and return the
/// length written without the terminator.
///
/// `dest` is the caller's whole buffer, so at most `dest.len() - 1`
/// bytes of `src` are copied and the byte after them is set to NUL.
/// An empty `dest` is left untouched and the answer is zero.
fn copy_terminated(dest: &mut [u8], src: &[u8]) -> usize {
    let copied = src.len().min(dest.len().saturating_sub(1));
    let (head, tail) = dest.split_at_mut(copied);
    head.copy_from_slice(&src[..copied]);

    // `tail` is empty only when `dest` is, because `copied` is at
    // most `dest.len() - 1`.
    if let Some(terminator) = tail.first_mut() {
        *terminator = 0;
    }

    copied
}

/// Copies the NUL-terminated `src` into the `len`-byte `dest`, always
/// terminating `dest`, and returns the length written without the
/// terminator.  `mig_strncpy()` in C.
///
/// The string is truncated when it does not fit, exactly as the C
/// loop did: the terminator then lands on `dest[len - 1]`.
///
/// # Safety
///
/// `dest` must be valid for writes of `len` bytes.  `src` must be
/// readable up to and including its first NUL byte, or for its first
/// `len - 1` bytes, whichever comes first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_strncpy(
    dest: *mut c_char,
    src: *const c_char,
    len: VmSize,
) -> VmSize {
    // The C returned before touching either buffer, and an empty
    // slice may not be built from a pointer that is only valid for
    // zero bytes.
    if len == 0 {
        return 0;
    }

    // The C loop read `src[0]` through `src[len - 2]` and stopped at
    // the first NUL, so the walk has the same bound.
    let bound = len - 1;
    let mut found = 0;
    while found < bound {
        // SAFETY: the caller promises `src` is readable to its first
        // NUL or for `bound` bytes, and the walk stops at whichever
        // comes first.
        if unsafe { *src.add(found) } == 0 {
            break;
        }
        found += 1;
    }

    // SAFETY: the walk proved the first `found` bytes of `src` are
    // readable, and they stay so for this call.
    let src = unsafe { slice::from_raw_parts(src.cast::<u8>(), found) };
    // SAFETY: the caller promises `len` writable bytes at `dest`, and
    // `src` above does not overlap it.
    let dest = unsafe { slice::from_raw_parts_mut(dest.cast::<u8>(), len) };

    copy_terminated(dest, src)
}

/// Lets the client recycle a reply port after an RPC.
/// `mig_put_reply_port()` in C.
///
/// The kernel keeps its reply port in the thread across calls, so
/// there is nothing to recycle and the C body was empty as well.
#[unsafe(no_mangle)]
pub extern "C" fn mig_put_reply_port(_reply_port: VmOffset) {}

/// Gets rid of a reply port.  `mig_dealloc_reply_port()` in C.
///
/// # Panics
///
/// Always halts through [`glue::Panic`].  This is a client-side
/// interface, and a kernel call never prompts MIG to call it.
#[unsafe(no_mangle)]
pub extern "C" fn mig_dealloc_reply_port(_reply_port: VmOffset) {
    // SAFETY: `Panic` does not return; the file, function and message
    // tags are the C `panic()` macro's, and the line is this Rust
    // file's.
    unsafe {
        glue::Panic(
            c"kern/ipc_mig.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            c"mig_dealloc_reply_port".as_ptr(),
            c"mig_dealloc_reply_port".as_ptr(),
        )
    }
}

/// Sends a message from the kernel and waits for the reply.
/// `mach_msg_rpc_from_kernel()` in C.
///
/// The message is a `const mach_msg_header_t *`, which nothing here
/// reads, so it stays an opaque address until `<mach/message.h>` has
/// a Rust mirror.
///
/// # Panics
///
/// Always halts through [`glue::Panic`]: this kernel has never
/// implemented the call, and the C body was the same one `panic()`.
#[unsafe(no_mangle)]
pub extern "C" fn mach_msg_rpc_from_kernel(
    _msg: *const c_void,
    _send_size: c_uint,
    _reply_size: c_uint,
) -> c_int {
    // SAFETY: `Panic` does not return; the file, function and message
    // tags are the C `panic()` macro's, and the line is this Rust
    // file's.
    unsafe {
        glue::Panic(
            c"kern/ipc_mig.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            c"mach_msg_rpc_from_kernel".as_ptr(),
            c"mach_msg_rpc_from_kernel".as_ptr(),
        )
    }
}

/// Destroy the kernel-RPC reply port `thread` holds, if it holds one.
///
/// # Safety
///
/// `thread` must point at a live thread, and nothing may be locked, as
/// the C documented.
unsafe fn abort_rpc(thread: *mut Thread) {
    // SAFETY: the caller's contract; `ith_lock_data` is the lock the C
    // held over the two port fields, and it protects them here too.
    let reply = unsafe {
        (*thread).ith_lock_data.lock();
        let reply = if (*thread).ith_self.is_null() {
            ptr::null_mut()
        } else {
            let reply = (*thread).ith_rpc_reply;
            (*thread).ith_rpc_reply = ptr::null_mut();
            reply
        };
        (*thread).ith_lock_data.unlock();
        reply
    };

    if !reply.is_null() {
        // SAFETY: `reply` was the thread's live reply port and the
        // thread no longer holds it, so this call owns the reference.
        // The C's `ipc_port_dealloc_reply()` is the
        // `ipc_port_dealloc_special(reply, ipc_space_reply)` macro,
        // and `ipc_space_reply` is the space `ipc_init()` built.
        unsafe {
            glue::ipc_port_dealloc_special(reply, glue::ipc_space_reply);
        }
    }
}

/// Destroy a thread's reply port, interrupting a pending kernel RPC
/// with `MACH_RCV_PORT_DIED`.  `mach_msg_abort_rpc()` of kern/ipc_mig.c.
///
/// # Safety
///
/// `thread` must point at a live thread, and nothing may be locked, as
/// the C documented.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mach_msg_abort_rpc(thread: *mut Thread) {
    // SAFETY: the caller's contract.
    unsafe { abort_rpc(thread) };
}

/// The reply port a kernel-side MIG call receives on, allocated on the
/// first call.  `mig_get_reply_port()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Only the current thread's own reply-port field is read and written,
/// which nothing else touches until the thread dies.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_get_reply_port() -> c_uint {
    // SAFETY: the current thread is live, and the C read and wrote its
    // `ith_mig_reply` field with no lock either.
    unsafe {
        let thread = current_thread();
        if (*thread).ith_mig_reply == MACH_PORT_NULL {
            (*thread).ith_mig_reply = glue::mach_reply_port();
        }
        (*thread).ith_mig_reply
    }
}

/// Free the kernel memory MIG deallocated, which here is a map copy.
/// `mig_deallocate()` of kern/ipc_mig.c.
///
/// The `size` argument is ignored, as the C's `(void) size` did.
///
/// # Safety
///
/// A non-null `addr` must be the address of a live map-copy object the
/// caller owns and no longer uses; null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mig_deallocate(addr: VmOffset, _size: VmSize) {
    // SAFETY: the caller promises a live map copy, which the Rust
    // `vm_map_copy_discard()` frees.  The C cast `addr` to
    // `vm_map_copy_t` the same way.
    unsafe { vm_map_copy_discard(addr as *mut VmMapCopy) };
}

/// Copy the user's state words into `scratch`, reporting how many were
/// copied.
///
/// `None` is the C's `KERN_INVALID_ARGUMENT`: a count outside
/// `1..=MAX_SELF_STATE`, or a `copyin()` fault.
///
/// # Safety
///
/// A `count` in range requires `new_state` to be readable for that many
/// `natural_t` words; a fault is what `copyin()` reports.
unsafe fn copy_self_state(
    new_state: *mut c_uint,
    count: c_uint,
    scratch: &mut [c_uint; MAX_SELF_STATE],
) -> Option<usize> {
    let words = usize::try_from(count).ok()?;
    if words == 0 || words > MAX_SELF_STATE {
        return None;
    }

    // SAFETY: the caller promises `new_state` readable for `count`
    // words, and `words` is in range, so the copy stays inside both
    // buffers.  `copyin()` reports a fault instead of raising one.
    let faulted = unsafe {
        glue::copyin(
            new_state.cast(),
            scratch.as_mut_ptr().cast(),
            words * size_of::<c_uint>(),
        )
    };
    if faulted != 0 { None } else { Some(words) }
}

/// Set the current thread's machine state from a user buffer.  The
/// generic state's success path does not return.
///
/// # Safety
///
/// Reached as trap -77 with a user `new_state`; a fault is what
/// `copyin()` reports.  No lock may be held, as the C documented.
unsafe fn set_self_state(
    flavor: c_int,
    new_state: *mut c_uint,
    count: c_uint,
) -> Result<(), KernError> {
    let mut scratch = [0; MAX_SELF_STATE];

    // SAFETY: the caller's contract, and `copy_self_state()` bounds the
    // copy to `scratch`.
    let Some(words) =
        (unsafe { copy_self_state(new_state, count, &mut scratch) })
    else {
        return Err(KernError::InvalidArgument);
    };

    let thread = current_thread();
    // SAFETY: `thread` is the running thread.  The C writes the syscall
    // return register first and then applies the state, as this does.
    unsafe {
        glue::thread_set_syscall_return(thread, KERN_SUCCESS);
        let code = glue::thread_setstatus(
            thread,
            flavor,
            scratch.as_mut_ptr(),
            // `words` is at most `MAX_SELF_STATE`, so the narrowing
            // cannot lose anything.
            words as c_uint,
        );
        let result = match u8::try_from(code) {
            Ok(code) => KernError::from_u8(code),
            Err(_) => Err(KernError::Failure),
        };
        if result.is_ok() {
            glue::thread_exception_return();
        }
        result
    }
}

/// Set the current thread's state, as if with a `thread_set_state()`
/// RPC.  `thread_set_self_state()` of kern/ipc_mig.c.
///
/// # Safety
///
/// Reached as trap -77: a non-null `new_state` must be readable for
/// `new_state_count` `natural_t` words, and the caller must hold no
/// locks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_set_self_state(
    flavor: c_int,
    new_state: *mut c_uint,
    new_state_count: c_uint,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { set_self_state(flavor, new_state, new_state_count) } {
        // The success path returns through `thread_exception_return()`,
        // so this arm cannot be reached.
        Ok(()) => KERN_SUCCESS,
        Err(error) => c_int::from(error),
    }
}
