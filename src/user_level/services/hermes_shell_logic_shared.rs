pub const HERMES_MAX_ARGS: usize = 8;
pub const HERMES_MAX_ARG_LEN: usize = 96;
pub const HERMES_CAMPAIGN_CASES: usize = 34;
pub const HERMES_SYSCALL_CAMPAIGN_CASES: usize = 14;
pub const HERMES_REPORT_DETAIL_LIMIT: usize = 64;
pub const HERMES_CAMPAIGN_FUZZSC_ITERATIONS: u64 = 50;
pub const HERMES_CAMPAIGN_FUZZSC_ITERATIONS_ARG: &str = "iterations=50";
pub const HERMES_CAMPAIGN_FUZZSC_TIME_SECONDS: u64 = 5;
pub const HERMES_CAMPAIGN_FUZZSC_TIME_ARG: &str = "time=5";
pub const HERMES_CAMPAIGN_VM_NAME: &str = "linux-demo";
pub const HERMES_CAMPAIGN_VM_XML: &str = "/shared/vm-demo.xml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesShellPolicy {
    Allowed,
    Forbidden,
    Invalid,
}

/// Shell verbs `hermes` accepts before the shared allowlist runs.
/// `test-all` and `random` are shell verbs. They are not `hermes exec`
/// targets, so a campaign cannot nest another campaign or start SKT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesShellVerb {
    Exec,
    Random,
    TestAll,
    Info,
    Test,
    Skills,
    Web,
    Ask,
    Usage,
}

/// Names printed by `help`, in shell-table order.
pub const SHELL_COMMAND_NAMES: &[&str] = &[
    "help",
    "version",
    "ps",
    "top",
    "meminfo",
    "components",
    "fxfs",
    "drivers",
    "ddk",
    "ifconfig",
    "dns",
    "dhcp",
    "ping",
    "curl",
    "ftp",
    "tls",
    "pwd",
    "ls",
    "cd",
    "cd..",
    "mkdir",
    "write",
    "cat",
    "cp",
    "mv",
    "rm",
    "run",
    "posixtest",
    "vi",
    "mount",
    "share",
    "svc",
    "docker",
    "hermes",
    "lvgl",
    "qml",
    "uptime",
    "dhrystone",
    "sched",
    "vm",
    "loglevel",
    "kill",
    "testsc",
    "sectest",
    "fuzzsc",
    "echo",
    "clear",
    "reboot",
    "exit",
];

pub fn parse_hermes_shell_verb(args: &[&str]) -> HermesShellVerb {
    match args.first().copied() {
        Some("exec") => HermesShellVerb::Exec,
        Some("random") => HermesShellVerb::Random,
        Some("test-all") => HermesShellVerb::TestAll,
        Some("info") | Some("status") => HermesShellVerb::Info,
        Some("test") | Some("smoke") => HermesShellVerb::Test,
        Some("skills") => HermesShellVerb::Skills,
        Some("web") | Some("ui") => HermesShellVerb::Web,
        Some("ask") | Some("run") => HermesShellVerb::Ask,
        _ => HermesShellVerb::Usage,
    }
}

/// Host jobs a guest may request. SKT, smoke, and `posixtest all` are absent.
pub fn host_job_name(name: &str) -> Option<&'static str> {
    match name {
        "ut" => Some("ut"),
        "it" => Some("it"),
        _ => None,
    }
}

pub fn shell_catalog_aligned(names: &[&str]) -> bool {
    names.len() == SHELL_COMMAND_NAMES.len()
        && names
            .iter()
            .zip(SHELL_COMMAND_NAMES.iter())
            .all(|(live, expected)| live == expected)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesCampaignMode {
    Ops,
    Syscall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesCampaignOptions {
    pub seed: Option<u64>,
    pub iterations: usize,
    pub mode: HermesCampaignMode,
    /// Run `posixtest all` once before random iterations. Default is on.
    pub run_posixtest: bool,
    /// Run `sectest all` once before random iterations. Default is on.
    pub run_sectest: bool,
    /// Bit i set means extra stage i is skipped. Default is 0 (run every stage).
    pub skip_extra: u32,
}

/// Guest and host suites orchestrated once per `hermes test-all`, outside the
/// per-iteration `ut`/`it` jobs. Index matches `HermesSuitePhases::extra`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HermesExtraStage {
    Guest = 0,
    El0 = 1,
    Skt = 2,
    St = 3,
    PosixTool = 4,
    PosixFetch = 5,
    PosixAudit = 6,
    PosixBuild = 7,
    PosixStage = 8,
    PosixBaseline = 9,
    Launcher = 10,
    Verus = 11,
    Testsc = 12,
    Fuzzsc = 13,
    CbmcFutex = 14,
}

pub const HERMES_EXTRA_STAGE_COUNT: usize = 15;
pub const HERMES_GUEST_CASE_COUNT: usize = 7;
/// `sectest all` listed probes plus the heap-fork and elf-memsz kernel cases.
pub const HERMES_SECTEST_ALL_CASES: usize = 91;
pub const HERMES_GUEST_CASES: [&str; HERMES_GUEST_CASE_COUNT] = [
    "/shared/f1_seccomp.elf",
    "/shared/f2_landlock.elf",
    "/shared/f3_pkey.elf",
    "/shared/f4_iouring.elf",
    "/shared/f5_ptrace.elf",
    "/shared/g1_stopcont.elf",
    "/shared/g1b_stop.elf",
];
/// Single `sectest <id>` the campaign runs after `sectest all` and `sectest undef`.
pub const HERMES_SECTEST_ONE_ID: &str = "el0-read-kernel-ram";

/// Closing stages run after the random loop and before the shm-mode check.
/// Bounded `fuzzsc` is last so shm-mode still observes `/dev/shm`.
/// `make posix-run` and `make posix-report` stay out of this list: they boot a
/// second SMROS for the same 1600-test manifest the opening `posixtest all`
/// already executed, and that runner's guest-start check cannot share this VM.
pub const HERMES_CLOSING_STAGE_ORDER: [HermesExtraStage; 13] = [
    HermesExtraStage::El0,
    HermesExtraStage::Skt,
    HermesExtraStage::St,
    HermesExtraStage::PosixTool,
    HermesExtraStage::PosixFetch,
    HermesExtraStage::PosixAudit,
    HermesExtraStage::PosixBuild,
    HermesExtraStage::PosixStage,
    HermesExtraStage::PosixBaseline,
    HermesExtraStage::Launcher,
    HermesExtraStage::Verus,
    HermesExtraStage::CbmcFutex,
    HermesExtraStage::Fuzzsc,
];

impl HermesExtraStage {
    pub const fn index(self) -> usize {
        self as u8 as usize
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Some(match index {
            0 => Self::Guest,
            1 => Self::El0,
            2 => Self::Skt,
            3 => Self::St,
            4 => Self::PosixTool,
            5 => Self::PosixFetch,
            6 => Self::PosixAudit,
            7 => Self::PosixBuild,
            8 => Self::PosixStage,
            9 => Self::PosixBaseline,
            10 => Self::Launcher,
            11 => Self::Verus,
            12 => Self::Testsc,
            13 => Self::Fuzzsc,
            14 => Self::CbmcFutex,
            _ => return None,
        })
    }
}

pub fn extra_stage_name(stage: HermesExtraStage) -> &'static str {
    match stage {
        HermesExtraStage::Guest => "guest",
        HermesExtraStage::El0 => "el0",
        HermesExtraStage::Skt => "skt",
        HermesExtraStage::St => "st",
        HermesExtraStage::PosixTool => "posix-tool-test",
        HermesExtraStage::PosixFetch => "posix-fetch",
        HermesExtraStage::PosixAudit => "posix-audit",
        HermesExtraStage::PosixBuild => "posix-build",
        HermesExtraStage::PosixStage => "posix-stage",
        HermesExtraStage::PosixBaseline => "posix-baseline",
        HermesExtraStage::Launcher => "launcher-test",
        HermesExtraStage::Verus => "verus",
        HermesExtraStage::Testsc => "testsc",
        HermesExtraStage::Fuzzsc => "fuzzsc",
        HermesExtraStage::CbmcFutex => "cbmc-futex",
    }
}

