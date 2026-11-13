//! Runtime kernel-heap watermark and per-process charge table.
//!
//! Thread stacks and address-space metadata are charged when they are
//! reserved and released when the thread slot or address space is dropped.
//! The global check reads the allocator's live byte counter so boot-time
//! heap use counts even though it is not attributed to a process.

use core::sync::atomic::{AtomicBool, Ordering};

use super::kernel_budget_logic_shared::{
    admit_heap, admit_process, mmap_metadata_bytes, uncharge, KERNEL_BUDGET_ADDRESS_SPACE_BYTES,
    KERNEL_BUDGET_HEAP_BYTES, KERNEL_BUDGET_OBJECT_BYTES, KERNEL_BUDGET_PROCESS_LIMIT_BYTES,
    KERNEL_BUDGET_RESERVE_BYTES,
};

const THREAD_SLOTS: usize = 2048;
const PROCESS_SLOTS: usize = 1024;

struct ThreadCharge {
    pid: usize,
    bytes: usize,
}

struct ProcessCharge {
    pid: usize,
    thread_bytes: usize,
    extra_bytes: usize,
}

struct Budget {
    threads: [ThreadCharge; THREAD_SLOTS],
    procs: [ProcessCharge; PROCESS_SLOTS],
}

static ENFORCE: AtomicBool = AtomicBool::new(false);
static LOCK: AtomicBool = AtomicBool::new(false);
static BUDGET: SyncBudget = SyncBudget(core::cell::UnsafeCell::new(Budget {
    threads: [const { ThreadCharge { pid: 0, bytes: 0 } }; THREAD_SLOTS],
    procs: [const { ProcessCharge { pid: 0, thread_bytes: 0, extra_bytes: 0 } }; PROCESS_SLOTS],
}));

struct SyncBudget(core::cell::UnsafeCell<Budget>);
unsafe impl Sync for SyncBudget {}

fn with_budget<R>(body: impl FnOnce(&mut Budget) -> R) -> R {
    let irq = super::cpu::mask_interrupts();
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let result = body(unsafe { &mut *BUDGET.0.get() });
    LOCK.store(false, Ordering::Release);
    super::cpu::restore_interrupts(irq);
    result
}

/// After this point, user-triggered stacks and other heap reservations must
/// leave `KERNEL_BUDGET_RESERVE_BYTES` free. Boot allocations stay infallible.
pub fn enable_user_budget() {
    ENFORCE.store(true, Ordering::Release);
}

pub fn user_budget_enabled() -> bool {
    ENFORCE.load(Ordering::Acquire)
}

pub fn heap_allocated() -> usize {
    crate::kernel_heap_allocated()
}

pub fn reserve_bytes() -> usize {
    KERNEL_BUDGET_RESERVE_BYTES
}

pub fn heap_limit_bytes() -> usize {
    KERNEL_BUDGET_HEAP_BYTES
}

pub fn address_space_bytes() -> usize {
    KERNEL_BUDGET_ADDRESS_SPACE_BYTES
}

pub fn object_bytes() -> usize {
    KERNEL_BUDGET_OBJECT_BYTES
}

pub fn mmap_metadata_cost(pages: usize) -> Option<usize> {
    mmap_metadata_bytes(pages)
}

pub fn heap_room_for(extra: usize) -> bool {
    if !user_budget_enabled() {
        return true;
    }
    admit_heap(
        heap_allocated(),
        KERNEL_BUDGET_HEAP_BYTES,
        KERNEL_BUDGET_RESERVE_BYTES,
        extra,
    )
}

fn process_slot<'a>(budget: &'a mut Budget, pid: usize) -> Option<&'a mut ProcessCharge> {
    if pid == 0 {
        return None;
    }
    if let Some(slot) = budget.procs.iter_mut().find(|slot| slot.pid == pid) {
        return Some(slot);
    }
    budget.procs.iter_mut().find(|slot| slot.pid == 0)
}

