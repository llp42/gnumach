// SPDX-License-Identifier: CMU-Mach
// Derived from vm/memory_object.c and vm/memory_object.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The external memory management interface, which `vm/memory_object.c` used
//! to define and `vm/memory_object.h` declares.

use crate::arch::types::{VmOffset, VmSize};
use crate::arch::vm_param::{PAGE_SHIFT, PAGE_SIZE};
use crate::glue::{
    memory_object_change_completed, memory_object_data_return,
    memory_object_lock_completed, memory_object_supply_completed,
    pmap_clear_modify, pmap_is_modified, pmap_page_protect,
    vm_page_queue_lock,
};
use crate::ipc::{IpcPort, ipc_port};
use crate::kern::debug::kpanic;
use crate::kern::host::Host;
use crate::kern::lock::SimpleLock;
use crate::kern::sched_prim::{
    THREAD_AWAKENED, assert_wait, thread_block, thread_sleep,
    thread_wakeup_prim,
};
use crate::vm::error::{Error, error_from_kern_return};
use crate::vm::types::{VmObject, VmPage, VmProt};
use crate::vm::vm_map::{VmMapCopy, round_page};
use crate::vm::vm_pageout_ffi::vm_pageout_setup;
use crate::vm::{vm_external, vm_object, vm_page, vm_resident};
use core::ffi::{c_int, c_uint, c_void};
use core::ptr::{NonNull, addr_of_mut, null_mut};
use core::sync::atomic::Ordering;

/// `MEMORY_OBJECT_RETURN_*` of <mach/memory_object.h>: what a lock request
/// asks the kernel to return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Return {
    /// `MEMORY_OBJECT_RETURN_NONE`.
    None,
    /// `MEMORY_OBJECT_RETURN_DIRTY`.
    Dirty,
    /// `MEMORY_OBJECT_RETURN_ALL`.
    All,
}

impl Return {
    /// The value a C `memory_object_return_t` names; anything outside the
    /// three behaves as `DIRTY` did in the C's two comparisons.
    pub(crate) const fn from_c(code: c_int) -> Self {
        match code {
            0 => Return::None,
            2 => Return::All,
            _ => Return::Dirty,
        }
    }
}

/// `MEMORY_OBJECT_COPY_*` of <mach/memory_object.h>: the strategies a memory
/// manager may ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyStrategy {
    /// `MEMORY_OBJECT_COPY_NONE`.
    None,
    /// `MEMORY_OBJECT_COPY_CALL`.
    Call,
    /// `MEMORY_OBJECT_COPY_DELAY`.
    Delay,
    /// `MEMORY_OBJECT_COPY_TEMPORARY`.
    Temporary,
}

impl CopyStrategy {
    /// The strategy a C `memory_object_copy_strategy_t` names, or `None`
    /// when the value is not one of the four.
    pub(crate) const fn from_c(code: c_int) -> Option<Self> {
        match code {
            0 => Some(CopyStrategy::None),
            1 => Some(CopyStrategy::Call),
            2 => Some(CopyStrategy::Delay),
            3 => Some(CopyStrategy::Temporary),
            _ => None,
        }
    }

    /// The C `memory_object_copy_strategy_t` this strategy names.
    pub(crate) const fn as_c(self) -> c_int {
        match self {
            CopyStrategy::None => 0,
            CopyStrategy::Call => 1,
            CopyStrategy::Delay => 2,
            CopyStrategy::Temporary => 3,
        }
    }
}

/// `MEMORY_OBJECT_LOCK_RESULT_*` of `vm/memory_object.c`: what a page lock
/// did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockResult {
    Done,
    MustBlock,
    MustClean,
    MustReturn,
}

/// `VM_EXTERNAL_SMALL_SIZE` and `VM_EXTERNAL_LARGE_SIZE` of
/// <vm/vm_external.h>.
const VM_EXTERNAL_SMALL_SIZE: VmSize = 128;
const VM_EXTERNAL_LARGE_SIZE: VmSize = 8192;

/// `DATA_WRITE_MAX` of `vm/memory_object.c`: how many holding pages one
/// data-return message may carry.
const DATA_WRITE_MAX: usize = 32;

/// One `memory_object_lock_request()` in Rust terms.
#[derive(Clone, Copy)]
pub(crate) struct LockRequest {
    pub(crate) offset: VmOffset,
    pub(crate) size: VmSize,
    pub(crate) should_return: Return,
    pub(crate) should_flush: bool,
    pub(crate) prot: VmProt,
    pub(crate) reply_to: *mut c_void,
    pub(crate) reply_to_type: c_uint,
}

/// One `memory_object_data_supply()` in Rust terms.
#[derive(Clone, Copy)]
pub(crate) struct SupplyRequest {
    pub(crate) offset: VmOffset,
    pub(crate) data: VmOffset,
    pub(crate) data_cnt: c_uint,
    pub(crate) lock_value: VmProt,
    pub(crate) precious: bool,
    pub(crate) reply_to: *mut c_void,
    pub(crate) reply_to_type: c_uint,
}