pub fn extra_stage_from_name(name: &str) -> Option<HermesExtraStage> {
    (0..HERMES_EXTRA_STAGE_COUNT)
        .find_map(|index| {
            let stage = HermesExtraStage::from_index(index)?;
            if extra_stage_name(stage) == name {
                Some(stage)
            } else {
                None
            }
        })
}

pub fn extra_stage_min_pass(stage: HermesExtraStage) -> usize {
    match stage {
        HermesExtraStage::Guest => HERMES_GUEST_CASE_COUNT,
        _ => 1,
    }
}

/// No stage passes merely because its tool was missing. A Verus or CBMC
/// UNAVAILABLE result fails `hermes test-all`.
pub fn extra_stage_allows_unavailable(_stage: HermesExtraStage) -> bool {
    false
}

/// `posix-baseline` publishes the qemu-user failure baseline. A completed run
/// with some non-passing reference tests still passes this stage. The in-guest
/// `posixtest all` remains the 1600/1600 SMROS gate. A harness error does not.
pub fn extra_stage_allows_reference_misses(stage: HermesExtraStage) -> bool {
    matches!(stage, HermesExtraStage::PosixBaseline)
}

/// Host stages the launcher may run once. Per-iteration jobs stay `ut` and `it`.
const HOST_STAGE_NAMES: &[&str] = &[
    "el0",
    "skt",
    "st",
    "posix-tool-test",
    "posix-fetch",
    "posix-audit",
    "posix-build",
    "posix-stage",
    "posix-baseline",
    "launcher-test",
    "verus",
    "cbmc-futex",
];

pub fn host_stage_name(name: &str) -> Option<&'static str> {
    HOST_STAGE_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesCampaignCase {
    pub command: &'static str,
    pub args: [&'static str; 2],
    pub arg_count: usize,
}

const CAMPAIGN_CATALOG: [HermesCampaignCase; HERMES_CAMPAIGN_CASES] = [
    campaign_case_const("version", "", "", 0),
    campaign_case_const("ps", "-a", "", 1),
    campaign_case_const("meminfo", "", "", 0),
    campaign_case_const("components", "", "", 0),
    campaign_case_const("fxfs", "", "", 0),
    campaign_case_const("drivers", "", "", 0),
    campaign_case_const("ifconfig", "", "", 0),
    campaign_case_const("svc", "", "", 0),
    campaign_case_const("sched", "status", "", 1),
    campaign_case_const("vm", "-s", "", 1),
    campaign_case_const("vm", "-c", "/shared/vm-demo.xml", 2),
    campaign_case_const("uptime", "", "", 0),
    campaign_case_const("docker", "images", "", 1),
    campaign_case_const("docker", "ps", "", 1),
    campaign_case_const("fuzzsc", "iterations=50", "", 1),
    campaign_case_const("lvgl", "test", "", 1),
    campaign_case_const("qml", "test", "", 1),
    campaign_case_const("ddk", "hello", "", 1),
    campaign_case_const("posixtest", "status", "", 1),
    campaign_case_const("hermes", "info", "", 1),
    campaign_case_const("ls", "", "", 0),
    campaign_case_const("top", "", "", 0),
    campaign_case_const("testsc", "", "", 0),
    campaign_case_const("hermes", "test", "", 1),
    campaign_case_const("hermes", "skills", "", 1),
    campaign_case_const("hermes", "web", "", 1),
    campaign_case_const("lvgl", "info", "", 1),
    campaign_case_const("lvgl", "render", "", 1),
    campaign_case_const("lvgl", "sched", "", 1),
    campaign_case_const("qml", "info", "", 1),
    campaign_case_const("qml", "render", "", 1),
    campaign_case_const("sched", "trace", "", 1),
    campaign_case_const("posixtest", "test", "getpid/1-1.c", 2),
    campaign_case_const("reboot", "", "", 0),
];

const SYSCALL_CAMPAIGN_CATALOG: [HermesCampaignCase; HERMES_SYSCALL_CAMPAIGN_CASES] = [
    campaign_case_const("testsc", "", "", 0),
    campaign_case_const("fuzzsc", "iterations=50", "", 1),
    campaign_case_const("hermes", "test", "", 1),
    campaign_case_const("version", "", "", 0),
    campaign_case_const("meminfo", "", "", 0),
    campaign_case_const("ps", "-a", "", 1),
    campaign_case_const("components", "", "", 0),
    campaign_case_const("fxfs", "", "", 0),
    campaign_case_const("svc", "", "", 0),
    campaign_case_const("uptime", "", "", 0),
    campaign_case_const("drivers", "", "", 0),
    campaign_case_const("ifconfig", "", "", 0),
    campaign_case_const("posixtest", "status", "", 1),
    campaign_case_const("posixtest", "test", "getpid/1-1.c", 2),
];

const POSIX_CANARY_IDS: [&str; 1] = ["getpid/1-1.c"];

const fn campaign_case_const(
    command: &'static str,
    first: &'static str,
    second: &'static str,
    arg_count: usize,
) -> HermesCampaignCase {
    HermesCampaignCase {
        command,
        args: [first, second],
        arg_count,
    }
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
        "rm" | "kill" | "exit" | "clear" | "vi" | "run" | "write" | "mkdir" | "mv" | "cp"
        | "mount" | "cd" | "cd.." => HermesShellPolicy::Forbidden,
        "help" | "version" | "meminfo" | "components" | "fxfs" | "drivers" | "ifconfig" | "pwd"
        | "ls" | "svc" | "uptime" | "tls" | "share" => no_args(args),
        "ddk" => ddk_policy(args),
        "reboot" => no_args(args),
        "testsc" => no_args(args),
        "ps" => optional_exact_arg(args, "-a"),
        "top" => no_args(args),
        "sched" => sched_policy(args),
        "loglevel" => loglevel_policy(args),
        "echo" => HermesShellPolicy::Allowed,
        "fuzzsc" => fuzz_policy(args),
        "vm" => vm_policy(args),
        "docker" => docker_policy(args),
        "hermes" => hermes_policy(args),
        "posixtest" => posix_policy(args),
        "lvgl" => lvgl_policy(args),
        "qml" => qml_policy(args),
        _ => HermesShellPolicy::Forbidden,
    }
}

