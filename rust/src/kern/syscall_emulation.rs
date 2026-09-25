// SPDX-License-Identifier: CMU-Mach
// Derived from kern/syscall_emulation.c and kern/syscall_emulation.h:
//   Copyright (c) 1991,1990,1989,1988,1987 Carnegie Mellon University.
//   Copyright (c) 1993,1994 The University of Utah and the Computer
//   Systems Laboratory (CSL).
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The user-space system call emulation module, which
//! `kern/syscall_emulation.h` declares and `kern/syscall_emulation.c` used
//! to define.

use crate::arch::types::{VmOffset, VmSize};
use crate::ipc::ipc_init;
use crate::kern::lock::SimpleLock;
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::Task;
use crate::vm::vm_kern::{kmem_alloc, kmem_free};
use crate::vm::vm_map::{VmMapCopy, round_page};
use core::ffi::c_int;
use core::mem::size_of;
use core::ptr::{self, NonNull};

/// `EML_BAD_TASK` of <kern/syscall_emulation.h>: the task is null.
const EML_BAD_TASK: c_int = 0x8001;

/// `KERN_RESOURCE_SHORTAGE` of <mach/kern_return.h>.
const KERN_RESOURCE_SHORTAGE: c_int = 3;

/// The `emulation_vector_t` of a task, as `task_get_emulation_vector()`
/// returns one.
pub(crate) struct EmulationVector {
    /// `*vector_start`.
    pub(crate) start: c_int,
    /// `*emulation_vector`: a `vm_map_copy_t` the server returns out of
    /// line.
    pub(crate) vector: *mut VmOffset,
    /// `*emulation_vector_count`.
    pub(crate) count: u32,
}

/// `struct eml_dispatch` of <kern/syscall_emulation.h>: one task's dispatch
/// table, whose `disp_vector` follows the header in one allocation.
///
/// The i386 and x86_64 `locore.S` syscall entries read `disp_min`,
/// `disp_count` and `disp_vector` at the offsets `i386asm.sym` derives from
/// the C header, so the field order here is the ABI.
#[repr(C)]
pub struct EmlDispatch {
    /// `lock`: protects `ref_count` only.
    lock: SimpleLock,
    ref_count: c_int,
    /// `disp_count`: the number of entries in the vector.
    disp_count: c_int,
    /// `disp_min`: the index of the vector's lowest entry.
    disp_min: c_int,
}

const _: () = {
    assert!(size_of::<EmlDispatch>() == 16);
    assert!(core::mem::offset_of!(EmlDispatch, lock) == 0);
    assert!(core::mem::offset_of!(EmlDispatch, ref_count) == 4);
    assert!(core::mem::offset_of!(EmlDispatch, disp_count) == 8);
    assert!(core::mem::offset_of!(EmlDispatch, disp_min) == 12);
};

/// `count_to_size()` of kern/syscall_emulation.c: the allocation size of a
/// dispatch table holding `count` entries.
const fn count_to_size(count: usize) -> usize {
    size_of::<EmlDispatch>() + size_of::<VmOffset>() * count
}

/// `eml->disp_vector` of <kern/syscall_emulation.h>.
///
/// # Safety
///
/// `eml` must point at a live dispatch table.
unsafe fn vector(eml: *mut EmlDispatch) -> *mut VmOffset {
    // SAFETY: every dispatch table allocates its vector in the same
    // allocation, right after the header.
    unsafe {
        eml.cast::<u8>()
            .add(size_of::<EmlDispatch>())
            .cast::<VmOffset>()
    }
}

/// `eml_task_reference()` in C: give `task` a reference to `parent`'s
/// emulation vector.
///
/// # Safety
///
/// `task` must be a live task, and `parent` null or a live task.
pub(crate) unsafe fn task_reference(task: *mut Task, parent: *mut Task) {
    // SAFETY: the caller promises the live tasks.
    let eml = unsafe {
        if parent.is_null() {
            ptr::null_mut()
        } else {
            (*parent).eml_dispatch
        }
    };

    if !eml.is_null() {
        // SAFETY: the caller's contract; the table's lock protects the
        // reference count.
        unsafe {
            (*eml).lock.lock();
            (*eml).ref_count = (*eml).ref_count.wrapping_add(1);
            (*eml).lock.unlock();
        }
    }

    // SAFETY: the caller promises the live task.
    unsafe { (*task).eml_dispatch = eml };
}

