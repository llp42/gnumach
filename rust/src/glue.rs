// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The C functions Rust calls.
//!
//! C *macros* cannot come through here; when Rust needs one, it gets a
//! small C shim function beside the header that defines it, and that
//! shim is declared below like any other C function.

use crate::arch::types::{VmOffset, VmSize};
use crate::kern::lock::LockData;
use crate::vm::types::{Pmap, VmObject, VmPage};
use core::ffi::{c_char, c_int, c_short, c_uint, c_void};

// `panic()` in <kern/debug.h> is a macro over `Panic()`.
unsafe extern "C" {
    pub fn Panic(
        file: *const c_char,
        line: c_int,
        fun: *const c_char,
        s: *const c_char,
        ...
    ) -> !;

    // <kern/printf.h>
    pub fn printf(fmt: *const c_char, ...) -> c_int;

    // <kern/mach_clock.h>
    pub fn timeout(
        fcn: Option<unsafe extern "C" fn(*mut c_void)>,
        param: *mut c_void,
        interval: c_int,
    ) -> *mut c_void;

    // <kern/machine.c>
    pub fn cpu_shutdown();

    // <i386at/kd.h>, the screen block moves in kdasm.S
    pub fn kd_slmwd(start: *mut c_void, count: c_int, value: c_int);
    pub fn kd_slmscu(from: *mut c_void, to: *mut c_void, count: c_int);
    pub fn kd_slmscd(from: *mut c_void, to: *mut c_void, count: c_int);

    // <kern/sched_prim.h>
    pub fn wakeup(channel: VmOffset);
    pub fn assert_wait(event: *mut c_void, interruptible: c_int);
    pub fn thread_block(continuation: Option<unsafe extern "C" fn()>);

    // <device/ds_routines.h>, the request passed as an opaque handle:
    // `struct io_req` itself belongs to its driver.
    pub fn iodone(ior: *mut c_void);
    pub fn device_read_alloc(ior: *mut c_void, size: usize) -> c_int;
    pub fn ds_read_done(ior: *mut c_void) -> c_int;

    // <machine/spl.h>: asm functions, `SPLKD` is a macro over `spltty`.
    pub fn splhi() -> c_int;
    pub fn spltty() -> c_int;
    pub fn splsoftclock() -> c_int;
    pub fn splx(level: c_int) -> c_int;

    // <i386at/com.h>
    pub fn comgetc(unit: c_int) -> c_int;

    // <device/tty.h> and <device/cirbuf.h>
    pub fn ttychars(tp: *mut c_void);
    pub fn char_open(
        dev: c_int,
        tp: *mut c_void,
        mode: c_int,
        ior: *mut c_void,
    ) -> c_int;
    pub fn ttyclose(tp: *mut c_void);
    pub fn tty_get_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: *mut u32,
    ) -> c_int;
    pub fn tty_set_status(
        tp: *mut c_void,
        flavor: c_uint,
        data: *mut c_int,
        count: u32,
    ) -> c_int;
    pub fn tty_portdeath(tp: *mut c_void, port: *mut c_void) -> c_int;
    pub fn tty_queue_completion(queue: *mut c_void);
    pub fn getc(buf: *mut c_void) -> c_int;

    // Shims in i386/i386at/kd_glue.c, for the tty lock macros, the
    // line discipline switch, `ttlowat[]` and `phystokv()`.
    pub fn kd_simple_lock_irq(lock: *mut c_void) -> c_int;
    pub fn kd_simple_unlock_irq(s: c_int, lock: *mut c_void);
    pub fn kd_simple_lock(lock: *mut c_void);
    pub fn kd_simple_unlock(lock: *mut c_void);
    pub fn kd_ldisc_read(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_write(
        line: c_int,
        tp: *mut c_void,
        ior: *mut c_void,
    ) -> c_int;
    pub fn kd_ldisc_rint(line: c_int, c: c_uint, tp: *mut c_void);
    pub fn kd_ttlowat(speed: c_int) -> c_short;

    // <kern/mach_clock.h> and <i386/i386at/model_dep.c>
    pub static hz: c_int;
    pub static rebootflag: c_int;

    // Shims for the C macros Rust cannot call: see i386/i386/pio_glue.c.
    pub fn pio_inb(port: u16) -> u8;
    pub fn pio_inw(port: u16) -> u16;
    pub fn pio_inl(port: u16) -> u32;
    pub fn pio_outb(port: u16, value: u8);
    pub fn pio_outw(port: u16, value: u16);
    pub fn pio_outl(port: u16, value: u32);

    // Shims for `mask_irq'/'unmask_irq' (static inline under APIC) and
    // for the NINTR-sized `ivect'/`iunit' arrays: see i386/i386/irq.c.
    pub fn irq_mask(irq: c_uint);
    pub fn irq_unmask(irq: c_uint);
    pub fn irq_set_handler(
        irq: c_int,
        handler: Option<unsafe extern "C" fn(c_int)>,
    );
    pub fn irq_get_handler(irq: c_int) -> Option<unsafe extern "C" fn(c_int)>;
    pub fn irq_set_unit(irq: c_int, unit: c_int);
    pub fn irq_get_unit(irq: c_int) -> c_int;

    // Shims for the NCOM-sized `cominfo' array: see i386/i386at/com.c.
    pub fn com_base_addr(unit: c_int) -> VmOffset;
    pub fn com_irq(unit: c_int) -> c_int;

    // <i386at/biosmem.h>, used by the /dev/mem mmap hook.
    pub fn biosmem_addr_available(addr: VmOffset) -> c_int;

    // <ipc/ipc_thread_glue.c>: the ith_next/ith_prev pair of a thread,
    // as one `struct ipc_thread_links *`.
    pub fn ipc_thread_glue_links(thread: *mut c_void) -> *mut c_void;

    // <kern/lock.c>: the sleep-capable recursive lock whose layout is
    // `kern/lock.rs`'s `LockData`.  Only `vm/vm_map.c` uses them so far.
    pub fn lock_init(lock: *mut LockData, can_sleep: c_int);
    pub fn lock_read(lock: *mut LockData);
    pub fn lock_write(lock: *mut LockData);
    pub fn lock_done(lock: *mut LockData);
    pub fn lock_read_to_write(lock: *mut LockData) -> c_int;

    // <kern/slab.h>.  `kmem_cache_alloc` returns the object address as
    // the C code does; the caller turns it into a pointer.
    pub fn kmem_cache_alloc(cache: *mut c_void) -> VmOffset;
    pub fn kmem_cache_free(cache: *mut c_void, obj: VmOffset);

    // The three caches of `vm/vm_map.c`, which still defines them.
    pub static mut vm_map_cache: c_void;
    pub static mut vm_map_entry_cache: c_void;
    pub static mut vm_map_copy_cache: c_void;

    // <vm/vm_kern.c>.  The map is passed as an opaque handle here:
    // `VmMap` is `!Unpin` (it embeds a list), and the C signature only
    // needs the address.
    pub fn projected_buffer_collect(map: *mut c_void) -> c_int;

    // <vm/pmap.h> and <i386/intel/pmap.h>.
    pub fn pmap_destroy(pmap: *mut Pmap);
    pub static kernel_pmap: *mut Pmap;

    // <vm/vm_page.h>.  `VM_PAGE_WAIT` is a macro over `vm_page_wait`.
    pub fn vm_page_mem_size() -> VmSize;
    pub fn vm_page_grab(flags: c_uint) -> *mut VmPage;
    pub fn vm_page_copy(src: *mut VmPage, dst: *mut VmPage);
    pub fn vm_page_wait(continuation: Option<unsafe extern "C" fn()>);
    // The page queue lock must be held for `replace`, `wire` and
    // `activate`, as the page-list copyout does.
    pub fn vm_page_replace(
        page: *mut VmPage,
        object: *mut VmObject,
        offset: VmOffset,
    );
    pub fn vm_page_wire(page: *mut VmPage);
    pub fn vm_page_activate(page: *mut VmPage);

    // <vm/pmap.h>: make a pmap range pageable, used when a wired copy
    // is entered.
    pub fn pmap_pageable(
        pmap: *mut Pmap,
        start: VmOffset,
        end: VmOffset,
        pageable: c_int,
    );

    // Shims in vm/vm_map_glue.c: the thread privilege bump the map
    // lock performs through `current_thread()`, and the machine-dependent
    // `pmap_attribute` macro.  Both die when their owners move.
    pub fn vm_map_glue_privilege_inc();
    pub fn vm_map_glue_privilege_dec();
    pub fn vm_map_glue_pmap_attribute(
        pmap: *mut Pmap,
        address: VmOffset,
        size: VmSize,
        attribute: c_uint,
        value: *mut c_int,
    ) -> c_int;
    pub fn vm_map_glue_thread_wakeup(event: *mut c_void);
    pub fn vm_map_glue_object_lock(object: *mut VmObject);
    pub fn vm_map_glue_object_unlock(object: *mut VmObject);
    pub fn vm_map_glue_object_can_release(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_is_pristine_submap(
        object: *mut VmObject,
    ) -> c_int;
    pub fn vm_map_glue_object_needs_shadow(
        object: *mut VmObject,
        size: VmSize,
        needs_copy: c_int,
        is_shared: c_int,
    ) -> c_int;
    pub fn vm_map_glue_object_is_temporary(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_use_shared_copy(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_make_shared(object: *mut VmObject);
    pub fn vm_map_glue_object_paging_begin(object: *mut VmObject);
    pub fn vm_map_glue_object_paging_end(object: *mut VmObject);
    pub fn vm_map_glue_object_can_coalesce(object: *mut VmObject) -> c_int;
    pub fn vm_map_glue_object_extend_size(object: *mut VmObject, size: VmSize);
    pub fn vm_map_glue_page_is_absent(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_is_tabled(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_object(page: *mut VmPage) -> *mut VmObject;
    pub fn vm_map_glue_page_free(page: *mut VmPage);
    pub fn vm_map_glue_page_set_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_clear_busy(page: *mut VmPage);
    pub fn vm_map_glue_page_set_dirty(page: *mut VmPage);
    pub fn vm_map_glue_page_wakeup_done(page: *mut VmPage);
    pub fn vm_map_glue_page_activate_if_idle(page: *mut VmPage);
    pub fn vm_map_glue_page_wire_count(page: *mut VmPage) -> c_int;
    pub fn vm_map_glue_page_offset(page: *mut VmPage) -> VmOffset;
    pub fn vm_map_glue_page_queue_lock();
    pub fn vm_map_glue_page_queue_unlock();
    pub fn vm_map_glue_pmap_enter(
        pmap: *mut Pmap,
        addr: VmOffset,
        page: *mut VmPage,
        protection: c_int,
        wired: c_int,
    );
    pub static mut vm_submap_object: *mut VmObject;

    // <vm/vm_page.h>.
    pub fn vm_page_lookup(
        object: *mut VmObject,
        offset: VmOffset,
    ) -> *mut VmPage;

    // <vm/vm_object.h> and <vm/pmap.h>: the object and pmap operations
    // the deletion path reaches.
    pub fn vm_object_reference(object: *mut VmObject);
    pub fn vm_object_deallocate(object: *mut VmObject);
    pub fn vm_object_page_remove(
        object: *mut VmObject,
        start: VmOffset,
        end: VmOffset,
    );
    pub fn vm_object_pmap_remove(
        object: *mut VmObject,
        start: VmOffset,
        end: VmOffset,
    );
    pub fn vm_object_coalesce(
        prev_object: *mut VmObject,
        next_object: *mut VmObject,
        prev_offset: VmOffset,
        next_offset: VmOffset,
        prev_size: VmSize,
        next_size: VmSize,
        new_object: *mut *mut VmObject,
        new_offset: *mut VmOffset,
    ) -> c_int;
    pub static kernel_object: *mut VmObject;
    pub static kernel_map: *mut c_void;
    pub static kernel_virtual_start: VmOffset;
    pub static kernel_virtual_end: VmOffset;
    pub fn pmap_remove(pmap: *mut Pmap, start: VmOffset, end: VmOffset);
    pub fn pmap_protect(
        pmap: *mut Pmap,
        start: VmOffset,
        end: VmOffset,
        prot: c_int,
    );
    pub fn vm_fault_unwire(map: *mut c_void, entry: *mut c_void);
    pub fn vm_fault_wire(map: *mut c_void, entry: *mut c_void);

    // <vm/vm_fault.h>: copy pages between objects for the overwrite.
    // The size is in/out and the version is the caller's map-version
    // snapshot.
    pub fn vm_fault_copy(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        src_size: *mut VmSize,
        dst_object: *mut VmObject,
        dst_offset: VmOffset,
        dst_map: *mut c_void,
        dst_version: *mut c_void,
        interruptible: c_int,
    ) -> c_int;

    // <vm/vm_object.h>.
    pub fn vm_object_shadow(
        object: *mut *mut VmObject,
        offset: *mut VmOffset,
        length: VmSize,
    );
    pub fn vm_object_allocate(size: VmSize) -> *mut VmObject;
    pub fn vm_object_collapse(object: *mut VmObject);
    pub fn vm_object_copy_slowly(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        size: VmSize,
        interruptible: c_int,
        result_object: *mut *mut VmObject,
    ) -> c_int;
    pub fn vm_object_copy_strategically(
        src_object: *mut VmObject,
        src_offset: VmOffset,
        size: VmSize,
        dst_object: *mut *mut VmObject,
        dst_offset: *mut VmOffset,
        dst_needs_copy: *mut c_int,
    ) -> c_int;
    pub fn vm_object_copy_temporary(
        object: *mut *mut VmObject,
        offset: *mut VmOffset,
        src_needs_copy: *mut c_int,
        dst_needs_copy: *mut c_int,
    ) -> c_int;
    pub fn vm_object_pmap_protect(
        object: *mut VmObject,
        offset: VmOffset,
        size: VmSize,
        pmap: *mut Pmap,
        start: VmOffset,
        protection: c_int,
    );

    // <vm/pmap.h>: the physical-map operations of the fork.  `pmap_copy`
    // is a macro here, so it comes through the vm_map_glue.c shim.
    pub fn pmap_create(size: VmSize) -> *mut Pmap;
    pub fn vm_map_glue_pmap_copy(
        dst: *mut Pmap,
        src: *mut Pmap,
        dst_addr: VmOffset,
        len: VmSize,
        src_addr: VmOffset,
    );

    // <kern/lock.h>: the recursive/downgrade operations of the map
    // lock, used by the pageability scan.
    pub fn lock_set_recursive(lock: *mut LockData);
    pub fn lock_write_to_read(lock: *mut LockData);
    pub fn lock_clear_recursive(lock: *mut LockData);
}
