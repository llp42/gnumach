// SPDX-License-Identifier: CMU-Mach
// Derived from vm/vm_init.c:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The virtual memory bootstrap, which `vm/vm_init.c` used to define
//! and `vm/vm_init.h` declares.
//!
//! The module owns one thing: the order in which the VM packages come
//! up.  `bootstrap` runs on the first CPU, before kernel memory is
//! virtual, and drives the resident-page table, the slab allocator,
//! the object and map modules, `kmem`, `pmap`, `kalloc` and the fault
//! machinery.  `init` runs later, with the scheduler alive, and brings
//! up the pager protocol, the proxy port and the default memory
//! manager.  The order is load-bearing; it is the C's, unchanged.
//!
//! The two entry points keep the C symbols and signatures behind the
//! adapters at the bottom of the file, and `kern/startup.c` is their
//! only caller.  The VM map module is Rust, so its step calls
//! [`VmMap::init_module`] directly rather than the C-shaped
//! `vm_map_init` adapter.

use crate::arch::types::VmOffset;
use crate::glue::{
    kalloc_init, kmem_init, memory_manager_default_init,
    memory_object_proxy_init, pmap_init, slab_bootstrap, slab_init,
    vm_fault_init, vm_object_bootstrap, vm_object_init, vm_page_bootstrap,
    vm_page_info_all, vm_page_module_init,
};
use crate::vm::vm_map::VmMap;

/// Bring up the VM system in the order the packages depend on.
/// `vm_mem_bootstrap()` in C.
fn bootstrap() {
    let mut start: VmOffset = 0;
    let mut end: VmOffset = 0;

    // SAFETY: `start` and `end` are live locals, and the boot caller
    // runs this once, before any other VM package reads the physical
    // range the C reports.
    unsafe { vm_page_bootstrap(&mut start, &mut end) };

    // SAFETY: the boot caller runs the sequence once and in this
    // order, and each callee requires the packages before it to be up.
    unsafe {
        slab_bootstrap();
        vm_object_bootstrap();
        VmMap::init_module();
        kmem_init(start, end);
        pmap_init();
        slab_init();
        kalloc_init();
        vm_fault_init();
        vm_page_module_init();
        memory_manager_default_init();
    }
}

/// Bring up the VM parts that wait for the scheduler.
/// `vm_mem_init()` in C.
fn init() {
    // SAFETY: the boot caller runs this after `bootstrap`, once, when
    // the scheduler is alive; each callee requires the state it left.
    unsafe {
        vm_object_init();
        memory_object_proxy_init();
        vm_page_info_all();
    }
}

//
// The C edge: one adapter per symbol `vm/vm_init.h` declares.
//

/// Bootstrap the virtual memory system.  `vm_mem_bootstrap()` in C.
///
/// # Safety
///
/// Must be called exactly once, by the first CPU to come up, before
/// any other VM package is used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_mem_bootstrap() {
    bootstrap();
}

/// Initialize the VM parts that run with the scheduler alive.
/// `vm_mem_init()` in C.
///
/// # Safety
///
/// Must be called once, after `vm_mem_bootstrap()`, when the
/// scheduler is running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vm_mem_init() {
    init();
}
