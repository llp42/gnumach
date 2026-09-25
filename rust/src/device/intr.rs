// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from device/intr.c:
//   Copyright (c) 2010, 2011, 2016, 2019 Free Software Foundation, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The interrupt device, which `device/intr.c` used to define and
//! <device/intr.h> declares.
//!
//! The registration list and the delivery thread are here; the `irqdev` and
//! `user_intr_t` records belong to <device/intr.h> and are mirrored by
//! [`crate::arch::i386::irq`].

use crate::arch::i386::ioapic::{self, InterruptHandler};
use crate::arch::i386::irq::{self, IrqDev, UserIntr};
use crate::arch::i386::percpu::current_thread;
use crate::config::NINTR;
use crate::device::r#return::DeviceError;
use crate::glue;
use crate::ipc::ipc_kmsg;
use crate::ipc::ipc_mqueue;
use crate::ipc::ipc_port;
use crate::ipc::{IpcPort, MachMsgHeader, MachMsgType};
use crate::kern::mach_clock::hz;
use crate::kern::queue::{
    QueueEntry, queue_end, queue_enter_tail, queue_first, queue_init,
    queue_next, queue_remove_generic,
};
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, clear_wait, thread_block,
    thread_set_timeout, thread_wakeup_prim,
};
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::current_task;
use crate::spin::{Mutex, MutexGuard};
use core::ffi::{c_int, c_uint, c_ulong, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{self, NonNull};

/// `IRQGETPICMODE` of <device/irq_status.h>.
const IRQGETPICMODE: c_uint = 0;

/// `SA_SHIRQ` of `device/intr.c`.
const SA_SHIRQ: c_ulong = 0x0400_0000;

/// `MACH_MSGH_BITS(MACH_MSG_TYPE_PORT_SEND, 0)` of <mach/message.h>.
const MACH_MSGH_BITS_PORT_SEND: u32 = 17;

/// `MACH_MSG_TYPE_INTEGER_32` of <mach/message.h>.
const MACH_MSG_TYPE_INTEGER_32: u32 = 2;

/// `DEVICE_INTR_NOTIFY` of <device/notify.h>.
const DEVICE_INTR_NOTIFY: c_int = 100;

/// `DEVICE_NOTIFY_MSGH_SEQNO` of <device/intr.h>.
const DEVICE_NOTIFY_MSGH_SEQNO: u32 = 0;

/// `KERN_INVALID_ARGUMENT` of <mach/kern_return.h>.
const KERN_INVALID_ARGUMENT: c_int = 4;

/// The `mach_msg_type_t` initializer of `deliver_intr()`.
#[cfg(target_pointer_width = "64")]
const INTR_TYPE: MachMsgType =
    MachMsgType::new(MACH_MSG_TYPE_INTEGER_32 | (32 << 8) | (1 << 29), 1);
/// The `mach_msg_type_t` initializer of `deliver_intr()`.
#[cfg(target_pointer_width = "32")]
const INTR_TYPE: MachMsgType =
    MachMsgType::new(MACH_MSG_TYPE_INTEGER_32 | (32 << 8) | (1 << 28), 1);

/// `device_intr_notification_t` of <device/notify.h>: the message a delivery
/// port receives, laid over the `struct ipc_kmsg` header.
#[repr(C)]
struct DeviceIntrNotification {
    intr_header: MachMsgHeader,
    intr_type: MachMsgType,
    id: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<DeviceIntrNotification>() == 48);
    assert!(core::mem::align_of::<DeviceIntrNotification>() == 8);
    assert!(offset_of!(DeviceIntrNotification, intr_header) == 0);
    assert!(offset_of!(DeviceIntrNotification, intr_type) == 32);
    assert!(offset_of!(DeviceIntrNotification, id) == 40);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<DeviceIntrNotification>() == 32);
    assert!(core::mem::align_of::<DeviceIntrNotification>() == 4);
    assert!(offset_of!(DeviceIntrNotification, intr_header) == 0);
    assert!(offset_of!(DeviceIntrNotification, intr_type) == 24);
    assert!(offset_of!(DeviceIntrNotification, id) == 28);
};