fn ddk_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        [] | ["hello"] | ["help"] | ["edu"] => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Invalid,
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
    match args {
        ["-s"] | ["-k"] => HermesShellPolicy::Allowed,
        ["-c", path] if valid_vm_xml_path(path) => HermesShellPolicy::Allowed,
        ["-k", name] if valid_identifier(name) => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Forbidden,
    }
}

fn docker_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        ["images"] | ["ps"] | ["ps", "-a"] | ["run"] | ["stop"] | ["rm"] => {
            HermesShellPolicy::Allowed
        }
        ["inspect", id] | ["logs", id] | ["stop", id] | ["rm", id] if valid_identifier(id) => {
            HermesShellPolicy::Allowed
        }
        ["load", path] if valid_shared_archive_path(docker_load_flag_path(path)) => {
            HermesShellPolicy::Allowed
        }
        ["load", "-i", path] | ["load", "--input", path] if valid_shared_archive_path(path) => {
            HermesShellPolicy::Allowed
        }
        _ if docker_run_args_allowed(args) => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Forbidden,
    }
}

fn hermes_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        ["test"] | ["smoke"] | ["info"] | ["status"] | ["skills"] | ["web"] => {
            HermesShellPolicy::Allowed
        }
        _ => HermesShellPolicy::Forbidden,
    }
}

fn lvgl_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        []
        | ["info"]
        | ["status"]
        | ["render"]
        | ["show"]
        | ["demo"]
        | ["sched"]
        | ["schedule"]
        | ["trace"]
        | ["test"]
        | ["smoke"] => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Invalid,
    }
}

fn qml_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        [] | ["info"] | ["status"] | ["render"] | ["show"] | ["demo"] | ["test"] | ["smoke"] => {
            HermesShellPolicy::Allowed
        }
        _ => HermesShellPolicy::Invalid,
    }
}

fn posix_policy(args: &[&str]) -> HermesShellPolicy {
    match args {
        ["status"] => HermesShellPolicy::Allowed,
        ["test", "getpid/1-1.c"] => HermesShellPolicy::Allowed,
        ["test", id] if posix_canary_id(id) => HermesShellPolicy::Allowed,
        _ => HermesShellPolicy::Forbidden,
    }
}

fn valid_identifier(value: &str) -> bool {
    valid_token(value, false)
}

fn valid_image_ref(value: &str) -> bool {
    valid_token(value, true)
}

fn valid_token(value: &str, allow_slash: bool) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
                || (allow_slash && byte == b'/')
        })
}

