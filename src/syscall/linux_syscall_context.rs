use crate::kernel_lowlevel::thread::{self, Aarch64ExceptionFrame};
use crate::kernel_objects::scheduler;

use super::{SysError, SysResult};

include!("linux_syscall_context_logic_shared.rs");

static FRAME_OWNERS: LinuxSyscallFrameOwners<{ thread::MAX_THREADS }> =
    LinuxSyscallFrameOwners::new();

#[derive(Clone, Copy)]
pub(crate) struct LinuxSyscallFrameRef {
    pub frame: *mut Aarch64ExceptionFrame,
    pub return_pc: u64,
    pub pstate: u64,
}

struct InstalledFrame {
    owner: usize,
    frame: usize,
}

impl Drop for InstalledFrame {
    fn drop(&mut self) {
        let _ = FRAME_OWNERS.clear(self.owner, self.frame);
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
    let owner = scheduler::scheduler().current().0;
    if !FRAME_OWNERS.install(owner, frame as usize, return_pc as usize, pstate as usize) {
        return Err(SysError::EINVAL);
    }

    let _installed = InstalledFrame {
        owner,
        frame: frame as usize,
    };
    dispatch()
}

pub(crate) fn current() -> Option<LinuxSyscallFrameRef> {
    let owner = scheduler::scheduler().current().0;
    let context = FRAME_OWNERS.current(owner)?;
    Some(LinuxSyscallFrameRef {
        frame: context.frame as *mut Aarch64ExceptionFrame,
        return_pc: context.return_pc as u64,
        pstate: context.pstate as u64,
    })
}

pub(crate) fn reset() {
    FRAME_OWNERS.clear_all();
}
