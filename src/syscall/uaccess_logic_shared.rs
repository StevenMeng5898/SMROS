macro_rules! smros_uaccess_el0_range_body {
    ($ptr:expr, $len:expr, $user_base:expr, $user_limit:expr) => {{
        if $len == 0 {
            true
        } else {
            match $ptr.checked_add($len) {
                Some(end) => $ptr >= $user_base && end <= $user_limit,
                None => false,
            }
        }
    }};
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) fn el0_user_range_plausible(
    ptr: usize,
    len: usize,
    user_base: usize,
    user_limit: usize,
) -> bool {
    smros_uaccess_el0_range_body!(ptr, len, user_base, user_limit)
}
