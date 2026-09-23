// SPDX-License-Identifier: CMU-Mach
// Derived from kern/ipc_mig.c:
//   Copyright (c) 1991,1990 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The MIG support entry points of <mach/mig_support.h>, and the
//! kernel-side RPC stub, which `kern/ipc_mig.c` used to define.
//!
//! [`mig_strncpy`] is the bounded copy MIG emits for a string
//! argument.  [`mig_put_reply_port`] and [`mig_dealloc_reply_port`]
//! are the client-side reply-port hooks: a kernel call never prompts
//! MIG to call either, so one does nothing and the other halts.
//! [`mach_msg_rpc_from_kernel`] has never been implemented in this
//! kernel and halts too.
//!
//! The rest of `kern/ipc_mig.c` stays C.  It reaches into `struct
//! ipc_space`, `struct ipc_entry` and `struct ipc_kmsg` through lock
//! macros that inline the dereference into the caller, and none of
//! those has a Rust mirror yet.

use crate::arch::types::{VmOffset, VmSize};
use crate::glue;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::slice;

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
