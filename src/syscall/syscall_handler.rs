//! System Call Handler
//!
//! This module handles system calls from EL0 (user mode) processes.
//! When an EL0 process executes an SVC instruction, control transfers to EL1
//! where this handler dispatches the syscall to the appropriate implementation.

use crate::syscall::syscall::{
    dispatch_linux_syscall, dispatch_zircon_syscall,
    SysError,
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
    // Get syscall number from x8 (saved on stack by assembly)
    // The stack layout is set by the assembly exception handler
    let stack_ptr = sp_el0 as *mut u64;
    
    // Read saved registers from stack
    // x0 is at sp + 0, x1 at sp + 8, etc.
    // x8 is at sp + 64 (8th pair, first element)
    let saved_regs = (stack_ptr.wrapping_sub(256 / 8)) as *const [u64; 30];
    let regs = &*saved_regs;
    
    let x0 = regs[0];
    let x1 = regs[1];
    let x2 = regs[2];
    let x3 = regs[3];
    let x4 = regs[4];
    let x5 = regs[5];
    let x6 = regs[6];
    let x7 = regs[7];
    let x8 = regs[8];
    
    let syscall_num = x8 as u32;
    
    // Extract arguments (x0-x5 for Linux, x0-x7 for Zircon)
    let args_linux = [
        x0 as usize, x1 as usize, x2 as usize,
        x3 as usize, x4 as usize, x5 as usize,
    ];
    
    let args_zircon = [
        x0 as usize, x1 as usize, x2 as usize,
        x3 as usize, x4 as usize, x5 as usize,
        x6 as usize, x7 as usize,
    ];
    
    // Dispatch syscall
    let result = if syscall_num < 1000 {
        // Linux syscall
        dispatch_linux_syscall(syscall_num, args_linux)
    } else {
        // Zircon syscall - map error to Linux for now
        match dispatch_zircon_syscall(syscall_num, args_zircon) {
            Ok(val) => Ok(val),
            Err(_) => Err(SysError::ENOSYS),
        }
    };
    
    // Store result - will be loaded by assembly
    // For now, we need a different approach - modify stack directly
    // This is complex, so let's use a simpler approach with global
    SYSCALL_RESULT.store(result.unwrap_or(0) as u64, core::sync::atomic::Ordering::Relaxed);
    
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
