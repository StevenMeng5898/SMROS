macro_rules! smros_futex_ptr_valid_body {
    ($ptr:expr, $align:expr) => {{
        $ptr != 0 && $align != 0 && $ptr % $align == 0
    }};
}

macro_rules! smros_futex_value_matches_body {
    ($observed:expr, $expected:expr) => {{
        $observed == $expected
    }};
}

macro_rules! smros_futex_min_count_body {
    ($left:expr, $right:expr) => {{
        if $left <= $right {
            $left
        } else {
            $right
        }
    }};
}

macro_rules! smros_futex_saturating_add_body {
    ($left:expr, $right:expr) => {{
        $left.saturating_add($right)
    }};
}