fn valid_guest_path(value: &str, prefixes: &[&str], suffix: &str) -> bool {
    if value.is_empty()
        || value.len() > HERMES_MAX_ARG_LEN
        || value.contains("..")
        || !value.ends_with(suffix)
        || !prefixes.iter().any(|prefix| value.starts_with(prefix))
    {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn valid_vm_xml_path(value: &str) -> bool {
    valid_guest_path(value, &["/shared/", "/config/"], ".xml")
}

fn valid_shared_archive_path(value: &str) -> bool {
    valid_guest_path(value, &["/shared/"], ".tar")
}

fn docker_load_flag_path(value: &str) -> &str {
    value.strip_prefix("--input=").unwrap_or(value)
}

fn docker_run_args_allowed(args: &[&str]) -> bool {
    if args.first().copied() != Some("run") {
        return false;
    }
    if args.len() == 1 {
        return true;
    }

    let mut index = 1usize;
    while index < args.len() {
        match args[index] {
            "-i" | "--interactive" | "-t" | "--tty" | "-it" | "-ti" | "--rm" => {
                index += 1;
            }
            "--name" => {
                if index + 1 >= args.len() || !valid_identifier(args[index + 1]) {
                    return false;
                }
                index += 2;
            }
            value if value.starts_with("--name=") => {
                if !valid_identifier(&value["--name=".len()..]) {
                    return false;
                }
                index += 1;
            }
            value if value.starts_with('-') => return false,
            _ => break,
        }
    }
    if index >= args.len() || !valid_image_ref(args[index]) {
        return false;
    }
    args[index + 1..]
        .iter()
        .all(|arg| valid_image_ref(arg) || valid_identifier(arg))
}

fn posix_canary_id(value: &str) -> bool {
    let mut index = 0usize;
    while index < POSIX_CANARY_IDS.len() {
        if POSIX_CANARY_IDS[index] == value {
            return true;
        }
        index += 1;
    }
    false
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
            "iterations" if number > 0 && number <= HERMES_CAMPAIGN_FUZZSC_ITERATIONS => {}
            "time" if number > 0 && number <= HERMES_CAMPAIGN_FUZZSC_TIME_SECONDS => {}
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
    iterations > 0
}

pub fn campaign_report_includes_round(round: usize) -> bool {
    round < HERMES_REPORT_DETAIL_LIMIT
}

pub fn campaign_report_omitted_rounds(iterations: usize) -> usize {
    iterations.saturating_sub(HERMES_REPORT_DETAIL_LIMIT)
}

pub fn campaign_catalog_len(mode: HermesCampaignMode) -> usize {
    match mode {
        HermesCampaignMode::Ops => HERMES_CAMPAIGN_CASES,
        HermesCampaignMode::Syscall => HERMES_SYSCALL_CAMPAIGN_CASES,
    }
}

pub fn campaign_case_index(seed: u64, round: usize) -> usize {
    campaign_case_index_for_mode(seed, round, HermesCampaignMode::Ops)
}

pub fn campaign_case_index_for_mode(seed: u64, round: usize, mode: HermesCampaignMode) -> usize {
    let mut state = seed ^ (round as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    next_random(&mut state) as usize % campaign_catalog_len(mode)
}

pub fn campaign_case(index: usize, _seed: u64, _round: usize) -> Option<HermesCampaignCase> {
    campaign_case_for_mode(HermesCampaignMode::Ops, index)
}

pub fn campaign_case_for_mode(
    mode: HermesCampaignMode,
    index: usize,
) -> Option<HermesCampaignCase> {
    match mode {
        HermesCampaignMode::Ops => CAMPAIGN_CATALOG.get(index).copied(),
        HermesCampaignMode::Syscall => SYSCALL_CAMPAIGN_CATALOG.get(index).copied(),
    }
}

pub fn parse_campaign_options(args: &[&str]) -> Option<HermesCampaignOptions> {
    let mut seed = None;
    let mut iterations = 8usize;
    let mut iterations_seen = false;
    let mut mode = HermesCampaignMode::Ops;
    let mut mode_seen = false;
    let mut run_posixtest = true;
    let mut posixtest_seen = false;
    let mut run_sectest = true;
    let mut sectest_seen = false;
    let mut skip_extra = 0u32;
    let mut extra_seen = 0u32;

    for arg in args {
        let (key, value) = arg.split_once('=')?;
        if let Some(stage) = extra_stage_from_name(key) {
            let bit = 1u32 << stage.index();
            if extra_seen & bit != 0 {
                return None;
            }
            extra_seen |= bit;
            match value {
                "run" => {}
                "skip" => skip_extra |= bit,
                _ => return None,
            }
            continue;
        }
        if key == "mode" {
            if mode_seen {
                return None;
            }
            mode = match value {
                "ops" => HermesCampaignMode::Ops,
                "syscall" => HermesCampaignMode::Syscall,
                _ => return None,
            };
            mode_seen = true;
            continue;
        }
        if key == "posixtest" {
            if posixtest_seen {
                return None;
            }
            run_posixtest = match value {
                "run" => true,
                "skip" => false,
                _ => return None,
            };
            posixtest_seen = true;
            continue;
        }
        if key == "sectest" {
            if sectest_seen {
                return None;
            }
            run_sectest = match value {
                "run" => true,
                "skip" => false,
                _ => return None,
            };
            sectest_seen = true;
            continue;
        }
        let number = parse_decimal(value)?;
        match key {
            "seed" if seed.is_none() => seed = Some(number),
            "iterations" if !iterations_seen && number <= usize::MAX as u64 => {
                iterations = number as usize;
                iterations_seen = true;
            }
            _ => return None,
        }
    }

    if campaign_iterations_valid(iterations) {
        Some(HermesCampaignOptions {
            seed,
            iterations,
            mode,
            run_posixtest,
            run_sectest,
            skip_extra,
        })
    } else {
        None
    }
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

pub const HERMES_RESUME_PATH: &str = "/data/hermes/tests/resume.cfg";
/// Resume text includes opening-suite counts. A full `usize` on every counter
/// plus a 64-byte pending op still has to fit, or a reboot checkpoint is dropped.
pub const HERMES_RESUME_TEXT_CAPACITY: usize = 8192;
/// `posixtest all` on the current manifest. The opening phase passes only at
/// this count with fail, unresolved, timeout, and launch-error all zero.
pub const HERMES_POSIXTEST_ALL_EXPECTED: usize = 1600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesResumeKind {
    TestAll,
    Random,
}

/// Opening `posixtest all` / `sectest all` checkpoint.
/// `Legacy` is a resume file written before those phases existed: do not
/// re-run them on top of a campaign that may already have run `fuzzsc`, and
/// do not fail the old checkpoint for counts it never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesSuiteDisposition {
    Legacy,
    Done,
    Skip,
    /// Selected and not finished. A campaign that ends here fails closed.
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesPosixPhase {
    pub disposition: HermesSuiteDisposition,
    pub selected: usize,
    pub passed: usize,
    pub failed: usize,
    pub unresolved: usize,
    pub timeouts: usize,
    pub launch_errors: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesSectestPhase {
    pub disposition: HermesSuiteDisposition,
    pub pass: usize,
    pub fail: usize,
    pub xfail: usize,
    pub xpass: usize,
    /// `sectest` printed SKIP because this kernel is not AArch64.
    pub arch_skipped: bool,
    /// Case ids `sectest all` must execute, including heap-fork and elf-memsz.
    pub ids_expected: usize,
    pub ids_ran: usize,
    /// `sectest undef` counts. A hard fail here fails the sectest phase.
    pub undef_pass: usize,
    pub undef_fail: usize,
    /// One `sectest <id>` invocation, so the single-id path is not only `all`.
    pub one_pass: usize,
    pub one_fail: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesCountedPhase {
    pub disposition: HermesSuiteDisposition,
    pub pass: usize,
    pub fail: usize,
    pub unavailable: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesSuitePhases {
    pub posix: HermesPosixPhase,
    pub sectest: HermesSectestPhase,
    pub extra: [HermesCountedPhase; HERMES_EXTRA_STAGE_COUNT],
}

impl HermesPosixPhase {
    pub const fn legacy() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Legacy,
            selected: 0,
            passed: 0,
            failed: 0,
            unresolved: 0,
            timeouts: 0,
            launch_errors: 0,
        }
    }

    pub const fn skipped() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Skip,
            selected: 0,
            passed: 0,
            failed: 0,
            unresolved: 0,
            timeouts: 0,
            launch_errors: 0,
        }
    }
}

impl HermesSectestPhase {
    pub const fn legacy() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Legacy,
            pass: 0,
            fail: 0,
            xfail: 0,
            xpass: 0,
            arch_skipped: false,
            ids_expected: 0,
            ids_ran: 0,
            undef_pass: 0,
            undef_fail: 0,
            one_pass: 0,
            one_fail: 0,
        }
    }

    pub const fn skipped() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Skip,
            pass: 0,
            fail: 0,
            xfail: 0,
            xpass: 0,
            arch_skipped: false,
            ids_expected: 0,
            ids_ran: 0,
            undef_pass: 0,
            undef_fail: 0,
            one_pass: 0,
            one_fail: 0,
        }
    }
}

