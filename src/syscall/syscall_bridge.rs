include!("syscall_bridge_shared.rs");

use super::syscall::SysError;

pub(crate) const SMROS_SAVED_REG_COUNT: usize = smros_saved_reg_count!();

pub(crate) fn is_linux_syscall_number(syscall_num: u64) -> bool {
    smros_is_linux_syscall_number_u64_body!(syscall_num)
}

pub(crate) fn saved_regs_ptr_from_el0_sp(sp_el0: u64) -> *const [u64; SMROS_SAVED_REG_COUNT] {
    smros_saved_regs_ptr_from_el0_sp_body!(sp_el0)
}

pub(crate) fn syscall_num_from_regs(regs: &[u64; SMROS_SAVED_REG_COUNT]) -> u64 {
    smros_syscall_num_from_regs_body!(regs)
}

pub(crate) fn linux_args_from_regs(regs: &[u64; SMROS_SAVED_REG_COUNT]) -> [usize; 6] {
    [
        smros_syscall_arg_from_reg_body!(regs, 0),
        smros_syscall_arg_from_reg_body!(regs, 1),
        smros_syscall_arg_from_reg_body!(regs, 2),
        smros_syscall_arg_from_reg_body!(regs, 3),
        smros_syscall_arg_from_reg_body!(regs, 4),
        smros_syscall_arg_from_reg_body!(regs, 5),
    ]
}

pub(crate) fn zircon_args_from_regs(regs: &[u64; SMROS_SAVED_REG_COUNT]) -> [usize; 8] {
    [
        smros_syscall_arg_from_reg_body!(regs, 0),
        smros_syscall_arg_from_reg_body!(regs, 1),
        smros_syscall_arg_from_reg_body!(regs, 2),
        smros_syscall_arg_from_reg_body!(regs, 3),
        smros_syscall_arg_from_reg_body!(regs, 4),
        smros_syscall_arg_from_reg_body!(regs, 5),
        smros_syscall_arg_from_reg_body!(regs, 6),
        smros_syscall_arg_from_reg_body!(regs, 7),
    ]
}

pub(crate) fn linux_args_from_u64s(
    arg0: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> [usize; 6] {
    [
        smros_syscall_arg_from_u64_body!(arg0),
        smros_syscall_arg_from_u64_body!(arg1),
        smros_syscall_arg_from_u64_body!(arg2),
        smros_syscall_arg_from_u64_body!(arg3),
        smros_syscall_arg_from_u64_body!(arg4),
        smros_syscall_arg_from_u64_body!(arg5),
    ]
}

pub(crate) fn linux_errno_code_to_u64(errno: u32) -> u64 {
    smros_linux_errno_to_u64_body!(errno)
}

pub(crate) fn sys_error_to_u64(err: SysError) -> u64 {
    linux_errno_code_to_u64(err as u32)
}

pub(crate) fn linux_sys_result_to_u64(result: Result<usize, SysError>) -> u64 {
    match result {
        Ok(value) => value as u64,
        Err(err) => sys_error_to_u64(err),
    }
}
