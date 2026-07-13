pub const HERMES_MAX_ARGS: usize = 8;
pub const HERMES_MAX_ARG_LEN: usize = 96;
pub const HERMES_MAX_ITERATIONS: usize = 64;
pub const HERMES_CAMPAIGN_CASES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesShellPolicy {
    Allowed,
    Forbidden,
    Invalid,
}

pub fn classify(command: &str, args: &[&str]) -> HermesShellPolicy {
    if command.is_empty()
        || command.len() > HERMES_MAX_ARG_LEN
        || args.len() > HERMES_MAX_ARGS
        || args.iter().any(|arg| arg.len() > HERMES_MAX_ARG_LEN)
    {
        return HermesShellPolicy::Invalid;
    }

    match command {
        "rm" | "kill" | "reboot" | "exit" | "clear" | "vi" | "run" | "write"
        | "mkdir" | "mv" | "cp" | "mount" | "cd" | "cd.." => HermesShellPolicy::Forbidden,
        "help" | "version" | "meminfo" | "components" | "fxfs" | "drivers" | "ifconfig"
        | "pwd" | "ls" | "svc" | "uptime" | "testsc" => no_args(args),
        "ps" => optional_exact_arg(args, "-a"),
        "top" => no_args(args),
        "sched" => sched_policy(args),
        "loglevel" => loglevel_policy(args),
        "echo" => HermesShellPolicy::Allowed,
        "fuzzsc" => fuzz_policy(args),
        "vm" => vm_policy(args),
        "docker" => docker_policy(args),
        "hermes" => hermes_policy(args),
        _ => HermesShellPolicy::Forbidden,
    }
}

fn no_args(args: &[&str]) -> HermesShellPolicy {
    if args.is_empty() {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Invalid
    }
}

fn optional_exact_arg(args: &[&str], value: &str) -> HermesShellPolicy {
    if args.is_empty() || args == [value] {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Invalid
    }
}

fn sched_policy(args: &[&str]) -> HermesShellPolicy {
    if args.is_empty() || args == ["status"] || args == ["trace"] {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Forbidden
    }
}

fn loglevel_policy(args: &[&str]) -> HermesShellPolicy {
    if args.is_empty() {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Forbidden
    }
}

fn vm_policy(args: &[&str]) -> HermesShellPolicy {
    if args == ["-s"] {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Forbidden
    }
}

fn docker_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        ["images"] | ["ps"] | ["ps", "-a"] => HermesShellPolicy::Allowed,
        ["inspect", id] | ["logs", id] if valid_identifier(id) => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Forbidden,
    }
}

fn hermes_policy(args: &[&str]) -> HermesShellPolicy {
    if args == ["test"] || args == ["info"] {
        HermesShellPolicy::Allowed
    } else {
        HermesShellPolicy::Forbidden
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn fuzz_policy(args: &[&str]) -> HermesShellPolicy {
    if args.is_empty() || args.len() > 3 {
        return HermesShellPolicy::Invalid;
    }

    for arg in args {
        let Some((key, value)) = arg.split_once('=') else {
            return HermesShellPolicy::Invalid;
        };
        let Some(number) = parse_decimal(value) else {
            return HermesShellPolicy::Invalid;
        };
        match key {
            "seed" => {}
            "iterations" if number > 0 && number <= 16 => {}
            "time" if number > 0 && number <= 5 => {}
            _ => return HermesShellPolicy::Invalid,
        }
    }
    HermesShellPolicy::Allowed
}

fn parse_decimal(value: &str) -> Option<u64> {
    if value.is_empty() {
        return None;
    }
    let mut number = 0u64;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        number = number.checked_mul(10)?.checked_add((byte - b'0') as u64)?;
    }
    Some(number)
}

pub fn campaign_iterations_valid(iterations: usize) -> bool {
    iterations > 0 && iterations <= HERMES_MAX_ITERATIONS
}

pub fn campaign_case_index(seed: u64, round: usize) -> usize {
    let mut state = seed ^ (round as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    next_random(&mut state) as usize % HERMES_CAMPAIGN_CASES
}

pub fn next_random(state: &mut u64) -> u64 {
    let mut value = if *state == 0 {
        0x9e37_79b9_7f4a_7c15
    } else {
        *state
    };
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *state = value;
    value
}