impl HermesCountedPhase {
    pub const fn legacy() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Legacy,
            pass: 0,
            fail: 0,
            unavailable: 0,
        }
    }

    pub const fn skipped() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Skip,
            pass: 0,
            fail: 0,
            unavailable: 0,
        }
    }

    pub const fn pending() -> Self {
        Self {
            disposition: HermesSuiteDisposition::Pending,
            pass: 0,
            fail: 0,
            unavailable: 0,
        }
    }
}

pub fn fresh_extra_stages(skip_extra: u32) -> [HermesCountedPhase; HERMES_EXTRA_STAGE_COUNT] {
    let mut extra = [HermesCountedPhase::pending(); HERMES_EXTRA_STAGE_COUNT];
    let mut index = 0usize;
    while index < HERMES_EXTRA_STAGE_COUNT {
        if skip_extra & (1u32 << index) != 0 {
            extra[index] = HermesCountedPhase::skipped();
        }
        index += 1;
    }
    extra
}

impl HermesSuitePhases {
    pub const fn legacy() -> Self {
        Self {
            posix: HermesPosixPhase::legacy(),
            sectest: HermesSectestPhase::legacy(),
            extra: [HermesCountedPhase::legacy(); HERMES_EXTRA_STAGE_COUNT],
        }
    }
}

pub fn suite_disposition_name(disposition: HermesSuiteDisposition) -> &'static str {
    match disposition {
        HermesSuiteDisposition::Legacy => "legacy",
        HermesSuiteDisposition::Done => "done",
        HermesSuiteDisposition::Skip => "skip",
        HermesSuiteDisposition::Pending => "pending",
    }
}

pub fn posix_phase_pass(phase: &HermesPosixPhase) -> bool {
    match phase.disposition {
        HermesSuiteDisposition::Legacy | HermesSuiteDisposition::Skip => true,
        HermesSuiteDisposition::Pending => false,
        HermesSuiteDisposition::Done => {
            phase.selected == HERMES_POSIXTEST_ALL_EXPECTED
                && phase.passed == HERMES_POSIXTEST_ALL_EXPECTED
                && phase.failed == 0
                && phase.unresolved == 0
                && phase.timeouts == 0
                && phase.launch_errors == 0
        }
    }
}

pub fn sectest_phase_pass(phase: &HermesSectestPhase) -> bool {
    match phase.disposition {
        HermesSuiteDisposition::Legacy | HermesSuiteDisposition::Skip => true,
        HermesSuiteDisposition::Pending => false,
        HermesSuiteDisposition::Done => {
            !phase.arch_skipped
                && phase.fail == 0
                && phase.xpass == 0
                && phase.ids_expected == HERMES_SECTEST_ALL_CASES
                && phase.ids_ran == phase.ids_expected
                && phase.undef_fail == 0
                && phase.undef_pass >= 1
                && phase.one_fail == 0
                && phase.one_pass >= 1
        }
    }
}

pub fn counted_phase_pass(
    phase: &HermesCountedPhase,
    min_pass: usize,
    allow_unavailable: bool,
    allow_reference_misses: bool,
) -> bool {
    match phase.disposition {
        HermesSuiteDisposition::Legacy | HermesSuiteDisposition::Skip => true,
        HermesSuiteDisposition::Pending => false,
        HermesSuiteDisposition::Done => {
            if phase.fail != 0 {
                return allow_reference_misses && phase.pass > 0 && phase.unavailable == 0;
            }
            if phase.unavailable > 0 {
                return allow_unavailable && phase.pass == 0;
            }
            phase.pass >= min_pass
        }
    }
}

pub fn extra_phases_pass(extra: &[HermesCountedPhase; HERMES_EXTRA_STAGE_COUNT]) -> bool {
    (0..HERMES_EXTRA_STAGE_COUNT).all(|index| {
        let Some(stage) = HermesExtraStage::from_index(index) else {
            return false;
        };
        counted_phase_pass(
            &extra[index],
            extra_stage_min_pass(stage),
            extra_stage_allows_unavailable(stage),
            extra_stage_allows_reference_misses(stage),
        )
    })
}