/// `memory_manager_default` of vm/memory_object.h: the port of the default
/// memory manager, or `IP_NULL`.
#[unsafe(no_mangle)]
pub static mut memory_manager_default: *mut c_void = null_mut();

/// `memory_manager_default_lock` of vm/memory_object.c.
static mut MEMORY_MANAGER_DEFAULT_LOCK: SimpleLock = SimpleLock::new();

/// `atop()` of <vm/vm_page.h>.
const fn atop(address: VmOffset) -> usize {
    address >> PAGE_SHIFT
}

/// `panic()` of vm/memory_object.c.
fn die(func: &'static str, message: &'static str) -> ! {
    kpanic!(func, "{}", message)
}

/// `VM_PAGE_FREE()` of <vm/vm_page.h>.
///
/// # Safety
///
/// `page` must be a live page that the caller owns, and the page-queues lock
/// must not be held.
unsafe fn page_free(page: *mut VmPage) {
    // SAFETY: the caller promises the live page; the queue lock is the C
    // macro's.
    unsafe {
        (*addr_of_mut!(vm_page_queue_lock)).lock();
        vm_resident::free(NonNull::new_unchecked(page));
        (*addr_of_mut!(vm_page_queue_lock)).unlock();
    }
}

/// `PAGE_WAKEUP()` of <vm/vm_page.h>.
///
/// # Safety
///
/// `page` must be a live page whose object lock the caller holds.
unsafe fn page_wakeup(page: *mut VmPage) {
    // SAFETY: the caller promises the live page and its lock.
    unsafe { vm_object::page_wakeup(page) };
}

/// `memory_object_lock_page()` in C: apply the lock request to one page.
///
/// # Safety
///
/// `page` must be a live page whose object lock the caller holds.
unsafe fn lock_page(
    page: *mut VmPage,
    should_return: Return,
    should_flush: bool,
    mut prot: VmProt,
) -> LockResult {
    // SAFETY: the caller promises the live page and its object lock.
    if unsafe { (*page).is_absent() } {
        return LockResult::Done;
    }

    // SAFETY: as above.
    if unsafe { (*page).is_busy() } {
        return LockResult::MustBlock;
    }

    // SAFETY: as above.
    if unsafe { (*page).wire_count() } != 0 {
        let unchanged = !should_flush
            && ((unsafe { (*page).page_lock() } == prot)
                || prot == VmProt::NO_CHANGE)
            && (should_return == Return::None
                || (!unsafe { (*page).is_dirty() }
                    && unsafe { pmap_is_modified((*page).phys_addr) } == 0
                    && (!unsafe { (*page).is_precious() }
                        || should_return != Return::All)));
        if unchanged {
            // SAFETY: the caller holds the object lock.
            unsafe {
                (*page).set_unlock_request(VmProt::NONE);
            }
            // SAFETY: the caller holds the object lock.
            unsafe { page_wakeup(page) };

            return LockResult::Done;
        }

        return LockResult::MustBlock;
    }

    if should_flush {
        prot = VmProt::ALL;
    }

    if prot != VmProt::NO_CHANGE {
        // SAFETY: the caller holds the object lock; the page is live.
        let decreases = unsafe {
            (((*page).page_lock().bits() ^ prot.bits()) & prot.bits()) != 0
        };
        if decreases {
            // SAFETY: the page is live, so its physical address names real
            // memory.
            unsafe {
                pmap_page_protect(
                    (*page).phys_addr,
                    (VmProt::ALL & !prot).bits(),
                )
            };
        }
        // SAFETY: the caller holds the object lock.
        unsafe {
            (*page).set_page_lock(prot);
            (*page).set_unlock_request(VmProt::NONE);
        }
        // SAFETY: as above.
        unsafe { page_wakeup(page) };
    }

    if should_return != Return::None {
        // SAFETY: the page is live and the object lock is held.
        unsafe {
            if !(*page).is_dirty() {
                (*page).set_dirty(pmap_is_modified((*page).phys_addr) != 0);
            }
        }

        // SAFETY: as above.
        let clean = unsafe {
            (*page).is_dirty()
                || ((*page).is_precious() && should_return == Return::All)
        };
        if clean {
            // SAFETY: the page-queues lock is the C macro's, and the page is
            // live.
            unsafe {
                (*addr_of_mut!(vm_page_queue_lock)).lock();
                vm_page::queues_remove(page);
                (*addr_of_mut!(vm_page_queue_lock)).unlock();
            }

            if !should_flush {
                // SAFETY: the page is live, so its physical address names
                // real memory.
                unsafe {
                    pmap_page_protect((*page).phys_addr, VmProt::NONE.bits())
                };
            }

            // SAFETY: the page is live and its dirty flag was just read.
            return if unsafe { (*page).is_dirty() } {
                LockResult::MustClean
            } else {
                LockResult::MustReturn
            };
        }
    }

    if should_flush {
        // SAFETY: the caller owns the page and holds its object lock.
        unsafe { page_free(page) };
    } else if vm_resident::VM_PAGE_DEACTIVATE_HINT.load(Ordering::Relaxed)
        && should_return != Return::None
    {
        // SAFETY: the page-queues lock is the C's, and the page is live.
        unsafe {
            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_page::deactivate(page);
            (*addr_of_mut!(vm_page_queue_lock)).unlock();
        }
    }

    LockResult::Done
}

