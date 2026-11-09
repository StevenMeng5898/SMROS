#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) fn el0_user_range_plausible(
    ptr: usize,
    len: usize,
    user_base: usize,
    user_limit: usize,
) -> bool {
    if len == 0 {
        return true;
    }
    match ptr.checked_add(len) {
        Some(end) => ptr >= user_base && end <= user_limit,
        None => false,
    }
}