fn process_total(slot: &ProcessCharge) -> usize {
    slot.thread_bytes.saturating_add(slot.extra_bytes)
}

/// Reserve the heap watermark for a new kernel stack and remember `bytes` on
/// the scheduler slot. The process account is updated later by `bind_thread_pid`.
pub fn try_reserve_thread(index: usize, bytes: usize) -> bool {
    if index == 0 || index >= THREAD_SLOTS || bytes == 0 {
        return false;
    }
    if !heap_room_for(bytes) {
        return false;
    }
    with_budget(|budget| {
        if budget.threads[index].bytes != 0 {
            return false;
        }
        budget.threads[index] = ThreadCharge { pid: 0, bytes };
        true
    })
}

/// Attribute a reserved thread stack to `pid`. Fails with the process cap.
pub fn bind_thread_pid(index: usize, pid: usize) -> bool {
    if index == 0 || index >= THREAD_SLOTS || pid == 0 {
        return false;
    }
    with_budget(|budget| {
        let bytes = budget.threads[index].bytes;
        if bytes == 0 {
            return true;
        }
        if budget.threads[index].pid == pid {
            return true;
        }
        if budget.threads[index].pid != 0 {
            return false;
        }
        let Some(slot) = process_slot(budget, pid) else {
            return false;
        };
        if slot.pid == 0 {
            slot.pid = pid;
        }
        if !admit_process(process_total(slot), KERNEL_BUDGET_PROCESS_LIMIT_BYTES, bytes) {
            if slot.thread_bytes == 0 && slot.extra_bytes == 0 {
                slot.pid = 0;
            }
            return false;
        }
        slot.thread_bytes = slot.thread_bytes.saturating_add(bytes);
        budget.threads[index].pid = pid;
        true
    })
}

/// Drop the charge recorded for a scheduler slot. Safe to call twice.
pub fn discharge_thread(index: usize) {
    if index == 0 || index >= THREAD_SLOTS {
        return;
    }
    with_budget(|budget| {
        let ThreadCharge { pid, bytes } = budget.threads[index];
        budget.threads[index] = ThreadCharge { pid: 0, bytes: 0 };
        if pid == 0 || bytes == 0 {
            return;
        }
        if let Some(slot) = budget.procs.iter_mut().find(|slot| slot.pid == pid) {
            slot.thread_bytes = uncharge(slot.thread_bytes, bytes);
            if slot.thread_bytes == 0 && slot.extra_bytes == 0 {
                slot.pid = 0;
            }
        }
    });
}

/// Charge address-space or mmap metadata to `pid`.
pub fn try_account_process(pid: usize, bytes: usize) -> bool {
    if pid == 0 || bytes == 0 {
        return false;
    }
    if !admit_process(0, KERNEL_BUDGET_PROCESS_LIMIT_BYTES, bytes) {
        return false;
    }
    with_budget(|budget| {
        let Some(slot) = process_slot(budget, pid) else {
            return false;
        };
        if slot.pid == 0 {
            slot.pid = pid;
        }
        if !admit_process(process_total(slot), KERNEL_BUDGET_PROCESS_LIMIT_BYTES, bytes) {
            if slot.thread_bytes == 0 && slot.extra_bytes == 0 {
                slot.pid = 0;
            }
            return false;
        }
        slot.extra_bytes = slot.extra_bytes.saturating_add(bytes);
        true
    })
}

pub fn forget_process(pid: usize, bytes: usize) {
    if pid == 0 || bytes == 0 {
        return;
    }
    with_budget(|budget| {
        if let Some(slot) = budget.procs.iter_mut().find(|slot| slot.pid == pid) {
            slot.extra_bytes = uncharge(slot.extra_bytes, bytes);
            if slot.thread_bytes == 0 && slot.extra_bytes == 0 {
                slot.pid = 0;
            }
        }
    });
}

pub fn accounted_total() -> usize {
    with_budget(|budget| {
        budget
            .procs
            .iter()
            .map(process_total)
            .fold(0usize, usize::saturating_add)
    })
}
