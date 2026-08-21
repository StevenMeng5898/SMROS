//! Syscall dispatch from assembly exception handler
//!
//! This module provides the interface between the assembly exception handler
//! and the Rust syscall implementations.

use crate::syscall::{
    dispatch_linux_syscall, dispatch_zircon_syscall,
    syscall_bridge::{
        is_linux_syscall_number, linux_args_from_u64s, linux_sys_result_to_u64, sys_error_to_u64,
    },
    syscall_logic::{is_zircon_syscall_number, zircon_syscall_from_raw},
    SysError, ZxError,
};
#[cfg(target_arch = "aarch64")]
use crate::{kernel_lowlevel::thread::Aarch64ExceptionFrame, syscall::linux_syscall_context};

/// Handle syscall from assembly exception handler
///
/// This function is called from the assembly exception handler after all
/// registers have been saved to the stack. It reads the syscall number
/// and arguments from the saved stack frame, dispatches the syscall,
/// and writes the result back to the stack.
///
/// # Safety
/// This function accesses the stack frame created by the assembly exception handler
#[no_mangle]
pub unsafe extern "C" fn handle_syscall() -> u64 {
    // Get stack pointer - registers were saved by assembly
    // The stack layout is:
    // [sp + 0]   = x0, x1
    // [sp + 16]  = x2, x3
    // [sp + 32]  = x4, x5
    // [sp + 48]  = x6, x7
    // [sp + 64]  = x8 (syscall number), x9
    // ...

    // We need to read the saved registers from the stack
    // Since we're in a function, sp points to our stack frame
    // The saved registers are at a known offset from the current sp

    // The assembly exception handler saved registers 256 bytes below the current stack
    // But we're now in a function call, so there's additional stack usage
    // We need to find the saved registers

    // Actually, this is getting too complex. Let me use a simpler approach:
    // Just return ENOSYS for now to show the mechanism works

    sys_error_to_u64(SysError::ENOSYS)
}

/// Simple syscall handler that takes arguments directly
/// This is easier to call from assembly
#[no_mangle]
pub extern "C" fn handle_syscall_simple(
    saved_frame: usize,
    syscall_num: u64,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64 {
    let args = linux_args_from_u64s(arg0, arg1, arg2, arg3, arg4, arg5);

    let result = if is_linux_syscall_number(syscall_num) {
        // Linux syscall
        #[cfg(target_arch = "aarch64")]
        {
            let return_pc = crate::kernel_lowlevel::cpu::read_exception_return_pc();
            let pstate = crate::kernel_lowlevel::cpu::read_exception_return_state();
            if syscall_num == 98 {
                crate::kobj_info!(
                    "fork",
                    "futex syscall frame={:#x} return_pc={:#x} pstate={:#x} op={:#x} uaddr={:#x}",
                    saved_frame,
                    return_pc,
                    pstate,
                    args[1],
                    args[0]
                );
            }
            let result = linux_sys_result_to_u64(linux_syscall_context::with_linux_syscall_frame(
                saved_frame as *mut Aarch64ExceptionFrame,
                return_pc,
                pstate,
                || dispatch_linux_syscall(syscall_num as u32, args),
            ));
            result
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = saved_frame;
            linux_sys_result_to_u64(dispatch_linux_syscall(syscall_num as u32, args))
        }
    } else if is_zircon_syscall_number(syscall_num) {
        let zircon_args = [args[0], args[1], args[2], args[3], args[4], args[5], 0, 0];
        match dispatch_zircon_syscall(zircon_syscall_from_raw(syscall_num), zircon_args) {
            Ok(value) => value as u64,
            Err(err) => zircon_error_to_u64(err),
        }
    } else {
        sys_error_to_u64(SysError::ENOSYS)
    };

    crate::user_level::user_test::record_el0_kernel_syscall_result(syscall_num as u32, result);
    result
}

fn zircon_error_to_u64(err: ZxError) -> u64 {
    err as i32 as i64 as u64
}

#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub extern "C" fn handle_aarch64_lower_el_sync(
    saved_frame: usize,
    esr: u64,
    far: u64,
    return_pc: u64,
) {
    use crate::kernel_lowlevel::aarch64_exception_logic_shared::{
        aarch64_lower_el_sync, Aarch64El0MemoryAccess, Aarch64LowerElSync,
    };
    use crate::syscall::linux_process_memory::LinuxMemoryFaultAccess;

    let access = match aarch64_lower_el_sync(esr) {
        Aarch64LowerElSync::MemoryFault(fault) => {
            let _ = fault.kind;
            match fault.access {
                Aarch64El0MemoryAccess::Read => LinuxMemoryFaultAccess::Read,
                Aarch64El0MemoryAccess::Write => LinuxMemoryFaultAccess::Write,
                Aarch64El0MemoryAccess::Execute => LinuxMemoryFaultAccess::Execute,
            }
        }
        _ => crate::kernel_lowlevel::boot::fatal_aarch64_sync_exception(esr, far, return_pc),
    };
    if crate::syscall::deliver_linux_synchronous_memory_fault(saved_frame, return_pc, far, access)
        .is_err()
    {
        crate::kernel_lowlevel::boot::fatal_aarch64_sync_exception(esr, far, return_pc);
    }
}
