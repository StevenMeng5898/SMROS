// Pure kernel-heap admission checks.
//
// The 256 MiB kernel heap is finite. User-triggered allocations must be
// refused while enough bytes remain for the shell and for infallible kernel
// bookkeeping. These functions do not allocate.

pub const KERNEL_BUDGET_HEAP_BYTES: usize = 0x1000_0000;
/// Bytes held back from user admission so a later shell or cleanup allocation
/// still succeeds after clone/mmap has been refused.
pub const KERNEL_BUDGET_RESERVE_BYTES: usize = 24 * 1024 * 1024;
/// Per-process cap on kernel memory charged to that process (stacks, address
/// space metadata, and mmap metadata).
pub const KERNEL_BUDGET_PROCESS_LIMIT_BYTES: usize = 192 * 1024 * 1024;
pub const KERNEL_BUDGET_ADDRESS_SPACE_BYTES: usize = 16 * 1024;
pub const KERNEL_BUDGET_OBJECT_BYTES: usize = 4 * 1024;
pub const KERNEL_BUDGET_MMAP_PER_PAGE_BYTES: usize = 64;

/// `true` when `extra` bytes can be taken from the heap without crossing the
/// reserve that is kept for the kernel and the shell.
pub fn admit_heap(allocated: usize, heap: usize, reserve: usize, extra: usize) -> bool {
    if extra == 0 {
        return allocated.saturating_add(reserve) <= heap || heap == 0;
    }
    match allocated.checked_add(extra).and_then(|sum| sum.checked_add(reserve)) {
        Some(need) => need <= heap,
        None => false,
    }
}

/// `true` when charging `extra` to one process stays within its limit.
pub fn admit_process(current: usize, limit: usize, extra: usize) -> bool {
    if extra == 0 {
        return current <= limit;
    }
    match current.checked_add(extra) {
        Some(next) => next <= limit,
        None => false,
    }
}

pub fn mmap_metadata_bytes(pages: usize) -> Option<usize> {
    pages.checked_mul(KERNEL_BUDGET_MMAP_PER_PAGE_BYTES)
}

/// Release `bytes` from an account without wrapping.
pub fn uncharge(current: usize, bytes: usize) -> usize {
    current.saturating_sub(bytes)
}
