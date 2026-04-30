//! System Call Handler
//!
//! This module handles system calls from EL0 (user mode) processes.
//! When an EL0 process executes an SVC instruction, control transfers to EL1
//! where this handler dispatches the syscall to the appropriate implementation.

use crate::syscall::{
    syscall::{dispatch_linux_syscall, dispatch_zircon_syscall, SysError},
    syscall_bridge::{
        is_linux_syscall_number, linux_args_from_regs, linux_sys_result_to_u64,
        saved_regs_ptr_from_el0_sp, syscall_num_from_regs, zircon_args_from_regs,
    },
};

/// Handle SVC exception from EL0 - called from assembly exception handler
///
/// This function is called when an EL0 process executes an SVC instruction.
/// The assembly code saves all registers on the stack, and this function
/// processes the syscall and updates x0 with the result.
///
/// # Safety
/// This function is called from assembly exception handler with proper context
#[no_mangle]
pub unsafe extern "C" fn handle_svc_exception_from_el0(
    _esr_el1: u64,
    elr_el1: u64,
    _spsr_el1: u64,
    sp_el0: u64,
) {
    // Read saved registers from stack
    // x0 is at sp + 0, x1 at sp + 8, etc.
    // x8 is at sp + 64 (8th pair, first element)
    let saved_regs = saved_regs_ptr_from_el0_sp(sp_el0);
    let regs = &*saved_regs;

    let syscall_num = syscall_num_from_regs(regs);

    // Extract arguments (x0-x5 for Linux, x0-x7 for Zircon)
    let args_linux = linux_args_from_regs(regs);
    let args_zircon = zircon_args_from_regs(regs);

    // Dispatch syscall
    let result = if is_linux_syscall_number(syscall_num) {
        // Linux syscall
        dispatch_linux_syscall(syscall_num as u32, args_linux)
    } else if syscall_num <= u32::MAX as u64 {
        // Zircon syscall - map error to Linux for now
        match dispatch_zircon_syscall(syscall_num as u32, args_zircon) {
            Ok(val) => Ok(val),
            Err(_) => Err(SysError::ENOSYS),
        }
    } else {
        Err(SysError::ENOSYS)
    };

    // Store result - will be loaded by assembly
    // For now, we need a different approach - modify stack directly
    // This is complex, so let's use a simpler approach with global
    SYSCALL_RESULT.store(
        linux_sys_result_to_u64(result),
        core::sync::atomic::Ordering::Relaxed,
    );

    // Advance ELR past the SVC instruction
    core::arch::asm!(
        "msr elr_el1, {elr}",
        elr = in(reg) elr_el1 + 4,
        options(nostack),
    );
}

/// Global to store syscall result temporarily
static SYSCALL_RESULT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Get syscall result (called by assembly)
#[no_mangle]
pub extern "C" fn get_syscall_result() -> u64 {
    SYSCALL_RESULT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Initialize syscall handler
pub fn init() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("[SYSCALL] Syscall handler initialized\n");
}
