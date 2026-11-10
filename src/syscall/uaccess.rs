//! EL0 versus kernel-buffer access.
//!
//! An AArch64 SVC from EL0 marks the current scheduler thread. While that mark
//! is set, user pointers must be translated through the process page tables.
//! EL1 callers (the diagnostic shell, drivers, and in-kernel tests) never set
//! the mark, so they can still pass their own kernel buffers.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel_objects::scheduler;

static EL0_SYSCALL_THREAD: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) struct El0SyscallGuard;

impl El0SyscallGuard {
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    pub(crate) fn enter() -> Self {
        EL0_SYSCALL_THREAD.store(scheduler::scheduler().current().0, Ordering::Relaxed);
        Self
    }
}

impl Drop for El0SyscallGuard {
    fn drop(&mut self) {
        EL0_SYSCALL_THREAD.store(usize::MAX, Ordering::Relaxed);
    }
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) fn el0_syscall_active() -> bool {
    let current = scheduler::scheduler().current().0;
    EL0_SYSCALL_THREAD.load(Ordering::Relaxed) == current && current != usize::MAX
}

pub(crate) fn clear_el0_syscall() {
    let current = scheduler::scheduler().current().0;
    if EL0_SYSCALL_THREAD.load(Ordering::Relaxed) == current {
        EL0_SYSCALL_THREAD.store(usize::MAX, Ordering::Relaxed);
    }
}

include!("uaccess_logic_shared.rs");
