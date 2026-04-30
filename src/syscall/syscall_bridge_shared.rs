macro_rules! smros_syscall_linux_threshold_u64 {
    () => {
        1000u64
    };
}

#[allow(unused_macros)]
macro_rules! smros_saved_reg_frame_bytes {
    () => {
        256usize
    };
}

macro_rules! smros_saved_reg_words {
    () => {
        32usize
    };
}

macro_rules! smros_saved_reg_count {
    () => {
        32usize
    };
}

macro_rules! smros_syscall_number_reg_index {
    () => {
        8usize
    };
}

#[allow(unused_macros)]
macro_rules! smros_syscall_arg_count_linux {
    () => {
        6usize
    };
}

#[allow(unused_macros)]
macro_rules! smros_syscall_arg_count_zircon {
    () => {
        8usize
    };
}

macro_rules! smros_is_linux_syscall_number_u64_body {
    ($syscall_num:expr) => {{
        $syscall_num < smros_syscall_linux_threshold_u64!()
    }};
}

#[allow(unused_macros)]
macro_rules! smros_saved_regs_ptr_from_el0_sp_body {
    ($sp_el0:expr) => {{
        ($sp_el0 as *const u64).wrapping_sub(smros_saved_reg_words!())
            as *const [u64; smros_saved_reg_count!()]
    }};
}

macro_rules! smros_syscall_num_from_regs_body {
    ($regs:expr) => {{
        $regs[smros_syscall_number_reg_index!()]
    }};
}

macro_rules! smros_syscall_arg_from_reg_body {
    ($regs:expr, $idx:expr) => {{
        $regs[$idx] as usize
    }};
}

macro_rules! smros_syscall_arg_from_u64_body {
    ($arg:expr) => {{
        $arg as usize
    }};
}

macro_rules! smros_linux_errno_to_u64_body {
    ($errno:expr) => {{
        if $errno == 0 {
            0
        } else {
            (u64::MAX - ($errno as u64)) + 1
        }
    }};
}
