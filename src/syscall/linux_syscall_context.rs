use crate::kernel_lowlevel::thread::{self, Aarch64ExceptionFrame};
use crate::kernel_objects::scheduler;

use super::linux_task::{LinuxRestartBlock, LinuxRestartTimeout};
use super::{SysError, SysResult};

include!("linux_syscall_context_logic_shared.rs");

static FRAME_OWNERS: LinuxSyscallFrameOwners<{ thread::MAX_THREADS }> =
    LinuxSyscallFrameOwners::new();

const ARM64_SYS_FUTEX: u64 = 98;
const ARM64_SYS_NANOSLEEP: u64 = 101;
const ARM64_SYS_CLOCK_NANOSLEEP: u64 = 115;

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
    let frame_snapshot = unsafe { &*frame };
    let syscall_number = frame_snapshot.regs[8];
    let restartable = (syscall_number == ARM64_SYS_FUTEX
        && super::linux_futex::restartable_wait_operation(frame_snapshot.regs[1] as u32))
        || syscall_number == ARM64_SYS_NANOSLEEP
        || syscall_number == ARM64_SYS_CLOCK_NANOSLEEP;
    let restart = if restartable {
        return_pc
            .checked_sub(4)
            .map(|svc_address| LinuxRestartBlock {
                syscall_number,
                arguments: [
                    frame_snapshot.regs[0],
                    frame_snapshot.regs[1],
                    frame_snapshot.regs[2],
                    frame_snapshot.regs[3],
                    frame_snapshot.regs[4],
                    frame_snapshot.regs[5],
                ],
                svc_address,
                timeout: LinuxRestartTimeout::Unset,
            })
    } else {
        None
    };
    let owner = scheduler::scheduler().current().0;
    if !FRAME_OWNERS.install(owner, frame as usize, return_pc as usize, pstate as usize) {
        return Err(SysError::EINVAL);
    }

    let _installed = InstalledFrame {
        owner,
        frame: frame as usize,
    };
    let restart_installed = restart
        .map(|restart| super::linux_task::install_current_restart_block(restart).unwrap_or(false))
        .unwrap_or(false);
    let result = dispatch();
    if restart_installed && result != Err(SysError::EINTR) {
        let _ = super::linux_task::clear_current_restart_block();
    }
    result
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

pub(crate) fn retire_owner(owner: usize) {
    let _ = FRAME_OWNERS.clear_owner(owner);
}

pub(crate) fn reset() {
    FRAME_OWNERS.clear_all();
}