/// One run of pages collected for a `memory_object_data_return()` message;
/// the state the C's `PAGEOUT_PAGES` macro carried between iterations.
struct PageoutBatch {
    new_object: *mut VmObject,
    new_offset: VmOffset,
    paging_offset: VmOffset,
    action: LockResult,
    holding: [*mut VmPage; DATA_WRITE_MAX],
}

impl PageoutBatch {
    const fn new() -> Self {
        Self {
            new_object: null_mut(),
            new_offset: 0,
            paging_offset: 0,
            action: LockResult::Done,
            holding: [null_mut(); DATA_WRITE_MAX],
        }
    }

    /// `PAGEOUT_PAGES` of `vm/memory_object.c`.
    ///
    /// # Safety
    ///
    /// The caller must hold `object`'s lock and a paging reference; the
    /// batch's `new_object` must be a live object.
    unsafe fn flush(&mut self, object: *mut VmObject, should_flush: bool) {
        // SAFETY: the caller holds the object lock, as the C did.
        unsafe { (*object).lock.unlock() };

        // SAFETY: the batch's object is live and held by the paging
        // reference, and the copy cache is initialized.
        let copy = unsafe {
            VmMapCopy::copyin_object(self.new_object, 0, self.new_offset)
        };

        // SAFETY: the object is live, and its pager fields are valid under
        // the paging reference the caller holds; the copy is the live
        // page-list copy just made.
        unsafe {
            memory_object_data_return(
                (*object).pager,
                (*object).pager_request,
                self.paging_offset,
                // `pointer_t` is a `vm_offset_t`, and a pointer is the same
                // width on both targets.
                copy.as_ptr() as VmOffset,
                // The copy holds at most `DATA_WRITE_MAX` pages, so the
                // byte count fits the C's `mach_msg_type_number_t`.
                self.new_offset as c_uint,
                c_int::from(self.action == LockResult::MustClean),
                c_int::from(!should_flush),
            );
            (*object).lock.lock();
        }

        let mut i = 0;
        while i < atop(self.new_offset) {
            let page = self.holding[i];
            if !page.is_null() {
                // SAFETY: the batch owns the holding page, and the object
                // lock is held as the macro's call site had it.
                unsafe { page_free(page) };
            }
            i += 1;
        }

        self.new_object = null_mut();
    }
}