/// `eml_task_deallocate()` in C: drop one reference to a task's emulation
/// vector, freeing it with the last one.
///
/// # Safety
///
/// `task` must be a live task whose emulation vector belongs to the task
/// this call is deallocating.
pub(crate) unsafe fn task_deallocate(task: *mut Task) {
    // SAFETY: the caller promises the live task.
    let eml = unsafe { (*task).eml_dispatch };
    if eml.is_null() {
        return;
    }

    // SAFETY: the caller's contract; the table's lock protects the count.
    let count = unsafe {
        (*eml).lock.lock();
        let count = (*eml).ref_count.wrapping_sub(1);
        (*eml).ref_count = count;
        (*eml).lock.unlock();
        count
    };

    if count == 0 {
        // SAFETY: the count just reached zero, so nothing else holds the
        // table; its size follows from the count it kept.
        unsafe {
            kfree(
                NonNull::new_unchecked(eml.cast::<u8>()),
                count_to_size((*eml).disp_count as usize),
            )
        };
    }
}

/// `task_set_emulation_vector_internal()` of kern/syscall_emulation.c.
///
/// # Safety
///
/// `task` must be null or a live task, and `emulation_vector` must point at
/// `emulation_vector_count` readable entries.
pub(crate) unsafe fn set_vector_internal(
    task: *mut Task,
    vector_start: c_int,
    emulation_vector: *mut VmOffset,
    emulation_vector_count: u32,
) -> c_int {
    if task.is_null() {
        return EML_BAD_TASK;
    }

    // The C added the unsigned count to the `int` start and kept the
    // low word.
    let vector_end =
        vector_start.wrapping_add(emulation_vector_count as c_int);
    let mut cur_eml;
    // The vector to discard once the table is installed.
    let mut old_eml: *mut EmlDispatch = ptr::null_mut();
    // The table allocated outside the task lock.
    let mut new_eml: *mut EmlDispatch = ptr::null_mut();
    let mut new_start: c_int = 0;
    let mut new_end: c_int = 0;

    loop {
        // SAFETY: the caller promises the live task; the task lock protects
        // `eml_dispatch`, and the table lock only the reference count.
        unsafe {
            (*task).lock.lock();
            cur_eml = (*task).eml_dispatch;

            if !cur_eml.is_null() {
                let cur_start = (*cur_eml).disp_min;
                let cur_end = (*cur_eml).disp_count.wrapping_add(cur_start);

                (*cur_eml).lock.lock();
                if (*cur_eml).ref_count == 1
                    && cur_start <= vector_start
                    && cur_end >= vector_end
                {
                    // The existing vector can hold the new entries; any
                    // newly allocated one is discarded.
                    (*cur_eml).lock.unlock();
                    old_eml = new_eml;
                    break;
                }

                if !new_eml.is_null()
                    && new_start <= cur_start
                    && new_end >= cur_end
                {
                    // The new vector holds the old entries; copy them over
                    // and drop the old table's reference.
                    ptr::copy_nonoverlapping(
                        vector(cur_eml),
                        vector(new_eml).add((cur_start - new_start) as usize),
                        (*cur_eml).disp_count as usize,
                    );
                    (*cur_eml).ref_count =
                        (*cur_eml).ref_count.wrapping_sub(1);
                    if (*cur_eml).ref_count == 0 {
                        old_eml = cur_eml;
                    }
                    (*cur_eml).lock.unlock();

                    (*task).eml_dispatch = new_eml;
                    cur_eml = new_eml;
                    break;
                }
                (*cur_eml).lock.unlock();

                new_start = if vector_start < cur_start {
                    vector_start
                } else {
                    cur_start
                };
                new_end = if vector_end < cur_end {
                    cur_end
                } else {
                    vector_end
                };
            } else {
                if !new_eml.is_null() {
                    (*task).eml_dispatch = new_eml;
                    cur_eml = new_eml;
                    break;
                }

                new_start = vector_start;
                new_end = vector_end;
            }

            (*task).lock.unlock();

            if !new_eml.is_null() {
                kfree(
                    NonNull::new_unchecked(new_eml.cast::<u8>()),
                    count_to_size((*new_eml).disp_count as usize),
                );
            }
        }

        let new_size = count_to_size(new_end.wrapping_sub(new_start) as usize);
        // The C kalloc() returned zero here and then memset a null pointer;
        // an out-of-memory boot returns the error instead.
        let Some(buffer) = kalloc(new_size) else {
            return KERN_RESOURCE_SHORTAGE;
        };

        // SAFETY: the buffer is a fresh allocation of `new_size` bytes; the
        // table's lock starts unlocked and the fields are filled in before
        // the next task-lock round can see it.
        unsafe {
            ptr::write_bytes(buffer.as_ptr(), 0, new_size);
            let table = buffer.as_ptr().cast::<EmlDispatch>();
            (*table).lock.init();
            (*table).ref_count = 1;
            (*table).disp_min = new_start;
            (*table).disp_count = new_end.wrapping_sub(new_start);
            new_eml = table;
        }
    }

    // SAFETY: the task lock is held and the table was chosen above; the
    // caller promises the readable entries.
    unsafe {
        if emulation_vector_count != 0 {
            ptr::copy_nonoverlapping(
                emulation_vector,
                vector(cur_eml)
                    .add((vector_start - (*cur_eml).disp_min) as usize),
                emulation_vector_count as usize,
            );
        }
        (*task).lock.unlock();

        if !old_eml.is_null() {
            kfree(
                NonNull::new_unchecked(old_eml.cast::<u8>()),
                count_to_size((*old_eml).disp_count as usize),
            );
        }
    }

    0
}

