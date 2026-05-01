macro_rules! smros_user_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_page_offset_body {
    ($base:expr, $page_index:expr, $page_size:expr) => {{
        match $page_index.checked_mul($page_size) {
            Some(offset) => smros_user_checked_end_body!($base, offset),
            None => None,
        }
    }};
}

macro_rules! smros_user_pfn_to_paddr_body {
    ($pfn:expr, $page_size:expr) => {{
        $pfn.checked_mul($page_size)
    }};
}

macro_rules! smros_user_stack_top_u64_body {
    ($stack_base:expr, $stack_size:expr) => {{
        $stack_base.checked_add($stack_size as u64)
    }};
}

macro_rules! smros_user_el0_thread_pstate_body {
    () => {
        0x3C0u64
    };
}

macro_rules! smros_user_el0_spsr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_el1h_spsr_masked_body {
    () => {
        0x3C5u64
    };
}

macro_rules! smros_user_syscall_should_advance_elr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_ascii_shell_input_body {
    ($byte:expr) => {{
        $byte >= 0x20 && $byte <= 0x7e
    }};
}

macro_rules! smros_user_decimal_digit_value_body {
    ($byte:expr) => {{
        if $byte >= 48u8 && $byte <= 57u8 {
            Some(($byte - 48u8) as usize)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_parse_digit_step_body {
    ($result:expr, $digit:expr) => {{
        match $result.checked_mul(10) {
            Some(scaled) => scaled.checked_add($digit),
            None => None,
        }
    }};
}

macro_rules! smros_user_saturating_sub_body {
    ($lhs:expr, $rhs:expr) => {{
        if $lhs >= $rhs {
            $lhs - $rhs
        } else {
            0
        }
    }};
}

macro_rules! smros_user_pages_to_kb_body {
    ($pages:expr, $page_size:expr) => {{
        match $pages.checked_mul($page_size) {
            Some(bytes) => bytes / 1024,
            None => usize::MAX,
        }
    }};
}

macro_rules! smros_user_usage_percent_body {
    ($used_pages:expr, $total_pages:expr) => {{
        if $total_pages == 0 {
            0
        } else {
            match $used_pages.checked_mul(100) {
                Some(scaled) => scaled / $total_pages,
                None => usize::MAX,
            }
        }
    }};
}

macro_rules! smros_user_uptime_parts_body {
    ($ticks:expr) => {{
        let seconds = $ticks / 100;
        let minutes = seconds / 60;
        let hours = minutes / 60;
        let days = hours / 24;
        (seconds, minutes, hours, days)
    }};
}

macro_rules! smros_user_mmap_result_ok_body {
    ($addr:expr, $page_size:expr, $base:expr, $limit:expr) => {{
        $page_size != 0 && $addr >= $base && $addr < $limit && $addr % $page_size == 0
    }};
}

macro_rules! smros_user_kernel_success_body {
    (
        $kernel_entered:expr,
        $kernel_finished:expr,
        $exit_code:expr,
        $kernel_write:expr,
        $kernel_pid:expr,
        $kernel_mmap:expr,
        $banner_len:expr
    ) => {{
        $kernel_entered
            && $kernel_finished
            && $exit_code == 0
            && $kernel_write == $banner_len as u64
            && $kernel_pid == 1
            && $kernel_mmap > 0
            && $kernel_mmap < 0xFFFF_FFFF_FFFF_F000u64
    }};
}