/// `memory_object_lock_request()` in C: apply a lock request to every page of
/// the object's range.
///
/// # Safety
///
/// A non-null `object` must be a live object, and the call consumes the
/// caller's reference to it; `reply_to` must be `IP_NULL` or a live port.
pub(crate) unsafe fn lock_request(
    object: *mut VmObject,
    request: &LockRequest,
) -> Result<(), Error> {
    let LockRequest {
        offset,
        size,
        should_return,
        should_flush,
        prot,
        reply_to,
        reply_to_type,
    } = *request;

    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };
    if (prot.bits() & !VmProt::ALL.bits()) != 0 && prot != VmProt::NO_CHANGE {
        return Err(Error::InvalidArgument);
    }

    let object = object.as_ptr();
    let original_offset = offset;
    let original_size = size;
    let mut size = round_page(size);
    let mut batch = PageoutBatch::new();
    let mut last_offset = offset;
    let mut pageout_action = LockResult::Done;

    // SAFETY: the caller promises the live object.
    unsafe {
        (*object).lock.lock();
        vm_object::paging_begin(object);
    }
    // SAFETY: the object is live and its lock is held.
    let mut offset = offset.wrapping_sub(unsafe { (*object).paging_offset });

    while size != 0 {
        if !batch.new_object.is_null()
            && batch.new_offset >= PAGE_SIZE * DATA_WRITE_MAX as VmOffset
        {
            // SAFETY: the batch is live and the object lock is held.
            unsafe { batch.flush(object, should_flush) };
        }

        // SAFETY: the object is live and locked, as `lookup` requires.
        while let Some(page) = unsafe {
            vm_resident::lookup(NonNull::new_unchecked(object), offset)
        } {
            let page = page.as_ptr();
            // SAFETY: the page is live and the object lock is held.
            let result =
                unsafe { lock_page(page, should_return, should_flush, prot) };

            match result {
                LockResult::Done => {
                    if !batch.new_object.is_null() {
                        // SAFETY: the batch is live and the object lock is
                        // held.
                        unsafe { batch.flush(object, should_flush) };
                        continue;
                    }
                }
                LockResult::MustBlock => {
                    if !batch.new_object.is_null() {
                        // SAFETY: the batch is live and the object lock is
                        // held.
                        unsafe { batch.flush(object, should_flush) };
                        continue;
                    }

                    // SAFETY: the page is live and the object lock is held,
                    // as `PAGE_ASSERT_WAIT` requires.
                    unsafe {
                        (*page).set_wanted(true);
                        assert_wait(page.cast(), 0);
                        (*object).lock.unlock();
                        thread_block(None);
                        (*object).lock.lock();
                    }
                    continue;
                }
                LockResult::MustClean | LockResult::MustReturn => {
                    // SAFETY: the page is live and the object lock is held.
                    unsafe { (*page).set_busy(true) };

                    if !batch.new_object.is_null()
                        && (last_offset != offset || pageout_action != result)
                    {
                        // SAFETY: the batch is live and the object lock is
                        // held.
                        unsafe { batch.flush(object, should_flush) };
                    }

                    // SAFETY: the page is live, and `vm_object_allocate()`
                    // and `vm_pageout_setup()` run unlocked, as the C did.
                    unsafe {
                        (*object).lock.unlock();
                    }

                    if batch.new_object.is_null() {
                        let Some(new_object) =
                            (unsafe { vm_object::allocate(original_size) })
                        else {
                            return Err(Error::ResourceShortage);
                        };
                        batch.new_object = new_object.as_ptr();
                        batch.new_offset = 0;
                        // The paging reference keeps the object alive, so
                        // these unlock without the lock as the C did.
                        // SAFETY: the page is busy and the object live.
                        batch.paging_offset = unsafe {
                            (*page).offset + (*object).paging_offset
                        };
                        pageout_action = result;
                    }

                    // SAFETY: the page is live and busy, the batch's object
                    // is live, and the object lock is released as the C's
                    // `vm_pageout_setup()` contract has it.
                    let new_page = unsafe {
                        vm_pageout_setup(
                            page,
                            (*page).offset + (*object).paging_offset,
                            batch.new_object,
                            batch.new_offset,
                            c_int::from(should_flush),
                        )
                    };

                    // The offset is below `DATA_WRITE_MAX` pages, which the
                    // flush above the lookup loop enforced.
                    batch.holding[atop(batch.new_offset)] = new_page;
                    batch.new_offset =
                        batch.new_offset.wrapping_add(PAGE_SIZE);
                    last_offset = offset.wrapping_add(PAGE_SIZE);

                    // SAFETY: the object is live and held by the paging
                    // reference.
                    unsafe { (*object).lock.lock() };
                }
            }
            break;
        }

        size -= PAGE_SIZE;
        offset = offset.wrapping_add(PAGE_SIZE);
    }

    if !batch.new_object.is_null() {
        // SAFETY: the batch is live and the object lock is held.
        unsafe { batch.flush(object, should_flush) };
    }

    if IpcPort::valid(reply_to).is_some() {
        // SAFETY: the object is live and locked; the reply routine consumes
        // the reply right, and the C re-locks around it.
        unsafe {
            (*object).lock.unlock();
            memory_object_lock_completed(
                reply_to,
                reply_to_type,
                (*object).pager_request,
                original_offset,
                original_size,
            );
            (*object).lock.lock();
        }
    }

    // SAFETY: the object is live and locked; the call consumes the caller's
    // reference.
    unsafe {
        vm_object::paging_end(object);
        (*object).lock.unlock();
        vm_object::deallocate(object);
    }

    Ok(())
}

