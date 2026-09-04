use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel_lowlevel::thread::MAX_THREADS;
use crate::kernel_objects::scheduler;

/// Context of a RISC-V trap being dispatched by one scheduler thread. The
/// frame lives on the dedicated trap stack; per-thread slots keep a blocked
/// syscall's return state independent from another process on the hart.
#[derive(Clone, Copy)]
pub(crate) struct LinuxRiscvSyscallFrameRef {
    pub frame: *const u64,
    pub return_pc: u64,
    pub pstate: u64,
    pub user_sp: u64,
}

static FRAMES: [AtomicUsize; MAX_THREADS] = [const { AtomicUsize::new(0) }; MAX_THREADS];
static RETURN_PCS: [AtomicUsize; MAX_THREADS] = [const { AtomicUsize::new(0) }; MAX_THREADS];
static PSTATES: [AtomicUsize; MAX_THREADS] = [const { AtomicUsize::new(0) }; MAX_THREADS];
static USER_SPS: [AtomicUsize; MAX_THREADS] = [const { AtomicUsize::new(0) }; MAX_THREADS];

#[inline]
fn current_slot() -> usize {
    core::cmp::min(scheduler::scheduler().current().0, MAX_THREADS - 1)
}

pub(crate) fn install(saved_frame: usize) -> bool {
    if saved_frame == 0 || saved_frame & (core::mem::align_of::<u64>() - 1) != 0 {
        return false;
    }
    // Trap entry stores the CSRs in slots 30..32 before any scheduler switch.
    let frame_ptr = saved_frame as *mut u64;
    let saved_pc = unsafe { frame_ptr.add(30).read() };
    let return_pc = saved_pc.saturating_add(4);
    let pstate = unsafe { frame_ptr.add(32).read() };
    let user_sp = unsafe { frame_ptr.add(31).read() as usize };
    if user_sp == 0 {
        return false;
    }
    unsafe { frame_ptr.add(30).write(return_pc) };
    let slot = current_slot();
    RETURN_PCS[slot].store(return_pc as usize, Ordering::Relaxed);
    PSTATES[slot].store(pstate as usize, Ordering::Relaxed);
    USER_SPS[slot].store(user_sp, Ordering::Relaxed);
    FRAMES[slot].store(saved_frame, Ordering::Release);
    true
}

/// Install a trap frame for a synchronous user exception.  Unlike a syscall,
/// the saved `sepc` is the faulting instruction and must not be advanced.
pub(crate) fn install_fault(saved_frame: usize) -> bool {
    if saved_frame == 0 || saved_frame & (core::mem::align_of::<u64>() - 1) != 0 {
        return false;
    }
    let frame_ptr = saved_frame as *mut u64;
    let return_pc = unsafe { frame_ptr.add(30).read() };
    let pstate = unsafe { frame_ptr.add(32).read() };
    let user_sp = unsafe { frame_ptr.add(31).read() as usize };
    if user_sp == 0 {
        return false;
    }
    let slot = current_slot();
    RETURN_PCS[slot].store(return_pc as usize, Ordering::Relaxed);
    PSTATES[slot].store(pstate as usize, Ordering::Relaxed);
    USER_SPS[slot].store(user_sp, Ordering::Relaxed);
    FRAMES[slot].store(saved_frame, Ordering::Release);
    true
}

pub(crate) fn current_riscv_syscall_context() -> Option<LinuxRiscvSyscallFrameRef> {
    let slot = current_slot();
    let frame = FRAMES[slot].load(Ordering::Acquire);
    (frame != 0).then_some(LinuxRiscvSyscallFrameRef {
        frame: frame as *const u64,
        return_pc: RETURN_PCS[slot].load(Ordering::Relaxed) as u64,
        pstate: PSTATES[slot].load(Ordering::Relaxed) as u64,
        user_sp: USER_SPS[slot].load(Ordering::Relaxed) as u64,
    })
}

pub(crate) fn set_return_pc(return_pc: u64) -> bool {
    let slot = current_slot();
    let frame = FRAMES[slot].load(Ordering::Acquire);
    if frame == 0 {
        return false;
    }
    unsafe { (frame as *mut u64).add(30).write(return_pc) };
    RETURN_PCS[slot].store(return_pc as usize, Ordering::Release);
    true
}

pub(crate) fn set_return_state(return_state: u64) -> bool {
    let slot = current_slot();
    let frame = FRAMES[slot].load(Ordering::Acquire);
    if frame == 0 {
        return false;
    }
    unsafe { (frame as *mut u64).add(32).write(return_state) };
    PSTATES[slot].store(return_state as usize, Ordering::Release);
    true
}

pub(crate) fn set_return_stack_to_trap_top() -> bool {
    let slot = current_slot();
    let frame = FRAMES[slot].load(Ordering::Acquire);
    if frame == 0 {
        return false;
    }
    let trap_top = frame.saturating_add(272);
    unsafe { (frame as *mut u64).add(31).write(trap_top as u64) };
    USER_SPS[slot].store(trap_top, Ordering::Release);
    true
}

pub(crate) fn set_return_stack(stack: u64) -> bool {
    let slot = current_slot();
    let frame = FRAMES[slot].load(Ordering::Acquire);
    if frame == 0 || stack == 0 {
        return false;
    }
    unsafe { (frame as *mut u64).add(31).write(stack) };
    USER_SPS[slot].store(stack as usize, Ordering::Release);
    true
}

pub(crate) fn clear() {
    let slot = current_slot();
    FRAMES[slot].store(0, Ordering::Release);
    RETURN_PCS[slot].store(0, Ordering::Relaxed);
    PSTATES[slot].store(0, Ordering::Relaxed);
    USER_SPS[slot].store(0, Ordering::Relaxed);
}
