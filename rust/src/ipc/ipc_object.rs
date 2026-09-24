// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_object.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The send-name conversion and the naked-capability destruction, which
//! `ipc/ipc_object.c` used to define and `ipc/ipc_object.h` declares.

use crate::glue;
use crate::ipc::IpcPort;
use crate::ipc::ipc_port;
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::ptr::NonNull;

/// A `mach_msg_type_name_t` the C `switch` accepted: the type names a message
/// can carry for a port right, plus the bare zero.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MsgTypeName {
    /// The bare `0` case: no right is named.
    Null = 0,
    /// `MACH_MSG_TYPE_MOVE_RECEIVE`: the sender held receive rights.
    MoveReceive = 16,
    /// `MACH_MSG_TYPE_MOVE_SEND`: the sender held send rights.
    MoveSend = 17,
    /// `MACH_MSG_TYPE_MOVE_SEND_ONCE`: the sender held send-once rights.
    MoveSendOnce = 18,
    /// `MACH_MSG_TYPE_COPY_SEND`: a copy of a send right.
    CopySend = 19,
    /// `MACH_MSG_TYPE_MAKE_SEND`: a new send right made from receive rights.
    MakeSend = 20,
    /// `MACH_MSG_TYPE_MAKE_SEND_ONCE`: a new send-once right made from receive
    /// rights.
    MakeSendOnce = 21,
}

impl MsgTypeName {
    /// The name a wire `mach_msg_type_name_t` spells, or `None` for a name the
    /// C `switch` would not have accepted.
    const fn from_u32(name: u32) -> Option<Self> {
        match name {
            0 => Some(Self::Null),
            16 => Some(Self::MoveReceive),
            17 => Some(Self::MoveSend),
            18 => Some(Self::MoveSendOnce),
            19 => Some(Self::CopySend),
            20 => Some(Self::MakeSend),
            21 => Some(Self::MakeSendOnce),
            _ => None,
        }
    }

    /// The form the receiver ends up holding.
    const fn received(self) -> Self {
        match self {
            Self::Null => Self::Null,
            Self::MoveReceive => Self::MoveReceive,
            Self::MoveSendOnce | Self::MakeSendOnce => Self::MoveSendOnce,
            Self::MoveSend | Self::MakeSend | Self::CopySend => Self::MoveSend,
        }
    }
}

/// `ipc_object_copyin_type()` in C.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when `msgt_name` is not one of the names the
/// C accepted, which the C `panic()`ed on.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_object_copyin_type(msgt_name: c_uint) -> c_uint {
    match MsgTypeName::from_u32(msgt_name) {
        Some(name) => name.received() as c_uint,
        None => strange_rights(
            c"ipc_object_copyin_type",
            c"ipc_object_copyin_type: strange rights",
        ),
    }
}

/// The C `default: panic()` arm of a rights switch.
fn strange_rights(fun: &'static CStr, message: &'static CStr) -> ! {
    // SAFETY: `Panic` does not return; the file is the one the switch belongs
    // to, the line is this Rust file's, and `fun` and `message` are the C's
    // own tags.
    unsafe {
        glue::Panic(
            c"ipc/ipc_object.c".as_ptr(),
            // Only `c_int` widths can reach `Panic`'s varargs.
            line!() as c_int,
            fun.as_ptr(),
            message.as_ptr(),
        )
    }
}

/// Destroys a naked capability, consuming one reference to the port.
fn destroy(port: IpcPort, name: MsgTypeName) {
    match name {
        MsgTypeName::MoveReceive => {
            // SAFETY: the caller owns the one reference the port holds, which
            // `ipc_port_release_receive` consumes.
            unsafe { ipc_port::release_receive(port) }
        }
        MsgTypeName::MoveSend => {
            // SAFETY: the caller owns the one reference the port holds, which
            // `ipc_port_release_send` consumes.
            unsafe { ipc_port::release_send(port) }
        }
        MsgTypeName::MoveSendOnce => {
            // SAFETY: the caller owns the one send-once right the port holds,
            // which `ipc_notify_send_once` consumes.
            unsafe { glue::ipc_notify_send_once(port.as_ptr()) }
        }
        MsgTypeName::Null
        | MsgTypeName::CopySend
        | MsgTypeName::MakeSend
        | MsgTypeName::MakeSendOnce => strange_rights(
            c"ipc_object_destroy",
            c"ipc_object_destroy: strange rights",
        ),
    }
}

/// `ipc_object_destroy()` in C.
///
/// # Safety
///
/// `object` must be a live `ipc_object` of the port kind, and the caller must
/// own the one reference the destruction consumes.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when `msgt_name` is not one of the three port
/// names, as the C `default` arm did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ipc_object_destroy(
    object: *mut c_void,
    msgt_name: c_uint,
) {
    // SAFETY: the caller promises a live object for the names the C switch
    // accepted; these three each dereference it.
    let port = IpcPort(unsafe { NonNull::new_unchecked(object) });

    match MsgTypeName::from_u32(msgt_name) {
        Some(name) => destroy(port, name),
        None => strange_rights(
            c"ipc_object_destroy",
            c"ipc_object_destroy: strange rights",
        ),
    }
}