/// `memory_object_data_supply()` in C: take the pages of a page-list copy
/// into the object.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate;
/// `vm_data_copy` must name a live page-list copy of `data_cnt` bytes;
/// `reply_to` must be `IP_NULL` or a live port.
pub(crate) unsafe fn data_supply(
    object: *mut VmObject,
    request: &SupplyRequest,
) -> Result<(), Error> {
    let SupplyRequest {
        offset,
        data,
        data_cnt,
        lock_value,
        precious,
        reply_to,
        reply_to_type,
    } = *request;

    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };
    if (lock_value.bits() & !VmProt::ALL.bits()) != 0 {
        // SAFETY: the caller owns the object reference.
        unsafe { vm_object::deallocate(object.as_ptr()) };
        return Err(Error::InvalidArgument);
    }
    if !data_cnt.is_multiple_of(PAGE_SIZE as c_uint) {
        // SAFETY: the caller owns the object reference.
        unsafe { vm_object::deallocate(object.as_ptr()) };
        return Err(Error::InvalidArgument);
    }

    let original_length = data_cnt;
    let original_offset = offset;
    // `vm_offset_t` and a pointer are the same width on both targets.
    // SAFETY: the caller promises `data` names a live page-list copy.
    let mut copy = unsafe { NonNull::new_unchecked(data as *mut VmMapCopy) };
    let orig_copy = copy;
    let mut page_list =
        unsafe { addr_of_mut!((*VmMapCopy::page_list(copy)).page_list) }
            .cast::<*mut VmPage>();

    // SAFETY: the caller promises the live object; the reference the call
    // consumes keeps it alive.
    unsafe {
        (*object.as_ptr()).lock.lock();
        vm_object::paging_begin(object.as_ptr());
    }
    // SAFETY: the object is live and locked.
    let mut offset =
        offset.wrapping_sub(unsafe { (*object.as_ptr()).paging_offset });
    let mut data_cnt = data_cnt;
    let mut result: Result<(), Error> = Ok(());
    let mut error_offset: VmOffset = 0;

    while data_cnt != 0 {
        // SAFETY: the caller promises the copy; its page list holds this
        // entry.
        let data_m = unsafe { *page_list };

        // SAFETY: the entry is a live page of the copy.
        let bad = unsafe {
            data_m.is_null()
                || (*data_m).is_tabled()
                || (*data_m).is_error()
                || (*data_m).is_absent()
                || (*data_m).is_fictitious()
        };
        if bad {
            die("memory_object_data_supply", "Data_supply: bad page");
        }

        // SAFETY: the object is live and locked, as `lookup` requires.
        let target = loop {
            // SAFETY: as above.
            let Some(page) = (unsafe { vm_resident::lookup(object, offset) })
            else {
                break None;
            };
            let page = page.as_ptr();

            // SAFETY: the target page is live and the object lock is held.
            let absent_busy =
                unsafe { (*page).is_absent() && (*page).is_busy() };
            if absent_busy {
                // SAFETY: the page is live; `VM_PAGE_FREE` takes the queue
                // lock and keeps the object lock the caller holds.
                unsafe { page_free(page) };
                break Some(true);
            }

            // SAFETY: the page is live and the object lock is held.
            if unsafe { (*page).is_busy() } {
                // SAFETY: the page is live and the object lock is held, as
                // `PAGE_ASSERT_WAIT` requires.
                unsafe {
                    (*page).set_wanted(true);
                    assert_wait(page.cast(), 0);
                    (*object.as_ptr()).lock.unlock();
                    thread_block(None);
                    (*object.as_ptr()).lock.lock();
                }
                continue;
            }

            result = Err(Error::MemoryPresent);
            error_offset = offset
                .wrapping_add(unsafe { (*object.as_ptr()).paging_offset });
            break None;
        };

        let was_absent = match target {
            None => {
                if result.is_err() {
                    break;
                }
                false
            }
            Some(was_absent) => was_absent,
        };

        // SAFETY: the entry is a live page of the copy, and the object lock
        // is held.
        unsafe {
            (*data_m).set_busy(false);
            (*data_m).set_dirty(false);
            pmap_clear_modify((*data_m).phys_addr);
            (*data_m).set_page_lock(lock_value);
            (*data_m).set_unlock_request(VmProt::NONE);
            (*data_m).set_precious(precious);

            (*addr_of_mut!(vm_page_queue_lock)).lock();
            vm_resident::insert(
                NonNull::new_unchecked(data_m),
                object,
                offset,
            );
            if was_absent {
                vm_page::activate(data_m);
            } else {
                vm_page::deactivate(data_m);
            }
            (*addr_of_mut!(vm_page_queue_lock)).unlock();

            *page_list = null_mut();
            page_list = page_list.add(1);
        }

        // SAFETY: the copy is live and holds the page list.
        let pages = unsafe { VmMapCopy::page_list(copy) };
        // SAFETY: `pages` names the live page-list variant.
        unsafe { (*pages).npages -= 1 };
        // SAFETY: as above.
        let exhausted = unsafe { (*pages).npages == 0 };
        // SAFETY: the copy may hold a continuation.
        let has_cont = unsafe { VmMapCopy::has_cont(copy) };

        if exhausted && has_cont {
            // SAFETY: the object lock is held, and the continuation runs
            // with it released, as the C did.
            unsafe { (*object.as_ptr()).lock.unlock() };

            // SAFETY: the copy is live and owned by this call.
            let (code, new_copy) = unsafe { VmMapCopy::invoke_cont(copy) };

            match error_from_kern_return(code) {
                Ok(()) => {
                    if copy != orig_copy {
                        // SAFETY: the copy is live and was replaced.
                        unsafe { VmMapCopy::discard(copy) };
                    }

                    let Some(new_copy) = NonNull::new(new_copy) else {
                        // The C's continuation never returns a null copy
                        // together with success; stop rather than follow a
                        // freed list.
                        error_offset =
                            offset.wrapping_add(PAGE_SIZE).wrapping_add(
                                unsafe { (*object.as_ptr()).paging_offset },
                            );
                        result = Err(Error::Failure);
                        break;
                    };
                    copy = new_copy;
                    // SAFETY: the new copy is a live page-list copy.
                    page_list = unsafe {
                        addr_of_mut!((*VmMapCopy::page_list(copy)).page_list)
                    }
                    .cast::<*mut VmPage>();

                    // SAFETY: the object is live and held by the paging
                    // reference.
                    unsafe { (*object.as_ptr()).lock.lock() };
                }
                Err(error) => {
                    // SAFETY: the object is live and held by the paging
                    // reference.
                    unsafe { (*object.as_ptr()).lock.lock() };
                    error_offset =
                        offset.wrapping_add(PAGE_SIZE).wrapping_add(unsafe {
                            (*object.as_ptr()).paging_offset
                        });
                    result = Err(error);
                    break;
                }
            }
        }

        data_cnt -= PAGE_SIZE as c_uint;
        offset = offset.wrapping_add(PAGE_SIZE);
    }

    // SAFETY: the object is live and locked; the paging reference is the
    // caller's.
    unsafe {
        vm_object::paging_end(object.as_ptr());
        (*object.as_ptr()).lock.unlock();
    }

    // SAFETY: a page-list copy may hold a continuation.
    if unsafe { VmMapCopy::has_cont(copy) } {
        // SAFETY: the copy is live and owned by this call.
        unsafe { VmMapCopy::abort_cont(copy) };
    }

    if IpcPort::valid(reply_to).is_some() {
        // SAFETY: the object is live under the caller's reference, and the
        // C sends the reply before releasing it.
        unsafe {
            memory_object_supply_completed(
                reply_to,
                reply_to_type,
                (*object.as_ptr()).pager_request,
                original_offset,
                // `original_length` is a `mach_msg_type_number_t` byte
                // count; `vm_size_t` holds every value it can name.
                original_length as VmSize,
                match result {
                    Ok(()) => 0,
                    Err(error) => error.as_kern_return(),
                },
                error_offset,
            );
        }
    }

    // SAFETY: the call consumes the caller's reference.
    unsafe { vm_object::deallocate(object.as_ptr()) };

    if copy != orig_copy {
        // SAFETY: the copy is live and was replaced.
        unsafe { VmMapCopy::discard(copy) };
    }
    if result.is_ok() {
        // SAFETY: the original copy is live and owned by this call.
        unsafe { VmMapCopy::discard(orig_copy) };
    }

    result
}