pub fn test_all_overall_pass(
    native_ok: bool,
    op_timeouts: usize,
    completed: usize,
    iterations: usize,
    host_passes: [usize; 2],
    suites: &HermesSuitePhases,
    shm_mode_ok: bool,
) -> bool {
    native_ok
        && op_timeouts == 0
        && completed == iterations
        && host_passes.iter().all(|passes| *passes == iterations)
        && posix_phase_pass(&suites.posix)
        && sectest_phase_pass(&suites.sectest)
        && extra_phases_pass(&suites.extra)
        && shm_mode_ok
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HermesResumeState {
    pub kind: HermesResumeKind,
    pub seed: u64,
    pub iterations: usize,
    pub mode: HermesCampaignMode,
    pub next_round: usize,
    pub native_ok: bool,
    pub completed: usize,
    pub denied: usize,
    pub invalid: usize,
    pub unknown: usize,
    pub host_ut_pass: usize,
    pub host_ut_fail: usize,
    pub host_it_pass: usize,
    pub host_it_fail: usize,
    pub host_jobs_pending: bool,
    /// Guest op began and had not finished when the checkpoint was written.
    /// A reboot that still sees this flag counts the op as a timeout.
    pub op_running: bool,
    pub op_timeouts: usize,
    pub pending_op_len: u8,
    pub pending_op: [u8; 64],
    /// Opening suites. Copied into every iteration checkpoint so a reboot
    /// after `posixtest all` / `sectest all` keeps the counts and does not
    /// run those commands again.
    pub suites: HermesSuitePhases,
}

impl HermesResumeState {
    pub fn set_pending_op(&mut self, op: &str) {
        let bytes = op.as_bytes();
        let len = bytes.len().min(self.pending_op.len());
        self.pending_op[..len].copy_from_slice(&bytes[..len]);
        self.pending_op_len = len as u8;
    }

    pub fn pending_op_str(&self) -> &str {
        let len = self.pending_op_len as usize;
        let len = len.min(self.pending_op.len());
        core::str::from_utf8(&self.pending_op[..len]).unwrap_or("?")
    }
}

pub fn format_op_label(command: &str, args: &[&str], out: &mut [u8]) -> Option<usize> {
    let mut written = 0usize;
    push_bytes(out, &mut written, command.as_bytes())?;
    for arg in args {
        if arg.is_empty() {
            continue;
        }
        push_bytes(out, &mut written, b" ")?;
        push_bytes(out, &mut written, arg.as_bytes())?;
    }
    Some(written)
}

pub fn resume_should_continue(state: &HermesResumeState) -> bool {
    state.next_round < state.iterations
}

pub fn campaign_case_is_reboot(command: &str) -> bool {
    command == "reboot"
}

pub fn campaign_case_is_fuzzsc(command: &str) -> bool {
    command == "fuzzsc"
}

pub fn campaign_fuzzsc_seed_arg(seed: u64, out: &mut [u8]) -> Option<&str> {
    let mut written = 0usize;
    push_bytes(out, &mut written, b"seed=")?;
    push_u64(out, &mut written, seed)?;
    core::str::from_utf8(&out[..written]).ok()
}

pub fn campaign_case_is_vm_create(case: &HermesCampaignCase) -> bool {
    case.command == "vm" && case.arg_count == 2 && case.args[0] == "-c"
}

pub fn campaign_case_is_vm_kill(case: &HermesCampaignCase) -> bool {
    case.command == "vm" && case.arg_count >= 1 && case.args[0] == "-k"
}

pub fn campaign_vm_recycle_name() -> &'static str {
    HERMES_CAMPAIGN_VM_NAME
}

pub fn resume_pending_host_iteration(state: &HermesResumeState) -> Option<usize> {
    if state.host_jobs_pending {
        Some(state.next_round)
    } else {
        None
    }
}