/// `struct intr_list` of `device/intr.c`: one shared-IRQ registration.
#[repr(C)]
struct IntrList {
    user_intr: *mut UserIntr,
    flags: c_ulong,
    next: *mut IntrList,
}

/// `user_intr_handlers[]` of `device/intr.c`: one list per interrupt.
static mut USER_INTR_HANDLERS: [*mut IntrList; NINTR] =
    [ptr::null_mut(); NINTR];

/// `main_intr_queue` of <device/intr.h>, the queue `irqtab` points at.
#[unsafe(export_name = "main_intr_queue")]
pub static mut MAIN_INTR_QUEUE: QueueEntry = QueueEntry::unlinked();

/// `intr_lock` of `device/intr.c`, the `simple_lock_irq` around the queue
/// and the handler lists.
static INTR_LOCK: Mutex<()> = Mutex::new(());

/// Take the lock at `splhigh`, as the `simple_lock_irq()` macro did.
fn lock_irq() -> (MutexGuard<'static, ()>, c_int) {
    // SAFETY: `splhigh()` is the real asm routine <machine/spl.h> declares,
    // and its result is only handed back to `splx()`.
    let level = unsafe { glue::splhigh() };
    (INTR_LOCK.lock(), level)
}

/// Release the lock and restore `level`, as `simple_unlock_irq()` did.
fn unlock_irq(guard: MutexGuard<'static, ()>, level: c_int) {
    drop(guard);
    // SAFETY: `level` is the value [`lock_irq()`] returned for this lock.
    unsafe { glue::splx(level) };
}

/// `e->dst_port` lost its last reference, or is unusable.
///
/// # Safety
///
/// `port` must be `MACH_PORT_NULL` or a live port, as the registration's own
/// reference keeps it until the entry is removed.
unsafe fn references_dead(port: *mut c_void) -> bool {
    match IpcPort::valid(port) {
        None => true,
        // SAFETY: `valid()` established the live port.
        Some(port) => (unsafe { port.references() }) == 1,
    }
}

/// Release the registration's reference on `port`.
///
/// # Safety
///
/// `port` must be `MACH_PORT_NULL` or a live port, as [`references_dead()`].
unsafe fn release_port(port: *mut c_void) {
    if let Some(port) = IpcPort::valid(port) {
        // SAFETY: `valid()` established the live port, which holds the
        // registration's reference.
        unsafe { port.release() };
    }
}

/// `irqtab.irq[id]`, or [`None`] outside the table.
///
/// # Safety
///
/// `dev` must be the live `irqtab`.
unsafe fn irq_of(dev: *mut IrqDev, id: c_int) -> Option<c_uint> {
    let index = usize::try_from(id).ok()?;
    // SAFETY: the caller promises the live table, and `get` keeps the index
    // inside its `NINTR` entries.
    unsafe { (*dev).irq.get(index).copied() }
}

/// The event the interrupt thread waits and wakes on: the C's
/// `(event_t) &intr_thread`.
fn intr_event() -> *mut c_void {
    intr_thread as *const () as *mut c_void
}

/// Whether the line's handler is `wanted`, compared by address.
fn is_handler(
    current: InterruptHandler,
    wanted: unsafe extern "C" fn(c_int),
) -> bool {
    match current {
        Some(current) => core::ptr::fn_addr_eq(current, wanted),
        None => false,
    }
}

/// Wake the interrupt thread, as the C's `thread_wakeup()` did.
fn wake_intr_thread() {
    // SAFETY: the event is this module's; a non-interruptible wait is woken
    // normally.
    unsafe { thread_wakeup_prim(intr_event(), 0, THREAD_AWAKENED) };
}

