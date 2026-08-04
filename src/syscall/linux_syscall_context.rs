use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel_lowlevel::thread::Aarch64ExceptionFrame;
use crate::kernel_objects::scheduler;

use super::{SysError, SysResult};

const INSTALLING: usize = 1;

static FRAMES: [AtomicUsize; scheduler::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_CPUS];
static RETURN_PCS: [AtomicUsize; scheduler::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_CPUS];
static PSTATES: [AtomicUsize; scheduler::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_CPUS];

#[derive(Clone, Copy)]
pub(crate) struct LinuxSyscallFrameRef {
    pub frame: *mut Aarch64ExceptionFrame,
    pub return_pc: u64,
    pub pstate: u64,
}

struct InstalledFrame {
    cpu: usize,
}

impl Drop for InstalledFrame {
    fn drop(&mut self) {
        RETURN_PCS[self.cpu].store(0, Ordering::Relaxed);
        PSTATES[self.cpu].store(0, Ordering::Relaxed);
        FRAMES[self.cpu].store(0, Ordering::Release);
    }
}

pub(crate) fn with_linux_syscall_frame(
    frame: *mut Aarch64ExceptionFrame,
    return_pc: u64,
    pstate: u64,
    dispatch: impl FnOnce() -> SysResult,
) -> SysResult {
    if frame.is_null() || (frame as usize) % core::mem::align_of::<Aarch64ExceptionFrame>() != 0 {
        return Err(SysError::EINVAL);
    }
    let cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;
    let Some(frame_slot) = FRAMES.get(cpu) else {
        return Err(SysError::EINVAL);
    };
    if frame_slot
        .compare_exchange(0, INSTALLING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(SysError::EINVAL);
    }

    let _installed = InstalledFrame { cpu };
    RETURN_PCS[cpu].store(return_pc as usize, Ordering::Relaxed);
    PSTATES[cpu].store(pstate as usize, Ordering::Relaxed);
    frame_slot.store(frame as usize, Ordering::Release);
    dispatch()
}

pub(crate) fn current() -> Option<LinuxSyscallFrameRef> {
    let cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;
    let frame = FRAMES.get(cpu)?.load(Ordering::Acquire);
    if frame <= INSTALLING {
        return None;
    }
    Some(LinuxSyscallFrameRef {
        frame: frame as *mut Aarch64ExceptionFrame,
        return_pc: RETURN_PCS[cpu].load(Ordering::Relaxed) as u64,
        pstate: PSTATES[cpu].load(Ordering::Relaxed) as u64,
    })
}