/// `task_get_emulation_vector()` in C: copy a task's emulation vector into
/// an out-of-line `vm_map_copy_t`.
///
/// # Safety
///
/// `task` must be null or a live task.
pub(crate) unsafe fn get_vector(
    task: *mut Task,
) -> Result<EmulationVector, c_int> {
    if task.is_null() {
        return Err(EML_BAD_TASK);
    }

    let map = ipc_init::kernel_map();
    let mut addr: VmOffset = 0;
    let mut size: VmSize = 0;
    let mut vector_size;
    let mut eml;

    loop {
        // SAFETY: the caller promises the live task; the task lock protects
        // the table pointer and its contents.
        unsafe {
            (*task).lock.lock();
            eml = (*task).eml_dispatch;
            if eml.is_null() {
                (*task).lock.unlock();
                if addr != 0 {
                    let _ = kmem_free(&mut *map, addr, size);
                }
                return Ok(EmulationVector {
                    start: 0,
                    vector: ptr::null_mut(),
                    count: 0,
                });
            }

            vector_size = ((*eml).disp_count as usize)
                .wrapping_mul(size_of::<VmOffset>());
        }

        let size_needed = round_page(vector_size);
        if size_needed <= size {
            break;
        }

        // SAFETY: the task lock is not held; the caller promised the live
        // task and the map is the live kernel map.
        unsafe {
            (*task).lock.unlock();
            if size != 0 {
                let _ = kmem_free(&mut *map, addr, size);
            }
            size = size_needed;
            match kmem_alloc(NonNull::new_unchecked(map), size) {
                Ok(allocated) => addr = allocated,
                Err(_) => return Err(KERN_RESOURCE_SHORTAGE),
            }
        }
    }

    // SAFETY: the task lock is held, the table is live, and `addr` holds
    // `size` bytes, at least `vector_size` of them.
    unsafe {
        let start = (*eml).disp_min;
        let count = (*eml).disp_count as u32;
        if vector_size != 0 {
            ptr::copy_nonoverlapping(
                vector(eml),
                addr as *mut VmOffset,
                vector_size / size_of::<VmOffset>(),
            );
        }
        (*task).lock.unlock();

        let size_used = round_page(vector_size);
        if size_used != size {
            let _ = kmem_free(&mut *map, addr + size_used, size - size_used);
        }

        let size_left = size_used - vector_size;
        if size_left > 0 {
            ptr::write_bytes((addr + vector_size) as *mut u8, 0, size_left);
        }

        // The C ignored the copyin result and returned an uninitialized
        // pointer on failure; the kernel map's wired pages cannot fail it,
        // and the error is returned instead of a garbage vector.
        let memory = if vector_size == 0 {
            ptr::null_mut()
        } else {
            match (&mut *map).copyin(addr, vector_size, true) {
                Ok(copy) => copy.as_ptr(),
                Err(error) => return Err(error.as_kern_return()),
            }
        };

        Ok(EmulationVector {
            start,
            vector: memory.cast(),
            count,
        })
    }
}

/// The `task_set_emulation_vector()` body of kern/syscall_emulation.c: map
/// the out-of-line vector into the kernel map, install it, and free the
/// mapping.
///
/// # Safety
///
/// `task` must be null or a live task, and `emulation_vector` a live
/// `vm_map_copy_t` or null.
pub(crate) unsafe fn set_vector(
    task: *mut Task,
    vector_start: c_int,
    emulation_vector: *mut VmOffset,
    emulation_vector_count: u32,
) -> c_int {
    if task.is_null() {
        return EML_BAD_TASK;
    }

    let map = ipc_init::kernel_map();

    let addr = match NonNull::new(emulation_vector.cast::<VmMapCopy>()) {
        // SAFETY: the caller promises the live copy and the map is live and
        // unlocked.
        Some(copy) => match unsafe { (&mut *map).copyout(copy) } {
            Ok(addr) => addr,
            Err(error) => return error.as_kern_return(),
        },
        None => 0,
    };

    // SAFETY: `addr` is where the copyout placed the caller's vector, which
    // has `emulation_vector_count` entries.
    let kr = unsafe {
        set_vector_internal(
            task,
            vector_start,
            addr as *mut VmOffset,
            emulation_vector_count,
        )
    };

    // SAFETY: the region came from the copyout above and is still mapped.
    unsafe {
        let _ = kmem_free(
            &mut *map,
            addr,
            (emulation_vector_count as usize)
                .wrapping_mul(size_of::<VmOffset>()),
        );
    }

    kr
}