/// `search_intr()` of `device/intr.c`.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, whose `intr_queue` is an initialized
/// queue of [`UserIntr`] entries with the chain as their first field.
unsafe fn search_intr(
    dev: *mut IrqDev,
    dst_port: *mut c_void,
) -> Option<NonNull<UserIntr>> {
    // SAFETY: the caller promises the live table and the queue invariant.
    let head = unsafe { (*dev).intr_queue };
    // SAFETY: the queue head is initialized and the entries are live.
    for entry in unsafe { (*head).iter() } {
        let e = entry.as_ptr().cast::<UserIntr>();
        // SAFETY: the queue's entries are `UserIntr`s, chain first.
        if unsafe { (*e).dst_port } == dst_port {
            return NonNull::new(e);
        }
    }
    None
}

/// `queue_intr()` of `device/intr.c`: account a delivery and wake the
/// interrupt thread.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, `id` inside its `irq` table, and `e` the
/// live registration the line belongs to.
unsafe fn queue_intr(dev: *mut IrqDev, id: c_int, e: *mut UserIntr) {
    // SAFETY: the caller promises the live table and the entry.
    unsafe {
        if let Some(irq) = irq_of(dev, id) {
            irq::__disable_irq(irq);
        }
        (*e).n_unacked += 1;
        (*e).interrupts += 1;
        (*dev).tot_num_intr += 1;
    }
    wake_intr_thread();
}

/// `deliver_user_intr()` of `device/intr.c`.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, `id` inside its `irq` table, and `e` the
/// live registration for that line.
pub(crate) unsafe fn deliver_user_intr(
    dev: *mut IrqDev,
    id: c_int,
    e: *mut UserIntr,
) -> bool {
    // SAFETY: the caller promises the live entry; the port field is read
    // without the lock, as the C read it.
    if unsafe { references_dead((*e).dst_port) } {
        wake_intr_thread();
        false
    } else {
        // SAFETY: as the caller.
        unsafe { queue_intr(dev, id, e) };
        true
    }
}

/// `insert_intr_entry()` of `device/intr.c`.
///
/// # Safety
///
/// `dev` must be the live `irqtab` with an initialized `intr_queue`, and
/// `dst_port` a port the caller keeps alive.
pub(crate) unsafe fn insert_intr_entry(
    dev: *mut IrqDev,
    id: c_int,
    dst_port: *mut c_void,
) -> Option<NonNull<UserIntr>> {
    // SAFETY: `kalloc()` returns fresh storage for the size asked, or
    // nothing.
    let new = kalloc(size_of::<UserIntr>())?.cast::<UserIntr>();

    let (guard, level) = lock_irq();
    // SAFETY: the caller promises the live table and queue.
    let found = unsafe { search_intr(dev, dst_port) }.is_some();
    let result = if found {
        // SAFETY: `printf` is the real C routine, and the one `%d` and the
        // one `%p` take the `c_int` and pointer below.
        unsafe {
            glue::printf(
                c"the interrupt entry for irq[%d] and port %p has already been inserted\n"
                    .as_ptr(),
                id,
                dst_port,
            )
        };
        None
    } else {
        // SAFETY: `printf` is the real C routine; the `%d`, `%p` and `%s`
        // take the values below, and the task name is NUL-terminated.
        unsafe {
            glue::printf(
                c"irq handler [%d]: new delivery port %p entry %p for %s\n"
                    .as_ptr(),
                id,
                dst_port,
                new.as_ptr(),
                (*current_task()).name.as_ptr(),
            )
        };
        // SAFETY: `new` is fresh, unshared storage, and the lock is held.
        unsafe {
            (*new.as_ptr()).id = id;
            (*new.as_ptr()).dst_port = dst_port;
            (*new.as_ptr()).interrupts = 0;
            (*new.as_ptr()).n_unacked = 0;
            queue_enter_tail(
                (*dev).intr_queue,
                new.as_ptr().cast(),
                offset_of!(UserIntr, chain),
            );
        }
        Some(new)
    };
    unlock_irq(guard, level);

    if found {
        // SAFETY: the block came from `kalloc()` and was never linked.
        unsafe { kfree(new.cast(), size_of::<UserIntr>()) };
    }
    result
}