pub fn format_resume_state(state: &HermesResumeState, out: &mut [u8]) -> Option<usize> {
    let mut written = 0usize;
    push_bytes(out, &mut written, b"kind=")?;
    push_bytes(
        out,
        &mut written,
        match state.kind {
            HermesResumeKind::TestAll => b"test-all",
            HermesResumeKind::Random => b"random",
        },
    )?;
    push_bytes(out, &mut written, b"\nseed=")?;
    push_u64(out, &mut written, state.seed)?;
    push_bytes(out, &mut written, b"\niterations=")?;
    push_u64(out, &mut written, state.iterations as u64)?;
    push_bytes(out, &mut written, b"\nmode=")?;
    push_bytes(
        out,
        &mut written,
        match state.mode {
            HermesCampaignMode::Ops => b"ops",
            HermesCampaignMode::Syscall => b"syscall",
        },
    )?;
    push_bytes(out, &mut written, b"\nnext_round=")?;
    push_u64(out, &mut written, state.next_round as u64)?;
    push_bytes(out, &mut written, b"\nnative=")?;
    push_bytes(
        out,
        &mut written,
        if state.native_ok { b"pass" } else { b"fail" },
    )?;
    push_bytes(out, &mut written, b"\ncompleted=")?;
    push_u64(out, &mut written, state.completed as u64)?;
    push_bytes(out, &mut written, b"\ndenied=")?;
    push_u64(out, &mut written, state.denied as u64)?;
    push_bytes(out, &mut written, b"\ninvalid=")?;
    push_u64(out, &mut written, state.invalid as u64)?;
    push_bytes(out, &mut written, b"\nunknown=")?;
    push_u64(out, &mut written, state.unknown as u64)?;
    push_bytes(out, &mut written, b"\nhost_ut_pass=")?;
    push_u64(out, &mut written, state.host_ut_pass as u64)?;
    push_bytes(out, &mut written, b"\nhost_ut_fail=")?;
    push_u64(out, &mut written, state.host_ut_fail as u64)?;
    push_bytes(out, &mut written, b"\nhost_it_pass=")?;
    push_u64(out, &mut written, state.host_it_pass as u64)?;
    push_bytes(out, &mut written, b"\nhost_it_fail=")?;
    push_u64(out, &mut written, state.host_it_fail as u64)?;
    push_bytes(out, &mut written, b"\nhost_jobs_pending=")?;
    push_bytes(
        out,
        &mut written,
        if state.host_jobs_pending { b"1" } else { b"0" },
    )?;
    push_bytes(out, &mut written, b"\nop_running=")?;
    push_bytes(
        out,
        &mut written,
        if state.op_running { b"1" } else { b"0" },
    )?;
    push_bytes(out, &mut written, b"\nop_timeouts=")?;
    push_u64(out, &mut written, state.op_timeouts as u64)?;
    push_bytes(out, &mut written, b"\npending_op=")?;
    push_bytes(out, &mut written, state.pending_op_str().as_bytes())?;
    push_bytes(out, &mut written, b"\nposix=")?;
    push_bytes(
        out,
        &mut written,
        suite_disposition_name(state.suites.posix.disposition).as_bytes(),
    )?;
    push_bytes(out, &mut written, b"\nposix_selected=")?;
    push_u64(out, &mut written, state.suites.posix.selected as u64)?;
    push_bytes(out, &mut written, b"\nposix_pass=")?;
    push_u64(out, &mut written, state.suites.posix.passed as u64)?;
    push_bytes(out, &mut written, b"\nposix_fail=")?;
    push_u64(out, &mut written, state.suites.posix.failed as u64)?;
    push_bytes(out, &mut written, b"\nposix_unresolved=")?;
    push_u64(out, &mut written, state.suites.posix.unresolved as u64)?;
    push_bytes(out, &mut written, b"\nposix_timeout=")?;
    push_u64(out, &mut written, state.suites.posix.timeouts as u64)?;
    push_bytes(out, &mut written, b"\nposix_launch=")?;
    push_u64(out, &mut written, state.suites.posix.launch_errors as u64)?;
    push_bytes(out, &mut written, b"\nsectest=")?;
    push_bytes(
        out,
        &mut written,
        suite_disposition_name(state.suites.sectest.disposition).as_bytes(),
    )?;
    push_bytes(out, &mut written, b"\nsectest_pass=")?;
    push_u64(out, &mut written, state.suites.sectest.pass as u64)?;
    push_bytes(out, &mut written, b"\nsectest_fail=")?;
    push_u64(out, &mut written, state.suites.sectest.fail as u64)?;
    push_bytes(out, &mut written, b"\nsectest_xfail=")?;
    push_u64(out, &mut written, state.suites.sectest.xfail as u64)?;
    push_bytes(out, &mut written, b"\nsectest_xpass=")?;
    push_u64(out, &mut written, state.suites.sectest.xpass as u64)?;
    push_bytes(out, &mut written, b"\nsectest_arch=")?;
    push_bytes(
        out,
        &mut written,
        if state.suites.sectest.arch_skipped {
            b"1"
        } else {
            b"0"
        },
    )?;
    push_bytes(out, &mut written, b"\nsectest_ids=")?;
    push_u64(out, &mut written, state.suites.sectest.ids_expected as u64)?;
    push_bytes(out, &mut written, b"\nsectest_ids_ran=")?;
    push_u64(out, &mut written, state.suites.sectest.ids_ran as u64)?;
    push_bytes(out, &mut written, b"\nsectest_undef_pass=")?;
    push_u64(out, &mut written, state.suites.sectest.undef_pass as u64)?;
    push_bytes(out, &mut written, b"\nsectest_undef_fail=")?;
    push_u64(out, &mut written, state.suites.sectest.undef_fail as u64)?;
    push_bytes(out, &mut written, b"\nsectest_one_pass=")?;
    push_u64(out, &mut written, state.suites.sectest.one_pass as u64)?;
    push_bytes(out, &mut written, b"\nsectest_one_fail=")?;
    push_u64(out, &mut written, state.suites.sectest.one_fail as u64)?;
    for index in 0..HERMES_EXTRA_STAGE_COUNT {
        let Some(stage) = HermesExtraStage::from_index(index) else {
            return None;
        };
        let phase = &state.suites.extra[index];
        push_bytes(out, &mut written, b"\n")?;
        push_bytes(out, &mut written, extra_stage_name(stage).as_bytes())?;
        push_bytes(out, &mut written, b"=")?;
        push_bytes(
            out,
            &mut written,
            suite_disposition_name(phase.disposition).as_bytes(),
        )?;
        push_bytes(out, &mut written, b"\n")?;
        push_bytes(out, &mut written, extra_stage_name(stage).as_bytes())?;
        push_bytes(out, &mut written, b"_pass=")?;
        push_u64(out, &mut written, phase.pass as u64)?;
        push_bytes(out, &mut written, b"\n")?;
        push_bytes(out, &mut written, extra_stage_name(stage).as_bytes())?;
        push_bytes(out, &mut written, b"_fail=")?;
        push_u64(out, &mut written, phase.fail as u64)?;
        push_bytes(out, &mut written, b"\n")?;
        push_bytes(out, &mut written, extra_stage_name(stage).as_bytes())?;
        push_bytes(out, &mut written, b"_unavail=")?;
        push_u64(out, &mut written, phase.unavailable as u64)?;
    }
    push_bytes(out, &mut written, b"\n")?;
    Some(written)
}

