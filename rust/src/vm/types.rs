// SPDX-License-Identifier: CMU-Mach
// Derived from include/mach/vm_prot.h, include/mach/vm_inherit.h and
// i386/include/mach/i386/vm_param.h, with the opaque handles of
// vm/pmap.h, vm/vm_object.h and vm/vm_page.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! VM scalar and handle types, from `mach/vm_prot.h`, `vm_inherit.h`
//! and the VM headers' opaque pointers.
//!
//! `vm_offset_t` and `vm_size_t` are `arch::types`', because they are
//! pointer-sized and the machine decides.

use crate::arch::types::VmSize;
use core::ffi::c_int;

/// `PAGE_SHIFT` of <machine/vm_param.h>: 12 on both x86 kernels.
pub(crate) const PAGE_SHIFT: u32 = 12;
/// `PAGE_SIZE`: one page.
pub(crate) const PAGE_SIZE: VmSize = 1 << PAGE_SHIFT;
/// `PAGE_MASK`: the in-page offset bits.
pub(crate) const PAGE_MASK: VmSize = PAGE_SIZE - 1;

/// `vm_prot_t` of <mach/vm_prot.h>: a set of bits.
///
/// `#[repr(transparent)]`, so it keeps the ABI of the `c_int` it
/// wraps.  This definition moved here from `kern/elf_load.rs`, which
/// uses it for `PT_GNU_STACK`; the VM map uses it for entry
/// protections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmProt(c_int);

impl VmProt {
    /// `VM_PROT_NONE`.
    pub const NONE: Self = Self(0x0);
    /// `VM_PROT_READ`.
    pub const READ: Self = Self(0x1);
    /// `VM_PROT_WRITE`.
    pub const WRITE: Self = Self(0x2);
    /// `VM_PROT_EXECUTE`.
    pub const EXECUTE: Self = Self(0x4);
    /// `VM_PROT_ALL`: read, write and execute.
    pub const ALL: Self = Self(Self::READ.0 | Self::WRITE.0 | Self::EXECUTE.0);
    /// `VM_PROT_NO_CHANGE`: a marker `vm_map_protect` refuses to set.
    pub const NO_CHANGE: Self = Self(0x08);
    /// `VM_PROT_NOTIFY`: a marker bit for callers of `vm_map_protect`.
    pub const NOTIFY: Self = Self(0x10);

    /// The `c_int` the C side passes and stores.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// A protection value from the C side.  Bits outside `ALL` are
    /// kept: `VM_PROT_NO_CHANGE` and `VM_PROT_NOTIFY` are defined on
    /// top of this type by their users.
    pub const fn from_bits(bits: c_int) -> Self {
        Self(bits)
    }

    /// Whether every bit of `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for VmProt {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for VmProt {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for VmProt {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl core::ops::BitAndAssign for VmProt {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// `vm_inherit_t` of <mach/vm_inherit.h>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct VmInherit(c_int);

impl VmInherit {
    /// `VM_INHERIT_SHARE`.
    pub const SHARE: Self = Self(0);
    /// `VM_INHERIT_COPY`.
    pub const COPY: Self = Self(1);
    /// `VM_INHERIT_NONE`.
    pub const NONE: Self = Self(2);

    /// The `c_int` the C side passes and stores.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// An inheritance value from the C side.
    pub const fn from_bits(bits: c_int) -> Self {
        Self(bits)
    }
}

/// `pmap_t`: the machine-dependent physical map of a VM map.
#[repr(C)]
pub struct Pmap {
    _private: [u8; 0],
}

/// `vm_object_t`: the memory object an entry maps.
#[repr(C)]
pub struct VmObject {
    _private: [u8; 0],
}

/// `vm_page_t`: a physical page in a page list.
#[repr(C)]
pub struct VmPage {
    _private: [u8; 0],
}