/// `user_irq_handler()` of `device/intr.c`: the vector a shared line points
/// at.
unsafe extern "C" fn user_irq_handler(id: c_int) {
    let (guard, level) = lock_irq();

    let index = usize::try_from(id).ok().filter(|index| *index < NINTR);
    // SAFETY: the lock is held, and `index` keeps the table access inside
    // `NINTR` exactly as the C's `user_intr_handlers[id]` assumed.
    if let Some(index) = index {
        // SAFETY: a listed node is this module's live `IntrList`.
        unsafe {
            let head = ptr::addr_of_mut!(USER_INTR_HANDLERS[index]);
            let mut prev = head;
            let mut handler = *head;
            while !handler.is_null() {
                let e = (*handler).user_intr;
                if !deliver_user_intr(ptr::addr_of_mut!(irq::irqtab), id, e) {
                    *prev = (*handler).next;
                }
                prev = ptr::addr_of_mut!((*handler).next);
                handler = (*handler).next;
            }
        }
    }

    unlock_irq(guard, level);
}

/// `install_user_intr_handler()` of `device/intr.c`.
///
/// # Safety
///
/// `dev` must be the live `irqtab`, `id` inside its `irq` table, and
/// `user_intr` the live entry [`insert_intr_entry()`] returned.
pub(crate) unsafe fn install_user_intr_handler(
    dev: *mut IrqDev,
    id: c_int,
    mut flags: c_ulong,
    user_intr: *mut UserIntr,
) -> Result<(), DeviceError> {
    flags |= SA_SHIRQ;

    // SAFETY: the caller promises the live table and a valid `id`.
    let irq = unsafe { irq_of(dev, id) };
    let Some(irq) = irq.and_then(|irq| c_int::try_from(irq).ok()) else {
        return Err(DeviceError::InvalidOperation);
    };

    let handler = irq::handler(irq);
    if !is_handler(handler, user_irq_handler)
        && !is_handler(handler, ioapic::intnull)
    {
        // SAFETY: `printf` is the real C routine; the two `%d`s take `id`
        // and `irq`.
        unsafe {
            glue::printf(
                c"You can't have this interrupt %d:%d\n".as_ptr(),
                id,
                irq,
            )
        };
        return Err(DeviceError::AlreadyOpen);
    }

    let index = usize::try_from(id).ok().filter(|index| *index < NINTR);
    let Some(index) = index else {
        return Err(DeviceError::InvalidOperation);
    };
    // SAFETY: `index` is inside the handlers table, and a non-null head
    // points at a live node.
    let old = unsafe { *ptr::addr_of!(USER_INTR_HANDLERS[index]) };
    if !old.is_null() {
        // SAFETY: a non-null head points at a live node.
        if unsafe { (*old).flags & flags & SA_SHIRQ } == 0 {
            unsafe { glue::printf(c"Cannot share irq\n".as_ptr()) };
            return Err(DeviceError::AlreadyOpen);
        }
    }

    // SAFETY: `kalloc()` returns fresh storage for the size asked, or
    // nothing.
    let Some(new) = kalloc(size_of::<IntrList>()) else {
        return Err(DeviceError::NoMemory);
    };
    let new = new.as_ptr().cast::<IntrList>();
    // SAFETY: `new` is fresh, unshared storage.
    unsafe {
        (*new).user_intr = user_intr;
        (*new).flags = flags;
    }

    let (guard, level) = lock_irq();
    // SAFETY: the lock is held, `new` is unlinked, and `index` is inside the
    // handlers table.
    unsafe {
        let head = ptr::addr_of_mut!(USER_INTR_HANDLERS[index]);
        (*new).next = *head;
        *head = new;
    }
    irq::set_handler(irq, Some(user_irq_handler));
    irq::set_unit(irq, irq);
    ioapic::unmask(irq);
    unlock_irq(guard, level);

    Ok(())
}

