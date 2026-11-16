//! EL0 versus kernel-buffer access.
//!
//! An AArch64 SVC from a lower EL saves that exception's SPSR on the current
//! scheduler thread. Copies ask that per-thread SPSR whether this entry came
//! from EL0 (`M[3:2] == 0`). A blocking syscall keeps its own slot across a
//! context switch, and another thread cannot clear it. There is no process-wide
//! thread-id slot.
//!
//! EL1 callers (the diagnostic shell, drivers, and in-kernel tests) do not
//! install an EL0 SPSR, so they can still pass their own kernel buffers.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel_lowlevel::thread::MAX_THREADS;
use crate::kernel_objects::scheduler;

fn saved_spsr_is_el0(spsr: u64) -> bool {
    // SPSR.M[3:2] is the target exception level. EL0 is 0. aarch64 uses the
    // shared helper so this cannot drift from exception dispatch.
    #[cfg(target_arch = "aarch64")]
    {
        crate::kernel_lowlevel::aarch64_exception_logic_shared::aarch64_saved_state_is_el0(spsr)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        spsr & 0xc == 0
    }
}

/// No exception entry is recorded for this scheduler thread.
const NO_ENTRY_SPSR: usize = usize::MAX;

static ENTRY_SPSR: [AtomicUsize; MAX_THREADS] =
    [const { AtomicUsize::new(NO_ENTRY_SPSR) }; MAX_THREADS];

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) struct El0SyscallGuard {
    owner: usize,
    previous: usize,
}

impl El0SyscallGuard {
    /// Record the hardware exception-return SPSR for the current thread.
    ///
    /// Call this from the lower-EL vector, before the thread can be scheduled
    /// out. The value is the SPSR captured for this entry, not a shared flag.
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    pub(crate) fn enter() -> Self {
        Self::install(crate::kernel_lowlevel::cpu::read_exception_return_state())
    }

    /// Record an explicit SPSR for this thread. Tests use `0` (EL0t) when they
    /// are not running inside the exception vector.
    pub(crate) fn enter_saved_spsr(spsr: u64) -> Self {
        Self::install(spsr)
    }

    fn install(spsr: u64) -> Self {
        let owner = scheduler::scheduler().current().0;
        let previous = if let Some(slot) = ENTRY_SPSR.get(owner) {
            slot.swap(spsr as usize, Ordering::AcqRel)
        } else {
            NO_ENTRY_SPSR
        };
        Self { owner, previous }
    }
}

impl Drop for El0SyscallGuard {
    fn drop(&mut self) {
        if let Some(slot) = ENTRY_SPSR.get(self.owner) {
            slot.store(self.previous, Ordering::Release);
        }
    }
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) fn el0_syscall_active() -> bool {
    let owner = scheduler::scheduler().current().0;
    let Some(slot) = ENTRY_SPSR.get(owner) else {
        return false;
    };
    let spsr = slot.load(Ordering::Acquire);
    spsr != NO_ENTRY_SPSR && saved_spsr_is_el0(spsr as u64)
}

pub(crate) fn clear_el0_syscall() {
    clear_owner(scheduler::scheduler().current().0);
}

pub(crate) fn clear_owner(owner: usize) {
    if let Some(slot) = ENTRY_SPSR.get(owner) {
        slot.store(NO_ENTRY_SPSR, Ordering::Release);
    }
}

include!("uaccess_logic_shared.rs");
