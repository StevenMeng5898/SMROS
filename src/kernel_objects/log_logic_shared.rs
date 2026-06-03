#[allow(unused_macros)]
macro_rules! smros_log_level_from_raw_body {
    (
        $raw:expr,
        $debug_raw:expr,
        $debug_level:expr,
        $warning_raw:expr,
        $warning_level:expr,
        $err_raw:expr,
        $err_level:expr,
        $fatal_raw:expr,
        $fatal_level:expr,
        $fallback_level:expr
    ) => {{
        if $raw == $debug_raw {
            $debug_level
        } else if $raw == $warning_raw {
            $warning_level
        } else if $raw == $err_raw {
            $err_level
        } else if $raw == $fatal_raw {
            $fatal_level
        } else {
            $fallback_level
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_log_should_log_body {
    ($level:expr, $threshold:expr) => {{
        $level >= $threshold
    }};
}

#[allow(unused_macros)]
macro_rules! smros_log_level_from_match_flags_body {
    (
        $debug_match:expr,
        $info_match:expr,
        $warning_match:expr,
        $warn_match:expr,
        $err_match:expr,
        $error_match:expr,
        $fatal_match:expr,
        $debug_level:expr,
        $info_level:expr,
        $warning_level:expr,
        $err_level:expr,
        $fatal_level:expr
    ) => {{
        if $debug_match {
            Some($debug_level)
        } else if $info_match {
            Some($info_level)
        } else if $warning_match || $warn_match {
            Some($warning_level)
        } else if $err_match || $error_match {
            Some($err_level)
        } else if $fatal_match {
            Some($fatal_level)
        } else {
            None
        }
    }};
}