/// `deliver_intr()` of `device/intr.c`: send the notification message.
///
/// # Safety
///
/// `dst_port` must be `MACH_PORT_NULL` or a live port, and that port must be
/// the send right the caller holds.
unsafe fn deliver_intr(id: c_int, dst_port: *mut c_void) -> bool {
    if dst_port.is_null() {
        return false;
    }

    // SAFETY: the message size fits the notification record, and a `None`
    // means no buffer.
    let Some(kmsg) =
        (unsafe { ipc_kmsg::alloc(size_of::<DeviceIntrNotification>()) })
    else {
        return false;
    };

    // SAFETY: the fresh message's header is the notification record.
    let n = unsafe { kmsg.header().cast::<DeviceIntrNotification>() };
    let size = u32::try_from(size_of::<DeviceIntrNotification>()).unwrap_or(0);
    // SAFETY: `kmsg` is live and this call owns it until the send below.
    unsafe {
        (*n).intr_header.set_bits(MACH_MSGH_BITS_PORT_SEND);
        (*n).intr_header.set_size(size);
        (*n).intr_header.set_seqno(DEVICE_NOTIFY_MSGH_SEQNO);
        (*n).intr_header.set_local(0);
        (*n).intr_header.set_remote(0);
        (*n).intr_header.set_id(DEVICE_INTR_NOTIFY);
        (*n).intr_type = INTR_TYPE;
        (*n).id = id;
        (*n).intr_header.set_remote(dst_port.addr());
    }

    // SAFETY: the caller holds a send right on the live port, and the fresh
    // message's remote field names it.
    unsafe { ipc_port::copy_send(dst_port) };
    // SAFETY: `kmsg` is a live message this call owns and the remote port
    // holds the reference `copy_send()` just made.
    unsafe { ipc_mqueue::send_always(kmsg.as_ptr()) };
    true
}

/// `intr_thread()` of `device/intr.c`: deliver the queued user interrupts.
///
/// # Safety
///
/// Started once, as the `intr` kernel thread, after the device layer and the
/// interrupt tables exist.
pub(crate) unsafe fn intr_thread() {
    // SAFETY: the current thread is the one `kernel_thread()` started for
    // this routine.
    unsafe { (*current_thread()).vm_privilege = 1 };
    // SAFETY: the queue is this module's and no entry is linked into it yet.
    unsafe { queue_init(ptr::addr_of_mut!(MAIN_INTR_QUEUE)) };

    loop {
        // SAFETY: this is the interrupt thread; the event is the one every
        // wakeup names, and `hz` is the kernel's tick rate.
        unsafe {
            assert_wait(intr_event(), 0);
            thread_set_timeout(hz);
        }
        let (mut guard, mut level) = lock_irq();

        loop {
            let mut deleted = ptr::null_mut::<UserIntr>();
            // SAFETY: the lock is held and the queue initialized.
            let (head, mut e) = unsafe {
                let head = ptr::addr_of_mut!(MAIN_INTR_QUEUE);
                (head, queue_first(head).cast::<UserIntr>())
            };
            while unsafe { queue_end(head, e.cast()) } == 0 {
                // SAFETY: `e` is the entry the queue links point at.
                let dst_port = unsafe { (*e).dst_port };
                // SAFETY: the lock is held; the port field is a registration
                // slot.
                if unsafe { references_dead(dst_port) } {
                    // SAFETY: the current thread is the one waiting above.
                    unsafe { clear_wait(current_thread(), 0, 0) };
                    deleted = e;
                    break;
                }

                // SAFETY: `e` is the entry the queue links point at.
                if unsafe { (*e).interrupts } != 0 {
                    // SAFETY: the current thread is the one waiting above.
                    unsafe { clear_wait(current_thread(), 0, 0) };
                    // SAFETY: `e` is live; the lock is still held, and
                    // `irqtab` is the live table.
                    let id = unsafe {
                        let id = (*e).id;
                        (*e).interrupts -= 1;
                        (*ptr::addr_of_mut!(irq::irqtab)).tot_num_intr -= 1;
                        id
                    };
                    unlock_irq(guard, level);
                    // SAFETY: the entry and its port are live; the C made
                    // the same drop of the lock before sending.
                    unsafe { deliver_intr(id, dst_port) };
                    (guard, level) = lock_irq();
                }

                // SAFETY: `e` is linked, and the lock is held.
                e = unsafe { queue_next(ptr::addr_of_mut!((*e).chain)) }
                    .cast::<UserIntr>();
            }

            if !deleted.is_null() {
                // SAFETY: `deleted` is linked in the queue and the lock is
                // held.
                unsafe {
                    queue_remove_generic(
                        head,
                        deleted.cast(),
                        offset_of!(UserIntr, chain),
                    );
                    glue::printf(
                        c"irq handler [%d]: release a dead delivery port %p entry %p\n"
                            .as_ptr(),
                        (*deleted).id,
                        (*deleted).dst_port,
                        deleted,
                    );
                    release_port((*deleted).dst_port);
                    (*deleted).dst_port = ptr::null_mut();

                    if (*deleted).n_unacked != 0 {
                        glue::printf(
                            c"irq handler [%d]: still %d unacked irqs in entry %p\n"
                                .as_ptr(),
                            (*deleted).id,
                            (*deleted).n_unacked,
                            deleted,
                        );
                    }
                    while (*deleted).n_unacked != 0 {
                        if let Some(irq) = irq_of(
                            ptr::addr_of_mut!(irq::irqtab),
                            (*deleted).id,
                        ) {
                            irq::__enable_irq(irq);
                        }
                        (*deleted).n_unacked -= 1;
                    }

                    let tot = ptr::addr_of_mut!(irq::irqtab);
                    (*tot).tot_num_intr -= (*deleted).interrupts;
                    (*deleted).interrupts = 0;
                }
            }

            // SAFETY: `irqtab` is the live table, and the lock is held.
            let pending =
                unsafe { (*ptr::addr_of!(irq::irqtab)).tot_num_intr };
            if deleted.is_null() && pending == 0 {
                break;
            }
        }

        unlock_irq(guard, level);
        // SAFETY: this thread is the one that waited above; the null
        // continuation resumes it at the top of the loop.
        unsafe { thread_block(None) };
    }
}