/// `memory_object_data_error()` in C: mark the waiting absent pages of a
/// range as failed.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
pub(crate) unsafe fn data_error(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };
    if size != round_page(size) {
        return Err(Error::InvalidArgument);
    }

    // SAFETY: the caller promises the live object; the reference the call
    // consumes keeps it alive.
    unsafe { (*object.as_ptr()).lock.lock() };
    // SAFETY: the object is live and locked.
    let mut offset =
        offset.wrapping_sub(unsafe { (*object.as_ptr()).paging_offset });
    let mut size = size;

    while size != 0 {
        // SAFETY: the object is live and locked.
        if let Some(page) = unsafe { vm_resident::lookup(object, offset) } {
            let page = page.as_ptr();
            // SAFETY: the page is live and the object lock is held.
            let waiting = unsafe { (*page).is_busy() && (*page).is_absent() };
            if waiting {
                // SAFETY: the page is live and the object lock is held.
                unsafe {
                    (*page).set_error(true);
                    (*page).set_absent(false);
                    vm_object::absent_release(object.as_ptr());
                    vm_object::page_wakeup_done(page);

                    (*addr_of_mut!(vm_page_queue_lock)).lock();
                    vm_page::activate(page);
                    (*addr_of_mut!(vm_page_queue_lock)).unlock();
                }
            }
        }

        size -= PAGE_SIZE;
        offset = offset.wrapping_add(PAGE_SIZE);
    }

    // SAFETY: the object is live and locked; the call consumes the caller's
    // reference.
    unsafe {
        (*object.as_ptr()).lock.unlock();
        vm_object::deallocate(object.as_ptr());
    }

    Ok(())
}

/// `memory_object_data_unavailable()` in C: clear the waiting absent pages of
/// a range without providing data.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
pub(crate) unsafe fn data_unavailable(
    object: *mut VmObject,
    offset: VmOffset,
    size: VmSize,
) -> Result<(), Error> {
    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };
    if size != round_page(size) {
        return Err(Error::InvalidArgument);
    }

    let mut existence_info: *mut c_void = null_mut();
    // SAFETY: the object is live under the caller's reference.
    let needs_map = offset == 0
        && size > VM_EXTERNAL_LARGE_SIZE
        && unsafe { (*object.as_ptr()).existence_info }.is_null();
    if needs_map {
        // SAFETY: the external module's caches are initialized before any
        // object exists.
        existence_info =
            unsafe { vm_external::vm_external_create(VM_EXTERNAL_SMALL_SIZE) }
                .cast::<c_void>();
    }

    // SAFETY: the caller promises the live object.
    unsafe { (*object.as_ptr()).lock.lock() };
    if !existence_info.is_null() {
        // SAFETY: the object is live and locked.
        unsafe { (*object.as_ptr()).existence_info = existence_info };
    }
    if offset == 0 && size > VM_EXTERNAL_LARGE_SIZE {
        // SAFETY: the object is live and locked; the call consumes the
        // caller's reference.
        unsafe {
            (*object.as_ptr()).lock.unlock();
            vm_object::deallocate(object.as_ptr());
        }
        return Ok(());
    }
    // SAFETY: the object is live and locked.
    let mut offset =
        offset.wrapping_sub(unsafe { (*object.as_ptr()).paging_offset });
    let mut size = size;

    while size != 0 {
        // SAFETY: the object is live and locked.
        if let Some(page) = unsafe { vm_resident::lookup(object, offset) } {
            let page = page.as_ptr();
            // SAFETY: the page is live and the object lock is held.
            let waiting = unsafe { (*page).is_busy() && (*page).is_absent() };
            if waiting {
                // SAFETY: the page is live and the object lock is held.
                unsafe {
                    vm_object::page_wakeup_done(page);

                    (*addr_of_mut!(vm_page_queue_lock)).lock();
                    vm_page::activate(page);
                    (*addr_of_mut!(vm_page_queue_lock)).unlock();
                }
            }
        }

        size -= PAGE_SIZE;
        offset = offset.wrapping_add(PAGE_SIZE);
    }

    // SAFETY: the object is live and locked; the call consumes the caller's
    // reference.
    unsafe {
        (*object.as_ptr()).lock.unlock();
        vm_object::deallocate(object.as_ptr());
    }

    Ok(())
}

