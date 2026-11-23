/// Linux and Zircon syscall classification for the dispatcher tail.
///
/// `smros_linux_audited_stub_class_body` is the table previously inlined in
/// `linux_audited_stub`. Class codes: 0 none, 1 fail-closed ENOSYS, 2 fail-closed
/// EPERM, 3 harmless success, 4 personality. Unhandled numbers, known or not,
/// produce ENOSYS. Unhandled Zircon numbers are rejected.

#[allow(unused_macros)]
macro_rules! smros_linux_audited_stub_class_body {
    ($syscall_num:expr) => {{
        match $syscall_num {
            0 | 1 | 2 | 3 | 4 | 292 => 1u8,
            18 => 1u8,
            30 => 2u8,
            31 => 3u8,
            42 => 1u8,
            58 => 2u8,
            60 => 1u8,
            89 => 2u8,
            92 => 4u8,
            104 | 105 | 106 | 273 | 294 => 1u8,
            116 => 2u8,
            117 => 2u8,
            128 => 3u8,
            142 => 2u8,
            217 | 218 | 219 => 1u8,
            224 | 225 => 2u8,
            234 => 1u8,
            235 | 236 | 237 => 1u8,
            238 | 239 => 2u8,
            241 => 1u8,
            262 | 263 => 2u8,
            264 | 265 => 1u8,
            270 | 271 => 1u8,
            272 => 2u8,
            274 => 2u8,
            280 => 1u8,
            281 => 1u8,
            282 => 1u8,
            284 => 3u8,
            288 | 289 | 290 => 1u8,
            293 => 1u8,
            424 => 2u8,
            425 | 426 | 427 => 1u8,
            428 | 429 | 430 | 431 | 432 | 433 | 442 => 1u8,
            434 => 1u8,
            438 => 2u8,
            440 => 2u8,
            444 | 445 | 446 => 1u8,
            _ => 0u8,
        }
    }};
}

/// Errno for a stub-class code. `None` is the harmless success path.
/// Every other class, including "not a stub", is an error. Personality that
/// reaches the stub result path is EINVAL, matching `linux_audited_stub_result`.
#[allow(unused_macros)]
macro_rules! smros_linux_stub_errno_body {
    ($class:expr, $enosys:expr, $eperm:expr, $einval:expr) => {{
        match $class {
            3u8 => None,
            2u8 => Some($eperm),
            4u8 => Some($einval),
            _ => Some($enosys),
        }
    }};
}

/// Exact set of dispatcher arms this proof names.
///
/// Lifecycle: exit 93, exit_group 94, waitid 95, kill 129, clone 220, execve
/// 221, wait4 260, clone3 435, fork 1000, vfork 1001. Scheduler: 118-127,
/// setpriority 140, getpriority 141. Futex 98 stays because that arm is a real
/// handler. This is not the whole `dispatch_linux_syscall` match.
#[allow(unused_macros)]
macro_rules! smros_linux_critical_handler_body {
    ($syscall_num:expr) => {{
        $syscall_num == 93
            || $syscall_num == 94
            || $syscall_num == 95
            || $syscall_num == 98
            || $syscall_num == 118
            || $syscall_num == 119
            || $syscall_num == 120
            || $syscall_num == 121
            || $syscall_num == 122
            || $syscall_num == 123
            || $syscall_num == 124
            || $syscall_num == 125
            || $syscall_num == 126
            || $syscall_num == 127
            || $syscall_num == 129
            || $syscall_num == 140
            || $syscall_num == 141
            || $syscall_num == 220
            || $syscall_num == 221
            || $syscall_num == 260
            || $syscall_num == 435
            || $syscall_num == 1000
            || $syscall_num == 1001
    }};
}

/// 1 implemented, 2 fail-closed, 3 harmless, 4 personality, 5 known but
/// unimplemented, 6 unknown. Fail-closed wins over the critical-handler list.
#[allow(unused_macros)]
macro_rules! smros_linux_handler_class_body {
    (
        $syscall_num:expr,
        $implemented:expr,
        $fail_closed:expr,
        $harmless:expr,
        $personality:expr,
        $unimplemented:expr,
        $unknown:expr
    ) => {{
        match smros_linux_audited_stub_class_body!($syscall_num) {
            1u8 | 2u8 => $fail_closed,
            3u8 => $harmless,
            4u8 => $personality,
            _ => {
                if smros_linux_critical_handler_body!($syscall_num) {
                    $implemented
                } else if smros_linux_syscall_interface_known_body!($syscall_num) {
                    $unimplemented
                } else {
                    $unknown
                }
            }
        }
    }};
}