pub fn parse_resume_state(text: &str) -> Option<HermesResumeState> {
    let mut kind = None;
    let mut seed = None;
    let mut iterations = None;
    let mut mode = HermesCampaignMode::Ops;
    let mut next_round = None;
    let mut native_ok = true;
    let mut completed = 0usize;
    let mut denied = 0usize;
    let mut invalid = 0usize;
    let mut unknown = 0usize;
    let mut host_ut_pass = 0usize;
    let mut host_ut_fail = 0usize;
    let mut host_it_pass = 0usize;
    let mut host_it_fail = 0usize;
    let mut host_jobs_pending = false;
    let mut op_running = false;
    let mut op_timeouts = 0usize;
    let mut pending_op = [0u8; 64];
    let mut pending_op_len = 0u8;
    let mut posix = HermesPosixPhase::legacy();
    let mut sectest = HermesSectestPhase::legacy();
    let mut extra = [HermesCountedPhase::legacy(); HERMES_EXTRA_STAGE_COUNT];
    let mut saw_posix = false;
    let mut saw_sectest = false;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        match key {
            "kind" => {
                kind = Some(match value {
                    "test-all" => HermesResumeKind::TestAll,
                    "random" => HermesResumeKind::Random,
                    _ => return None,
                });
            }
            "seed" => seed = Some(parse_decimal(value)?),
            "iterations" => iterations = Some(parse_decimal(value)? as usize),
            "mode" => {
                mode = match value {
                    "ops" => HermesCampaignMode::Ops,
                    "syscall" => HermesCampaignMode::Syscall,
                    "ui" => HermesCampaignMode::Ops,
                    _ => return None,
                };
            }
            "next_round" => next_round = Some(parse_decimal(value)? as usize),
            "native" => {
                native_ok = match value {
                    "pass" => true,
                    "fail" => false,
                    _ => return None,
                };
            }
            "completed" => completed = parse_decimal(value)? as usize,
            "denied" => denied = parse_decimal(value)? as usize,
            "invalid" => invalid = parse_decimal(value)? as usize,
            "unknown" => unknown = parse_decimal(value)? as usize,
            "host_ut_pass" => host_ut_pass = parse_decimal(value)? as usize,
            "host_ut_fail" => host_ut_fail = parse_decimal(value)? as usize,
            "host_it_pass" => host_it_pass = parse_decimal(value)? as usize,
            "host_it_fail" => host_it_fail = parse_decimal(value)? as usize,
            "host_jobs_pending" => {
                host_jobs_pending = match value {
                    "1" => true,
                    "0" => false,
                    _ => return None,
                };
            }
            "op_running" => {
                op_running = match value {
                    "1" => true,
                    "0" => false,
                    _ => return None,
                };
            }
            "op_timeouts" => op_timeouts = parse_decimal(value)? as usize,
            "pending_op" => {
                let bytes = value.as_bytes();
                let len = bytes.len().min(pending_op.len());
                pending_op[..len].copy_from_slice(&bytes[..len]);
                pending_op_len = len as u8;
            }
            "posix" => {
                posix.disposition = parse_suite_disposition(value)?;
                saw_posix = true;
            }
            "posix_selected" => posix.selected = parse_decimal(value)? as usize,
            "posix_pass" => posix.passed = parse_decimal(value)? as usize,
            "posix_fail" => posix.failed = parse_decimal(value)? as usize,
            "posix_unresolved" => posix.unresolved = parse_decimal(value)? as usize,
            "posix_timeout" => posix.timeouts = parse_decimal(value)? as usize,
            "posix_launch" => posix.launch_errors = parse_decimal(value)? as usize,
            "sectest" => {
                sectest.disposition = parse_suite_disposition(value)?;
                saw_sectest = true;
            }
            "sectest_pass" => sectest.pass = parse_decimal(value)? as usize,
            "sectest_fail" => sectest.fail = parse_decimal(value)? as usize,
            "sectest_xfail" => sectest.xfail = parse_decimal(value)? as usize,
            "sectest_xpass" => sectest.xpass = parse_decimal(value)? as usize,
            "sectest_arch" => {
                sectest.arch_skipped = match value {
                    "1" => true,
                    "0" => false,
                    _ => return None,
                };
            }
            "sectest_ids" => sectest.ids_expected = parse_decimal(value)? as usize,
            "sectest_ids_ran" => sectest.ids_ran = parse_decimal(value)? as usize,
            "sectest_undef_pass" => sectest.undef_pass = parse_decimal(value)? as usize,
            "sectest_undef_fail" => sectest.undef_fail = parse_decimal(value)? as usize,
            "sectest_one_pass" => sectest.one_pass = parse_decimal(value)? as usize,
            "sectest_one_fail" => sectest.one_fail = parse_decimal(value)? as usize,
            other => {
                if !parse_extra_stage_field(&mut extra, other, value)? {
                    // Unknown keys stay ignored so older writers can add fields.
                }
            }
        }
    }
    if saw_posix != saw_sectest {
        return None;
    }

    let iterations = iterations?;
    if iterations == 0 {
        return None;
    }
    Some(HermesResumeState {
        kind: kind?,
        seed: seed?,
        iterations,
        mode,
        next_round: next_round?,
        native_ok,
        completed,
        denied,
        invalid,
        unknown,
        host_ut_pass,
        host_ut_fail,
        host_it_pass,
        host_it_fail,
        host_jobs_pending,
        op_running,
        op_timeouts,
        pending_op_len,
        pending_op,
        suites: HermesSuitePhases {
            posix,
            sectest,
            extra,
        },
    })
}

fn parse_extra_stage_field(
    extra: &mut [HermesCountedPhase; HERMES_EXTRA_STAGE_COUNT],
    key: &str,
    value: &str,
) -> Option<bool> {
    let (name, field) = if let Some(name) = key.strip_suffix("_pass") {
        (name, 1u8)
    } else if let Some(name) = key.strip_suffix("_fail") {
        (name, 2u8)
    } else if let Some(name) = key.strip_suffix("_unavail") {
        (name, 3u8)
    } else {
        (key, 0u8)
    };
    let Some(stage) = extra_stage_from_name(name) else {
        return Some(false);
    };
    let phase = &mut extra[stage.index()];
    match field {
        0 => phase.disposition = parse_suite_disposition(value)?,
        1 => phase.pass = parse_decimal(value)? as usize,
        2 => phase.fail = parse_decimal(value)? as usize,
        _ => phase.unavailable = parse_decimal(value)? as usize,
    }
    Some(true)
}

fn parse_suite_disposition(value: &str) -> Option<HermesSuiteDisposition> {
    match value {
        "legacy" => Some(HermesSuiteDisposition::Legacy),
        "done" => Some(HermesSuiteDisposition::Done),
        "skip" => Some(HermesSuiteDisposition::Skip),
        "pending" => Some(HermesSuiteDisposition::Pending),
        _ => None,
    }
}

fn push_bytes(buf: &mut [u8], written: &mut usize, bytes: &[u8]) -> Option<()> {
    let next = written.checked_add(bytes.len())?;
    if next > buf.len() {
        return None;
    }
    buf[*written..next].copy_from_slice(bytes);
    *written = next;
    Some(())
}

fn push_u64(buf: &mut [u8], written: &mut usize, value: u64) -> Option<()> {
    let mut digits = [0u8; 20];
    let mut remaining = value;
    let mut index = digits.len();
    loop {
        index -= 1;
        digits[index] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    push_bytes(buf, written, &digits[index..])
}