/// Types and functions for the default memory manager.
///
/// # Safety
///
/// See each function.
pub(crate) mod default_manager {
    use super::*;

    /// `vm_set_default_memory_manager()` in C: replace or fetch the default
    /// memory manager's port.
    ///
    /// # Safety
    ///
    /// A non-null `host` must be a live host, and `default_manager` must be
    /// writable for one port.
    pub(crate) unsafe fn set(
        host: *mut Host,
        default_manager: *mut *mut c_void,
    ) -> Result<(), Error> {
        if host.is_null() {
            return Err(Error::InvalidHost);
        }

        // SAFETY: the caller promises the out-slot is writable.
        let new_manager = unsafe { *default_manager };

        // SAFETY: the module's lock is not held.
        unsafe { (*addr_of_mut!(MEMORY_MANAGER_DEFAULT_LOCK)).lock() };
        // SAFETY: the global is read under its lock.
        let current = unsafe { memory_manager_default };
        let returned = if new_manager.is_null() {
            // SAFETY: the current manager is `IP_NULL` or a live port.
            unsafe { ipc_port::copy_send(current) }
        } else {
            // SAFETY: the global is written under its lock.
            unsafe { memory_manager_default = new_manager };

            // SAFETY: the wakeup event is the global's address, as the C
            // put it.
            unsafe {
                thread_wakeup_prim(
                    addr_of_mut!(memory_manager_default).cast(),
                    0,
                    THREAD_AWAKENED,
                );
            }
            current
        };
        // SAFETY: the lock was taken above.
        unsafe { (*addr_of_mut!(MEMORY_MANAGER_DEFAULT_LOCK)).unlock() };

        // SAFETY: the caller promises the out-slot is writable.
        unsafe { *default_manager = returned };
        Ok(())
    }

    /// `memory_manager_default_reference()` in C: a naked send right for the
    /// default memory manager, waiting until one exists.
    ///
    /// # Safety
    ///
    /// The caller must not hold the default-manager lock.
    pub(crate) unsafe fn reference() -> IpcPort {
        let lock = addr_of_mut!(MEMORY_MANAGER_DEFAULT_LOCK);

        // SAFETY: the caller does not hold the lock.
        unsafe { (*lock).lock() };
        loop {
            // SAFETY: the global is the module's, read under its lock.
            let current =
                unsafe { ipc_port::copy_send(memory_manager_default) };
            if let Some(port) = IpcPort::valid(current) {
                // SAFETY: the lock was taken above.
                unsafe { (*lock).unlock() };
                return port;
            }

            // SAFETY: `thread_sleep` releases the lock and blocks on the
            // global's address, as the C's call did.
            unsafe {
                thread_sleep(
                    addr_of_mut!(memory_manager_default).cast(),
                    lock,
                    0,
                );
                (*lock).lock();
            }
        }
    }

    /// `memory_manager_default_port()` in C: whether `port` receives for the
    /// default memory manager.
    ///
    /// # Safety
    ///
    /// `port` must be `IP_NULL` or a live port; the caller must not hold the
    /// default-manager lock.
    pub(crate) unsafe fn port(port: *mut c_void) -> bool {
        let lock = addr_of_mut!(MEMORY_MANAGER_DEFAULT_LOCK);

        // SAFETY: the caller does not hold the lock.
        unsafe { (*lock).lock() };
        // SAFETY: the global is read under its lock.
        let current = unsafe { memory_manager_default };
        let result = match (IpcPort::valid(port), IpcPort::valid(current)) {
            // SAFETY: both handles name live ports.
            (Some(port), Some(current)) => unsafe {
                port.receiver() == current.receiver()
            },
            _ => false,
        };
        // SAFETY: the lock was taken above.
        unsafe { (*lock).unlock() };

        result
    }