/// Enable the line registration `id` names, outside the lock, as
/// `irq_acknowledge()` did.
pub(crate) fn enable_line(id: c_int) {
    // SAFETY: `irqtab` is the live table, and `id` came from a registration.
    if let Some(irq) = unsafe { irq_of(ptr::addr_of_mut!(irq::irqtab), id) } {
        irq::__enable_irq(irq);
    }
}

/// `irq_acknowledge()` of `device/intr.c`: account a userland acknowledgement
/// and report the line to enable.
///
/// # Safety
///
/// `receive_port` must be the port named by a live registration.
pub(crate) unsafe fn irq_acknowledge(
    receive_port: *mut c_void,
) -> Result<c_int, c_int> {
    let (guard, level) = lock_irq();
    // SAFETY: the caller promises the live table.
    let entry =
        unsafe { search_intr(ptr::addr_of_mut!(irq::irqtab), receive_port) };
    let result = match entry {
        None => {
            // SAFETY: `printf` is the real C routine and takes no varargs.
            unsafe {
                glue::printf(
                    c"didn't find user intr for interrupt !?\n".as_ptr(),
                )
            };
            Err(KERN_INVALID_ARGUMENT)
        }
        Some(e) => {
            // SAFETY: `e` is live under the lock.
            if unsafe { (*e.as_ptr()).n_unacked } == 0 {
                Err(DeviceError::InvalidOperation as c_int)
            } else {
                // SAFETY: the count is nonzero, as the branch checked.
                unsafe { (*e.as_ptr()).n_unacked -= 1 };
                Ok(unsafe { (*e.as_ptr()).id })
            }
        }
    };
    unlock_irq(guard, level);
    result
}

/// The status reply for `flavor`, or [`None`] for a flavor the device does
/// not serve.
pub(crate) fn getstat(flavor: c_uint) -> Option<(c_int, u32)> {
    match flavor {
        IRQGETPICMODE => {
            // SAFETY: `pic_mode` is the machine global the APIC setup
            // initialized before the device layer starts and never wrote
            // again.
            let mode = unsafe { ioapic::pic_mode };
            Some((mode, 1))
        }
        _ => None,
    }
}
