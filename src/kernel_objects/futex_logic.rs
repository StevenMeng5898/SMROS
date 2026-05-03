include!("futex_logic_shared.rs");

pub(crate) fn ptr_valid(ptr: usize, align: usize) -> bool {
    smros_futex_ptr_valid_body!(ptr, align)
}

pub(crate) fn value_matches(observed: i32, expected: i32) -> bool {
    smros_futex_value_matches_body!(observed, expected)
}

pub(crate) fn min_count(left: u32, right: u32) -> u32 {
    smros_futex_min_count_body!(left, right)
}

pub(crate) fn saturating_add(left: u32, right: u32) -> u32 {
    smros_futex_saturating_add_body!(left, right)
}