    /// `memory_manager_default_init()` in C.
    pub(crate) fn init() {
        // SAFETY: the bootstrap runs once, before any other user of the
        // global and its lock.
        unsafe {
            memory_manager_default = null_mut();
            (*addr_of_mut!(MEMORY_MANAGER_DEFAULT_LOCK)).init();
        }
    }
}

/// `memory_object_set_attributes_common()` in C: apply a manager's attribute
/// change.
///
/// # Safety
///
/// A non-null `object` must be a live object; the call consumes the caller's
/// reference.
pub(crate) unsafe fn set_attributes(
    object: *mut VmObject,
    may_cache: bool,
    copy_strategy: CopyStrategy,
) -> Result<(), Error> {
    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };

    // SAFETY: the caller promises the live object.
    unsafe { (*object.as_ptr()).lock.lock() };

    // SAFETY: the object is live and locked.
    unsafe {
        if !(*object.as_ptr()).is_pager_ready() {
            vm_object::wakeup_pager_ready(object.as_ptr());
        }

        (*object.as_ptr()).set_can_persist(may_cache);
        (*object.as_ptr()).set_pager_ready(true);
        if copy_strategy == CopyStrategy::Temporary {
            (*object.as_ptr()).set_temporary(true);
        } else {
            (*object.as_ptr()).copy_strategy = copy_strategy.as_c();
        }

        (*object.as_ptr()).lock.unlock();
        vm_object::deallocate(object.as_ptr());
    }

    Ok(())
}

/// `memory_object_change_attributes()` in C: apply the change and acknowledge
/// it.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate;
/// `reply_to` must be `IP_NULL` or a live port.
pub(crate) unsafe fn change_attributes(
    object: *mut VmObject,
    may_cache: bool,
    copy_strategy: c_int,
    reply_to: *mut c_void,
    reply_to_type: c_uint,
) -> Result<(), Error> {
    let result =
        match (NonNull::new(object), CopyStrategy::from_c(copy_strategy)) {
            (None, _) => Err(Error::InvalidArgument),
            // SAFETY: the caller's contract is `set_attributes`'.
            (Some(object), Some(strategy)) => unsafe {
                set_attributes(object.as_ptr(), may_cache, strategy)
            },
            (Some(object), None) => {
                // SAFETY: the C's invalid-strategy path released the reference.
                unsafe { vm_object::deallocate(object.as_ptr()) };
                Err(Error::InvalidArgument)
            }
        };

    if IpcPort::valid(reply_to).is_some() {
        // SAFETY: `reply_to` is a live port and the C sends the raw request
        // values back.
        unsafe {
            memory_object_change_completed(
                reply_to,
                reply_to_type,
                c_int::from(may_cache),
                copy_strategy,
            );
        }
    }

    result
}

/// `memory_object_ready()` in C: apply the manager's ready attributes.
///
/// # Safety
///
/// A non-null `object` must be a live object the call may deallocate.
pub(crate) unsafe fn ready(
    object: *mut VmObject,
    may_cache: bool,
    copy_strategy: c_int,
) -> Result<(), Error> {
    match (NonNull::new(object), CopyStrategy::from_c(copy_strategy)) {
        (None, _) => Err(Error::InvalidArgument),
        // SAFETY: the caller's contract is `set_attributes`'.
        (Some(object), Some(strategy)) => unsafe {
            set_attributes(object.as_ptr(), may_cache, strategy)
        },
        (Some(object), None) => {
            // SAFETY: the C's invalid-strategy path released the reference.
            unsafe { vm_object::deallocate(object.as_ptr()) };
            Err(Error::InvalidArgument)
        }
    }
}

/// `memory_object_get_attributes()` in C: read the object's pager state.
pub(crate) struct Attributes {
    /// `object_ready`: whether the manager has set the attributes.
    pub ready: bool,
    /// `may_cache`: whether the object may be cached.
    pub may_cache: bool,
    /// `copy_strategy`: the raw `memory_object_copy_strategy_t`.
    pub copy_strategy: c_int,
}

/// `memory_object_get_attributes()` in C: read the object's pager state.
///
/// # Safety
///
/// A non-null `object` must be a live object; the call consumes the caller's
/// reference.
pub(crate) unsafe fn get_attributes(
    object: *mut VmObject,
) -> Result<Attributes, Error> {
    let Some(object) = NonNull::new(object) else {
        return Err(Error::InvalidArgument);
    };

    // SAFETY: the caller promises the live object.
    unsafe { (*object.as_ptr()).lock.lock() };
    // SAFETY: the object is live and locked.
    let attributes = unsafe {
        Attributes {
            ready: (*object.as_ptr()).is_pager_ready(),
            may_cache: (*object.as_ptr()).can_persist(),
            copy_strategy: (*object.as_ptr()).copy_strategy,
        }
    };
    // SAFETY: the object is live and locked; the call consumes the caller's
    // reference.
    unsafe {
        (*object.as_ptr()).lock.unlock();
        vm_object::deallocate(object.as_ptr());
    }

    Ok(attributes)
}
