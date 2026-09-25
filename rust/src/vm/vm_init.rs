// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_init.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The virtual memory bootstrap, which `vm/vm_init.c` used to define and
//! `vm/vm_init.h` declares.

use crate::arch::types::VmOffset;
use crate::glue::{
    memory_manager_default_init, pmap_init, vm_fault_init,
    vm_object_bootstrap, vm_object_init, vm_page_bootstrap, vm_page_info_all,
};
use crate::kern::slab::{kalloc_init, slab_bootstrap, slab_init};
use crate::vm::memory_object_proxy;
use crate::vm::vm_kern_ffi::kmem_init;
use crate::vm::vm_map::VmMap;
use crate::vm::vm_resident;

/// `vm_mem_bootstrap()` in C.
fn bootstrap() {
    let mut start: VmOffset = 0;
    let mut end: VmOffset = 0;

    // SAFETY: `start` and `end` are live locals, and the boot caller runs this
    // once, before any other VM package reads the physical range the C
    // reports.
    unsafe { vm_page_bootstrap(&mut start, &mut end) };

    // SAFETY: the boot caller runs the sequence once and in this order, and
    // each callee requires the packages before it to be up.
    unsafe {
        slab_bootstrap();
        vm_object_bootstrap();
        VmMap::init_module();
        kmem_init(start, end);
        pmap_init();
        slab_init();
        kalloc_init();
        vm_fault_init();
        vm_resident::module_init();
        memory_manager_default_init();
    }
}

/// `vm_mem_init()` in C.
fn init() {
    // SAFETY: the boot caller runs this after `bootstrap`, once, when the
    // scheduler is alive; each callee requires the state it left.
    unsafe {
        vm_object_init();
        vm_page_info_all();
    }
    memory_object_proxy::init();
}

/// `vm_mem_bootstrap()` in C.
///
/// # Safety
///
/// Must be called exactly once, by the first CPU to come up, before any other
/// VM package is used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_mem_bootstrap() {
    bootstrap();
}

/// `vm_mem_init()` in C.
///
/// # Safety
///
/// Must be called once, after `vm_mem_bootstrap()`, when the scheduler is
/// running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_mem_init() {
    init();
}
