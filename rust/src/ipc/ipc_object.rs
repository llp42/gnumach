// SPDX-License-Identifier: CMU-Mach
// Derived from ipc/ipc_object.c:
//   Copyright (c) 1991,1990,1989 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The send-to-received type-name conversion, which `ipc/ipc_object.c`
//! used to define and `ipc/ipc_object.h` declares.
//!
//! Only [`ipc_object_copyin_type()`] has moved; the rest of
//! `ipc/ipc_object.c` stays C until its slab and rights dependencies
//! have Rust homes.

use crate::glue;
use core::ffi::{c_int, c_uint};

/// A `mach_msg_type_name_t` the C `switch` accepted: the type names a
/// message can carry for a port right, plus the bare zero.
///
/// The discriminants are the `MACH_MSG_TYPE_*` values of
/// <mach/message.h>, where the received names are aliases of the moved
/// ones.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MsgTypeName {
    /// The bare `0` case: no right is named.
    Null = 0,
    /// `MACH_MSG_TYPE_MOVE_RECEIVE`: the sender held receive rights.
    MoveReceive = 16,
    /// `MACH_MSG_TYPE_MOVE_SEND`: the sender held send rights.
    MoveSend = 17,
    /// `MACH_MSG_TYPE_MOVE_SEND_ONCE`: the sender held send-once
    /// rights.
    MoveSendOnce = 18,
    /// `MACH_MSG_TYPE_COPY_SEND`: a copy of a send right.
    CopySend = 19,
    /// `MACH_MSG_TYPE_MAKE_SEND`: a new send right made from receive
    /// rights.
    MakeSend = 20,
    /// `MACH_MSG_TYPE_MAKE_SEND_ONCE`: a new send-once right made
    /// from receive rights.
    MakeSendOnce = 21,
}

impl MsgTypeName {
    /// The name a wire `mach_msg_type_name_t` spells, or `None` for a
    /// name the C `switch` would not have accepted.
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

    /// The form the receiver ends up holding.  The C's four return
    /// cases, with the send names collapsing into one received name.
    const fn received(self) -> Self {
        match self {
            Self::Null => Self::Null,
            Self::MoveReceive => Self::MoveReceive,
            Self::MoveSendOnce | Self::MakeSendOnce => Self::MoveSendOnce,
            Self::MoveSend | Self::MakeSend | Self::CopySend => Self::MoveSend,
        }
    }
}

/// Converts a send type name to a received type name.
/// `ipc_object_copyin_type()` in C.
///
/// # Panics
///
/// Halts through [`glue::Panic`] when `msgt_name` is not one of the
/// names the C accepted, which the C `panic()`ed on.
#[unsafe(no_mangle)]
pub extern "C" fn ipc_object_copyin_type(msgt_name: c_uint) -> c_uint {
    match MsgTypeName::from_u32(msgt_name) {
        Some(name) => name.received() as c_uint,
        None => {
            // SAFETY: `Panic` does not return; the file, function and
            // message tags are the C `panic()` macro's, and the line
            // is this Rust file's.
            unsafe {
                glue::Panic(
                    c"ipc/ipc_object.c".as_ptr(),
                    // Only `c_int` widths can reach `Panic`'s varargs.
                    line!() as c_int,
                    c"ipc_object_copyin_type".as_ptr(),
                    c"ipc_object_copyin_type: strange rights".as_ptr(),
                )
            }
        }
    }
}
