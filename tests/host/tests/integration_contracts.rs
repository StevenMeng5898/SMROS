#![allow(unused_comparisons, unused_macros)]

mod syscall_address_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/address_logic_shared.rs"
    ));

    pub fn checked_end(addr: usize, len: usize) -> Option<usize> {
        smros_checked_end_body!(addr, len)
    }

    pub fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
        smros_range_overlaps_body!(start_a, len_a, start_b, len_b)
    }

    pub fn fixed_mmap_request_ok(
        addr: usize,
        len: usize,
        page_size: usize,
        base: usize,
        limit: usize,
    ) -> bool {
        smros_fixed_linux_mmap_request_ok_body!(addr, len, page_size, base, limit)
    }
}

mod kernel_object_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/object_logic_shared.rs"
    ));

    pub fn checked_end(addr: usize, len: usize) -> Option<usize> {
        smros_ko_checked_end_body!(addr, len)
    }

    pub fn ranges_overlap(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
        smros_ko_ranges_overlap_body!(start_a, len_a, start_b, len_b)
    }

    pub fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
        smros_ko_signal_update_body!(current, clear_mask, set_mask)
    }
}

mod syscall_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_logic_shared.rs"
    ));

    pub fn is_zircon_syscall_number(syscall_num: u64, threshold: u64) -> bool {
        smros_is_zircon_syscall_number_body!(syscall_num, threshold)
    }

    pub fn zircon_syscall_from_raw(syscall_num: u64, threshold: u64) -> u32 {
        smros_zircon_syscall_from_raw_body!(syscall_num, threshold)
    }

    pub fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
        smros_syscall_signal_update_body!(current, clear_mask, set_mask)
    }

    pub fn linux_syscall_interface_known(syscall_num: u64) -> bool {
        smros_linux_syscall_interface_known_body!(syscall_num)
    }

    pub fn zircon_syscall_interface_known(syscall_num: u32) -> bool {
        smros_zircon_syscall_interface_known_body!(syscall_num)
    }
}

mod syscall_bridge_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_bridge_shared.rs"
    ));

    pub fn is_linux_syscall_number(syscall_num: u64) -> bool {
        smros_is_linux_syscall_number_u64_body!(syscall_num)
    }
}

mod fifo_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/fifo_logic_shared.rs"
    ));

    pub fn ring_index(read_pos: usize, offset: usize, capacity: usize) -> usize {
        smros_fifo_ring_index_body!(read_pos, offset, capacity)
    }

    pub fn remaining_capacity(len: usize, capacity: usize) -> usize {
        smros_fifo_remaining_capacity_body!(len, capacity)
    }

    pub fn min_count(left: usize, right: usize) -> usize {
        smros_fifo_min_count_body!(left, right)
    }
}

mod socket_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/socket_logic_shared.rs"
    ));

    pub fn ring_index(read_pos: usize, offset: usize, capacity: usize) -> usize {
        smros_socket_ring_index_body!(read_pos, offset, capacity)
    }

    pub fn remaining_capacity(len: usize, capacity: usize) -> usize {
        smros_socket_remaining_capacity_body!(len, capacity)
    }

    pub fn min_count(left: usize, right: usize) -> usize {
        smros_socket_min_count_body!(left, right)
    }
}

mod lowlevel_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/lowlevel_logic_shared.rs"
    ));

    pub fn checked_end(addr: usize, len: usize) -> Option<usize> {
        smros_ll_checked_end_body!(addr, len)
    }
}

mod user_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/user_logic_shared.rs"
    ));

    pub fn checked_end(addr: usize, len: usize) -> Option<usize> {
        smros_user_checked_end_body!(addr, len)
    }

    pub fn elf_segment_mapping_range(
        vaddr: usize,
        mem_size: usize,
        page_size: usize,
    ) -> Option<(usize, usize)> {
        smros_user_elf_segment_mapping_range_body!(vaddr, mem_size, page_size)
    }
}

fn braced_body(source: &str) -> &str {
    let open = source.find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("closing brace");
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("smros-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_posix_make_value_is_shell_safe(target: &str, variable: &str, flag: &str) {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new("posix-make-shell-safety");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = temp.0.join("bin");
    std::fs::create_dir(&bin).expect("create fake executable directory");
    let python = bin.join("python3");
    std::fs::write(
        &python,
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$ARGV_CAPTURE\"\n",
    )
    .expect("write argv recorder");
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o700))
        .expect("make argv recorder executable");

    let injected = temp.0.join("injected");
    let substituted = temp.0.join("substituted");
    let make_value = format!(
        "value with spaces'; touch {}; $(shell touch {}); # apostrophe' semicolon; wildcard*",
        injected.display(),
        substituted.display(),
    );
    let expected_value = &make_value;
    let capture = temp.0.join(format!("{target}.argv"));
    let original_path = std::env::var_os("PATH").expect("PATH");
    let path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::split_paths(&original_path)),
    )
    .expect("compose PATH");

    let disk = temp.0.join("fxfs.img");
    std::fs::write(&disk, []).expect("create existing fake disk");
    let mut command = std::process::Command::new("make");
    command
        .current_dir(&repository)
        .arg("--no-print-directory")
        .arg("--old-file=posix-stage")
        .arg(target)
        .arg(format!("{variable}={make_value}"))
        .env("ARGV_CAPTURE", &capture)
        .env("PATH", &path);
    if target == "posix-run" {
        command
            .arg(format!("FXFS_DISK={}", disk.display()))
            .arg("MAKE=true");
    }
    let output = command.output().expect("execute POSIX Make target");
    assert!(
        output.status.success(),
        "{target} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = std::fs::read(&capture).expect("read captured Python argv");
    let arguments = captured
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8(argument.to_vec()).expect("UTF-8 argv"))
        .collect::<Vec<_>>();
    let flag_index = arguments
        .iter()
        .position(|argument| argument == flag)
        .expect("captured expected flag");

    let mut dry_run = std::process::Command::new("make");
    dry_run
        .current_dir(&repository)
        .arg("--no-print-directory")
        .arg("--dry-run")
        .arg("--old-file=posix-stage")
        .arg(target)
        .arg(format!("{variable}={make_value}"));
    if target == "posix-run" {
        dry_run
            .arg(format!("FXFS_DISK={}", disk.display()))
            .arg("MAKE=true");
    }
    let dry_output = dry_run.output().expect("dry-run POSIX Make target");
    assert!(dry_output.status.success(), "{target} dry-run failed");
    let dry_stdout = String::from_utf8(dry_output.stdout).expect("UTF-8 dry-run output");

    assert!(
        !injected.exists()
            && !substituted.exists()
            && arguments.get(flag_index + 1) == Some(expected_value)
            && arguments.iter().filter(|argument| *argument == flag).count() == 1
            && !dry_stdout.contains(&injected.to_string_lossy().to_string())
            && !dry_stdout.contains(&substituted.to_string_lossy().to_string()),
        "unsafe {target} value handling: injected={} substituted={} argv={arguments:?} dry-run={dry_stdout:?}",
        injected.exists(),
        substituted.exists(),
    );
}

fn compile_build_script() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new("build-script-contract");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let binary = temp.0.join("build-script");
    let output = std::process::Command::new("rustc")
        .arg(repository.join("build.rs"))
        .arg("--edition=2021")
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile build.rs");
    assert!(
        output.status.success(),
        "build.rs compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (temp, binary)
}

fn run_build_script(binary: &std::path::Path, root: &std::path::Path, flags: &str) -> String {
    let manifest = root.join("manifest");
    let out_dir = root.join("out");
    std::fs::create_dir_all(&manifest).expect("create manifest directory");
    std::fs::create_dir_all(&out_dir).expect("create output directory");
    let output = std::process::Command::new(binary)
        .env("TARGET", "aarch64-unknown-none")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .env("CARGO_MANIFEST_DIR", manifest)
        .env("OUT_DIR", out_dir)
        .output()
        .expect("run build.rs");
    assert!(
        output.status.success(),
        "build.rs failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("build.rs output is UTF-8")
}

#[test]
fn checked_end_helpers_share_boundary_semantics() {
    let cases = [
        (0usize, 0usize, Some(0usize)),
        (0, 1, Some(1)),
        (usize::MAX, 0, Some(usize::MAX)),
        (usize::MAX, 1, None),
        (usize::MAX - 4, 4, Some(usize::MAX)),
        (usize::MAX - 4, 5, None),
    ];

    for (addr, len, expected) in cases {
        assert_eq!(syscall_address_logic::checked_end(addr, len), expected);
        assert_eq!(kernel_object_logic::checked_end(addr, len), expected);
        assert_eq!(lowlevel_logic::checked_end(addr, len), expected);
        assert_eq!(user_logic::checked_end(addr, len), expected);
    }
}

#[test]
fn range_overlap_helpers_agree_on_touching_and_overflowing_ranges() {
    let cases = [
        (10usize, 5usize, 14usize, 2usize, true),
        (10, 5, 15, 2, false),
        (10, 0, 10, 0, false),
        (usize::MAX - 1, 4, 0, 8, false),
    ];

    for (start_a, len_a, start_b, len_b, expected) in cases {
        assert_eq!(
            syscall_address_logic::range_overlaps(start_a, len_a, start_b, len_b),
            expected
        );
        assert_eq!(
            kernel_object_logic::ranges_overlap(start_a, len_a, start_b, len_b),
            expected
        );
    }
}

#[test]
fn fifo_and_socket_ring_helpers_have_the_same_contract() {
    for capacity in [0usize, 1, 4, 8] {
        for read_pos in [0usize, 3, usize::MAX] {
            for offset in [0usize, 1, 7, usize::MAX] {
                assert_eq!(
                    fifo_logic::ring_index(read_pos, offset, capacity),
                    socket_logic::ring_index(read_pos, offset, capacity)
                );
            }
        }
    }

    for (len, capacity, expected) in [(0usize, 4usize, 4usize), (3, 4, 1), (4, 4, 0), (5, 4, 0)] {
        assert_eq!(fifo_logic::remaining_capacity(len, capacity), expected);
        assert_eq!(socket_logic::remaining_capacity(len, capacity), expected);
    }

    for (left, right, expected) in [(0usize, 3usize, 0usize), (7, 3, 3), (5, 5, 5)] {
        assert_eq!(fifo_logic::min_count(left, right), expected);
        assert_eq!(socket_logic::min_count(left, right), expected);
    }
}

#[test]
fn syscall_routing_boundaries_match_known_interface_windows() {
    let zircon_base = 1000u64;

    for syscall_num in [0u64, 446, 447, 600, 999] {
        assert!(syscall_bridge_logic::is_linux_syscall_number(syscall_num));
    }
    assert!(!syscall_bridge_logic::is_linux_syscall_number(zircon_base));

    assert!(syscall_logic::linux_syscall_interface_known(0));
    assert!(syscall_logic::linux_syscall_interface_known(446));
    assert!(!syscall_logic::linux_syscall_interface_known(447));
    assert!(syscall_logic::linux_syscall_interface_known(600));
    assert!(!syscall_logic::linux_syscall_interface_known(999));

    assert!(syscall_logic::is_zircon_syscall_number(
        zircon_base,
        zircon_base
    ));
    assert_eq!(
        syscall_logic::zircon_syscall_from_raw(zircon_base, zircon_base),
        0
    );
    assert!(syscall_logic::is_zircon_syscall_number(
        zircon_base + u32::MAX as u64,
        zircon_base
    ));
    assert!(!syscall_logic::is_zircon_syscall_number(
        zircon_base + u32::MAX as u64 + 1,
        zircon_base
    ));

    for syscall_num in [0u32, 154, 183, 211] {
        assert!(syscall_logic::zircon_syscall_interface_known(syscall_num));
        assert!(syscall_logic::is_zircon_syscall_number(
            zircon_base + syscall_num as u64,
            zircon_base
        ));
        assert_eq!(
            syscall_logic::zircon_syscall_from_raw(zircon_base + syscall_num as u64, zircon_base),
            syscall_num
        );
    }
    assert!(!syscall_logic::zircon_syscall_interface_known(155));
    assert!(!syscall_logic::zircon_syscall_interface_known(212));
}

#[test]
fn signal_update_contract_is_shared_between_syscall_and_kernel_objects() {
    let cases = [
        (0b1111u32, 0b0101u32, 0b1000u32, 0b1010u32),
        (0b0000, 0b1111, 0b0011, 0b0011),
        (0b1010, 0b0010, 0b0001, 0b1001),
    ];

    for (current, clear_mask, set_mask, expected) in cases {
        assert_eq!(
            syscall_logic::signal_update(current, clear_mask, set_mask),
            expected
        );
        assert_eq!(
            kernel_object_logic::signal_update(current, clear_mask, set_mask),
            expected
        );
    }
}

#[test]
fn elf_mapping_ranges_feed_fixed_mmap_window_checks() {
    let page_size = 0x1000usize;
    let mapping = user_logic::elf_segment_mapping_range(0x1234, 0x1800, page_size);

    assert_eq!(mapping, Some((0x1000, 0x3000)));
    let (start, end) = mapping.unwrap();
    assert!(syscall_address_logic::fixed_mmap_request_ok(
        start,
        end - start,
        page_size,
        0x1000,
        0x4000
    ));
    assert!(!syscall_address_logic::fixed_mmap_request_ok(
        start + 1,
        end - start,
        page_size,
        0x1000,
        0x4000
    ));
}

#[test]
fn linker_script_selection_is_single_source_for_nested_worktrees() {
    let cargo_config = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.cargo/config.toml"
    ));
    let build_script = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../build.rs"));

    assert!(
        !cargo_config.contains("link-arg=-Tlinker/"),
        "Cargo rustflag arrays concatenate when a linked worktree is nested under another checkout"
    );
    for (target, script) in [
        ("aarch64-unknown-none", "linker/kernel.ld"),
        ("riscv64gc-unknown-none-elf", "linker/kernel-riscv64.ld"),
        ("x86_64-unknown-none", "linker/kernel-x86_64.ld"),
    ] {
        let mapping = format!("\"{target}\" => Some(\"{script}\")");
        assert_eq!(
            build_script.matches(&mapping).count(),
            1,
            "{target} must select exactly one linker script"
        );
    }
    assert!(build_script.contains("CARGO_ENCODED_RUSTFLAGS"));
}

#[test]
fn build_script_recognizes_supported_linker_script_flag_forms() {
    let (temp, binary) = compile_build_script();
    let custom_script_flags = [
        "-C\x1flink-arg=-Tcustom.ld",
        "-Clink-arg=-Tcustom.ld",
        "-C\x1flink-args=-T custom.ld",
        "-Clink-args=-Tcustom.ld",
        "-C\x1flink-args=--script custom.ld",
        "-Clink-arg=--script=custom.ld",
        "-C\x1flink-arg=-Wl,-T,custom.ld",
        "-Clink-arg=-Wl,--script,custom.ld",
    ];
    for flags in custom_script_flags {
        let stdout = run_build_script(&binary, &temp.0, flags);
        assert!(
            !stdout.contains("cargo:rustc-link-arg=-Tlinker/kernel.ld"),
            "default script emitted for {flags:?}"
        );
    }

    for flags in [
        "-C\x1flink-arg=-Ttext=0x40200000",
        "-Clink-args=-Ttext-segment 0x40200000",
        "-C\x1flink-arg=-Tdata=0x44000000",
        "-Clink-args=-Tbss 0x48000000",
        "-C\x1flink-arg=-Wl,-Ttext,0x40200000",
        "-Clink-arg=-Wl,-Ttext-segment,0x40200000",
        "-C\x1flink-arg=-Wl,-Tdata,0x44000000",
        "-Clink-arg=-Wl,-Tbss=0x48000000",
        "-C\x1flink-arg=-Trodata-segment=0x44000000",
        "-Clink-args=-Trodata-segment 0x44000000",
        "-C\x1flink-arg=-Tldata-segment=0x48000000",
        "-Clink-args=-Tldata-segment 0x48000000",
        "-C\x1flink-arg=-Wl,-Trodata-segment,0x44000000",
        "-Clink-arg=-Wl,-Tldata-segment=0x48000000",
        "-C\x1flink-arg=--defsym=NOT-TARGET=1",
        "-C\x1flink-args=-z notext --trace",
        "-Ctarget-feature=+neon",
    ] {
        let stdout = run_build_script(&binary, &temp.0, flags);
        assert!(
            stdout.contains("cargo:rustc-link-arg=-Tlinker/kernel.ld"),
            "default script suppressed for unrelated flags {flags:?}"
        );
    }
}

#[test]
fn test_layer_commands_and_docs_are_wired() {
    let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"));
    let docs = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/TESTING.md"
    ));
    let smoke = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/smoke-qemu.sh"
    ));

    assert!(makefile.contains("ut:\n\t@./scripts/run-host-unit-tests.sh --lib"));
    assert!(
        makefile.contains("it:\n\t@./scripts/run-host-unit-tests.sh --test integration_contracts")
    );
    assert!(makefile
        .contains("linker-layout-test:\n\t@python3 scripts/test-check-aarch64-link-layout.py"));
    assert!(makefile.contains("python3 scripts/check-aarch64-link-layout.py '$(BUILD_DIR)/smros'"));
    assert!(makefile.contains(
        "test: host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test build-test"
    ));

    assert!(docs.contains("make ut"));
    assert!(docs.contains("make it"));
    assert!(docs.contains("SMROS_ST_REQUIRED_PATTERNS"));

    assert!(smoke.contains("SMROS_ST_REQUIRED_PATTERNS"));
    assert!(smoke.contains("[INFO] Fast boot complete. Starting shell"));
    assert!(smoke.contains("smros:/>"));
}

#[test]
fn posix_make_targets_are_explicit_and_keep_the_default_suite_offline() {
    let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"));
    let targets = [
        "posix-tool-test",
        "posix-fetch",
        "posix-audit",
        "posix-build",
        "posix-stage",
        "posix-baseline",
        "posix-run",
        "posix-report",
    ];

    let phony = makefile
        .lines()
        .find(|line| line.starts_with(".PHONY:"))
        .expect("Makefile .PHONY declaration");
    for target in targets {
        assert!(phony.split_whitespace().any(|word| word == target));
        assert!(
            makefile.lines().any(|line| {
                line.strip_suffix(':') == Some(target)
                    || line
                        .strip_prefix(&format!("{target}: "))
                        .is_some_and(|dependencies| !dependencies.is_empty())
            }),
            "missing recipe target {target}"
        );
    }

    assert!(makefile.contains("POSIX_QEMU_MEMORY ?= 1024M"));
    assert!(makefile.contains("AARCH64_SYSROOT ?= /usr/aarch64-linux-gnu"));
    assert!(makefile.contains(
        "posix-tool-test:\n\t@PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/posix/tests -v"
    ));
    assert!(makefile
        .contains("posix-fetch:\n\t@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli fetch"));
    assert!(makefile.contains("posix-audit: posix-fetch"));
    assert!(makefile.contains("posix-build: posix-audit"));
    assert!(makefile.contains("posix-stage: posix-build"));
    assert!(makefile.contains("posix-baseline: posix-stage"));
    assert!(makefile.contains("--sysroot \"$${AARCH64_SYSROOT}\""));
    assert!(makefile.contains("posix-run: posix-stage $(FXFS_DISK)"));
    assert!(makefile.contains("--qemu-memory \"$${POSIX_QEMU_MEMORY}\""));
    assert!(makefile.contains("POSIX_QUALITY_EVIDENCE"));
    assert!(makefile.contains("--quality-evidence"));

    let test_dependencies = makefile
        .lines()
        .find_map(|line| line.strip_prefix("test: "))
        .expect("test target dependencies");
    assert!(test_dependencies
        .split_whitespace()
        .any(|word| word == "posix-tool-test"));
    for excluded in targets
        .into_iter()
        .filter(|target| *target != "posix-tool-test")
    {
        assert!(
            !test_dependencies
                .split_whitespace()
                .any(|word| word == excluded),
            "default test target must not depend on {excluded}"
        );
    }
}

#[test]
fn posix_baseline_make_value_is_shell_safe() {
    assert_posix_make_value_is_shell_safe("posix-baseline", "AARCH64_SYSROOT", "--sysroot");
}

#[test]
fn posix_run_make_value_is_shell_safe() {
    assert_posix_make_value_is_shell_safe("posix-run", "POSIX_QEMU_MEMORY", "--qemu-memory");
}

#[test]
fn posix_report_make_value_is_shell_safe() {
    assert_posix_make_value_is_shell_safe(
        "posix-report",
        "POSIX_QUALITY_EVIDENCE",
        "--quality-evidence",
    );
}

#[test]
fn posix_conformance_workflow_and_limitations_are_documented() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide = std::fs::read_to_string(repository.join("docs/POSIX_CONFORMANCE.md"))
        .expect("read POSIX conformance guide");
    let testing =
        std::fs::read_to_string(repository.join("docs/TESTING.md")).expect("read testing guide");
    let shell =
        std::fs::read_to_string(repository.join("docs/USER_SHELL.md")).expect("read shell guide");
    let readme = std::fs::read_to_string(repository.join("README.md")).expect("read README");

    for text in [&guide, &testing, &shell, &readme] {
        assert!(
            text.contains("docs/POSIX_CONFORMANCE.md") || text.contains("POSIX_CONFORMANCE.md")
        );
        assert!(text.contains("infrastructure") && text.contains("failure baseline"));
        assert!(
            text.contains("not POSIX certification") || text.contains("not a POSIX certification")
        );
    }

    for required in [
        "IEEE 1003.1-2001 System Interfaces",
        "AArch64, then x86_64, then RISC-V64",
        "Every optional group is required",
        "256 MiB",
        "identity-mapped execution",
        "modeled process state",
        "incomplete VFS, signals, and threads",
        "Open POSIX Test Suite evidence is not IEEE or Open Group certification",
        "Direct Rust and model tests never count as POSIX passes",
        "Quality evidence text rejects all Unicode C0/C1 control characters",
        "including tab, newline, and carriage return",
        "quality evidence never changes POSIX denominators",
    ] {
        assert!(
            guide.contains(required),
            "missing POSIX guide statement: {required}"
        );
    }
    for command in [
        "make posix-tool-test",
        "make posix-fetch",
        "make posix-audit",
        "make posix-build",
        "make posix-stage",
        "make posix-baseline",
        "make posix-run",
        "make posix-report",
    ] {
        assert!(
            guide.contains(command),
            "missing documented command {command}"
        );
    }
    for artifact in [
        "events.ndjson",
        "summary.json",
        "junit.xml",
        "groups.csv",
        "apis.csv",
        "report.md",
        "index.html",
    ] {
        assert!(
            guide.contains(artifact),
            "missing report artifact {artifact}"
        );
    }
    for concept in [
        "audited upstream stub",
        "reviewed file allowlist",
        "build coverage",
        "execution coverage",
        "pass coverage",
        "program completion",
        "resource evidence",
        "raw input",
        "provenance",
        "watchdog",
        "resume",
        "PTS_UNRESOLVED",
        "PTS_UNSUPPORTED",
        "PTS_UNTESTED",
    ] {
        assert!(
            guide.contains(concept),
            "missing POSIX guide concept: {concept}"
        );
    }

    let live_coverage_docs = format!("{guide}\n{shell}");
    for phrase in [
        "selection coverage",
        "apis-complete",
        "apis-pass",
        "groups-complete",
        "groups-pass",
        "every 25 completed tests",
        "does not prove POSIX compliance",
    ] {
        assert!(
            live_coverage_docs.contains(phrase),
            "missing coverage documentation: {phrase}"
        );
    }
}

#[test]
fn posix_guest_manifest_parser_is_exported_bounded_and_canonical() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let services = std::fs::read_to_string(repository.join("src/user_level/services/mod.rs"))
        .expect("read user service exports");
    let parser = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("POSIX guest manifest parser must exist");
    let producer = std::fs::read_to_string(repository.join("scripts/posix/build.py"))
        .expect("read POSIX manifest producer");

    assert!(services.contains("pub mod posix_test;"));
    assert!(services.contains("pub(crate) mod posix_test_logic_shared;"));
    assert!(parser
        .contains("pub const POSIX_MANIFEST_PATH: &str = \"/shared/posixtest/manifest.tsv\";"));
    assert!(parser.contains("pub const POSIX_MANIFEST_SCHEMA: u32 = 1;"));
    assert!(parser.contains("pub const POSIX_MANIFEST_MAX_BYTES: usize = 2 * 1024 * 1024;"));
    assert!(parser.contains("pub const POSIX_MANIFEST_MAX_TESTS: usize = 4_096;"));
    for (name, value) in [
        ("METADATA_VALUE", "1_024"),
        ("TEST_ID", "256"),
        ("GROUP", "96"),
        ("API", "96"),
        ("STAGED_PATH", "512"),
    ] {
        assert!(parser.contains(&format!(
            "pub const POSIX_MANIFEST_MAX_{name}_BYTES: usize = {value};"
        )));
        assert!(producer.contains(&format!(
            "MAX_MANIFEST_{name}_BYTES = {}",
            value.replace('_', "")
        )));
    }
    assert!(parser.contains("SMROS_POSIX_MANIFEST\\t1"));
    assert!(parser.contains("fxfs::ensure_host_share()"));
    assert!(parser.contains("fxfs::read_file(POSIX_MANIFEST_PATH"));
    assert!(parser.contains("str::from_utf8"));
    assert!(parser.contains("parse_fixed_fields::<9>(line)"));
    assert!(parser.contains("fn parse_fixed_fields<const N: usize>"));
    assert!(!parser.contains("collect::<Vec<&str>>()"));
    assert!(parser.contains("BTreeSet"));
    assert!(!parser.contains("test_ids: Vec"));
    assert!(!parser.contains("test_ids.iter()"));
    assert!(!parser.contains("test_paths: Vec"));
    assert!(parser.contains("previous.as_str().cmp(test.test_id.as_str())"));
    assert!(parser.contains("manifest_sha256"));
    assert!(parser.contains("64 ASCII zeroes"));
    assert!(parser.contains("sha256("));

    for metadata in [
        "source",
        "revision",
        "architecture",
        "compiler",
        "libc",
        "patch_sha256",
        "build_results_sha256",
        "manifest_sha256",
        "smros_commit",
    ] {
        assert!(
            parser.contains(metadata),
            "missing metadata contract {metadata}"
        );
    }
    for rejection in [
        "InvalidUtf8",
        "UnknownRowType",
        "UnknownKind",
        "UnknownDisposition",
        "MissingMetadata",
        "DuplicateMetadata",
        "DuplicateTestId",
        "DuplicateTestPath",
        "InvalidAtom",
        "InvalidPath",
        "InvalidChecksum",
        "InvalidTimeout",
        "ManifestChecksumMismatch",
    ] {
        assert!(parser.contains(rejection), "missing rejection {rejection}");
    }

    assert!(parser.contains("pub enum PosixFilter"));
    assert!(parser
        .contains("pub fn parse_filter(args: &[&str]) -> Result<PosixFilter, PosixTestError>"));
    assert!(parser.contains("pub fn load_manifest() -> Result<PosixManifest, PosixTestError>"));
    assert!(parser.contains("pub fn status_snapshot() -> PosixRunnerStatus"));
}

#[test]
fn posix_resource_snapshot_uses_authoritative_state_without_resetting_it() {
    let syscall = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall.rs"
    ));
    let compat = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/compat.rs"
    ));
    let syscall_logic = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_logic_shared.rs"
    ));
    let start = syscall
        .find("pub fn posix_resource_snapshot()")
        .expect("POSIX resource snapshot must exist");
    let body = braced_body(&syscall[start..]);
    let memory_start = syscall
        .find("fn memory_resource_counts()")
        .expect("non-initializing memory resource helper must exist");
    let memory_body = braced_body(&syscall[memory_start..]);
    let state_new_start = syscall
        .find("impl MemorySyscallState")
        .and_then(|start| {
            syscall[start..]
                .find("fn new() -> Self")
                .map(|inner| start + inner)
        })
        .expect("memory syscall state initializer must exist");
    let state_new_body = braced_body(&syscall[state_new_start..]);

    for field in [
        "processes",
        "scheduler_threads",
        "linux_mappings",
        "linux_fds",
        "linux_shared_memory",
        "kernel_handles",
        "timers",
        "ipc_objects",
        "aio_requests",
    ] {
        assert!(
            syscall.contains(&format!("pub {field}:")),
            "missing {field}"
        );
        assert!(body.contains(field), "snapshot does not populate {field}");
    }

    assert!(body.contains("process_manager().active_processes()"));
    assert!(body.contains("scheduler().active_threads()"));
    assert!(memory_body.contains("MEMORY_SYSCALL_STATE"));
    assert!(memory_body.contains(".as_ref()"));
    assert!(memory_body.contains("state.linux_mappings.len()"));
    assert!(memory_body.contains("state.linux_fds.len()"));
    assert!(memory_body.contains("state.linux_shared_memory.len()"));
    assert!(memory_body.contains("state.handles.len()"));
    assert!(memory_body.contains("logical_memory_handle_count"));
    assert!(memory_body.contains("logical_memory_handle_count(None)"));
    assert!(state_new_body.contains("MEMORY_PERMANENT_HANDLE_COUNT"));
    assert!(syscall_logic.contains("pub const MEMORY_PERMANENT_HANDLE_COUNT: usize = 1;"));
    assert!(syscall_logic.contains("pub fn logical_memory_handle_count"));
    assert!(body.contains("compat::posix_resource_counts()"));
    assert!(body.contains("memory_resource_counts()"));
    assert!(!body.contains("memory_state()"));
    assert!(body.contains("aio_requests: linux_aio_request_count()"));
    assert!(syscall.contains("fn linux_aio_request_count() -> usize"));
    assert!(syscall.contains("AIO entry points do not allocate request state"));
    assert!(!body.contains("reset"));
    assert!(compat.contains("pub fn posix_resource_counts()"));
    assert!(compat.contains("ObjectType::Timer"));
    assert!(compat.contains("ObjectType::TimerFd"));
    assert!(compat.contains("ObjectType::Semaphore"));
    assert!(compat.contains("ObjectType::MessageQueue"));
}

#[test]
fn linux_process_reset_reclaims_transient_state_without_reinitializing_global_state() {
    let syscall = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall.rs"
    ));
    let method_start = syscall
        .find("fn reset_linux_process_state(&mut self)")
        .expect("memory-state process reset");
    let method = braced_body(&syscall[method_start..]);
    let public_start = syscall
        .find("pub fn reset_linux_process_state()")
        .expect("public process reset");
    let public = braced_body(&syscall[public_start..]);

    for required in [
        "core::mem::take(&mut self.linux_mappings)",
        "MemorySyscallState::free_linux_pages(&mapping.pfns)",
        "core::mem::replace(&mut self.brk, BrkState::new())",
        "MemorySyscallState::free_linux_pages(&brk.pfns)",
        "record.attachments.clear()",
        "self.next_linux_addr = LINUX_MAPPING_BASE",
        "self.next_fd = COMPAT_FD_START",
        "self.reset_linux_container_state()",
    ] {
        assert!(
            method.contains(required),
            "missing reset operation: {required}"
        );
    }
    assert!(public.contains("sys_close(fd)"));
    assert!(public.contains("linux_timer_handles"));
    assert!(public.contains("sys_handle_close(handle)"));
    assert!(public.contains("reset_linux_signal_timer_state()"));
    assert!(!method.contains("MemorySyscallState::new()"));
    assert!(!public.contains("MEMORY_SYSCALL_STATE = None"));
}

#[test]
fn run_elf_observer_api_is_typed_environment_aware_and_compatible() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");
    let shared =
        std::fs::read_to_string(repository.join("src/user_level/services/user_logic_shared.rs"))
            .expect("read shared user logic");
    let shell = std::fs::read_to_string(repository.join("src/user_level/services/user_shell.rs"))
        .expect("read shell service");

    for declaration in [
        "pub enum RunObserver",
        "Shell,",
        "PosixTest,",
        "pub enum RunTermination",
        "Exit(i32)",
        "LaunchError(RunElfError)",
        "InfrastructureError(RunInfrastructureError)",
        "pub enum RunInfrastructureError",
        "MissingRequest",
        "pub struct RunOutcome",
        "pub path: String",
        "pub termination: RunTermination",
        "pub elapsed_ticks: u64",
        "pub fn spawn_observed(",
    ] {
        assert!(launcher.contains(declaration), "missing {declaration}");
    }
    assert!(launcher.contains("env: Vec<String>"));
    assert!(launcher.contains("observer: RunObserver"));
    assert!(launcher.contains("spawn_observed(path, argv, Vec::new(), RunObserver::Shell)"));
    assert!(shell.contains("crate::user_level::run_elf::spawn(path.clone(), argv)"));
    assert!(!shell.contains("RunObserver::PosixTest"));

    assert!(launcher.contains("LD_LIBRARY_PATH=/shared/posixtest/lib:/shared/lib:/lib"));
    assert!(launcher.contains("run_elf_environment_valid"));
    assert!(launcher.contains("run_elf_environment_effective_totals"));
    assert!(launcher.contains("run_elf_environment_source_at"));
    for limit in [
        "const RUN_ELF_MAX_ENV_ENTRIES: usize = 64;",
        "const RUN_ELF_MAX_ENV_ENTRY_BYTES: usize = 4 * 1024;",
        "const RUN_ELF_MAX_ENV_TOTAL_BYTES: usize = 32 * 1024;",
    ] {
        assert!(
            launcher.contains(limit),
            "missing environment limit {limit}"
        );
    }
    assert!(shared.contains("pub(crate) fn run_elf_environment_valid"));
    assert!(shared.contains("env[..index]"));

    let resolver_start = launcher
        .find("fn resolve_library_path(name_or_path: &str)")
        .expect("library resolver must exist");
    let resolver = braced_body(&launcher[resolver_start..]);
    let posix_lib = resolver
        .find("/shared/posixtest/lib/")
        .expect("POSIX library directory must be searched");
    let shared_lib = resolver[posix_lib + 1..]
        .find("/shared/lib/")
        .map(|offset| posix_lib + 1 + offset)
        .expect("shared library directory must be searched");
    let system_lib = resolver[shared_lib + 1..]
        .find("/lib/")
        .map(|offset| shared_lib + 1 + offset)
        .expect("system library directory must be searched");
    assert!(posix_lib < shared_lib && shared_lib < system_lib);
    assert!(launcher.contains("run_elf_library_name_valid"));
    assert!(launcher.contains("run_elf_library_search_stage"));
}

#[test]
fn run_elf_terminal_outcomes_are_dispatched_once_after_state_is_cleared() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let posix = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("read POSIX service");

    assert!(launcher.contains("RunElfStateCell"));
    assert!(launcher.contains("RunElfLifecycleState"));
    assert!(launcher.contains("RunElfActiveRequest"));
    assert!(launcher.contains("RunElfOwnedResource"));
    assert!(launcher.contains("run_elf_attach_resource_transition"));
    assert!(launcher.contains("fn with_run_state"));
    assert!(!launcher.contains("static RUN_ACTIVE"));
    assert!(!launcher.contains("static ACTIVE_RUN"));
    assert!(!launcher.contains("static RUN_RETURN_PENDING"));
    assert!(!launcher.contains("static RUN_EXIT_CODE"));
    assert!(launcher.contains("fn take_active_request("));
    assert!(launcher.contains("fn dispatch_outcome("));
    assert!(!launcher.contains("run_elf_completion_state_action"));
    assert!(launcher.contains("run_elf_start_transition"));
    assert!(launcher.contains("run_elf_prepare_return_transition"));
    assert!(launcher.contains("run_elf_take_completion_transition"));
    assert!(launcher.contains("run_elf_clear_transition"));
    assert!(launcher.contains("RunTermination::LaunchError(err)"));
    assert!(launcher.contains("RunTermination::Exit(exit_code)"));
    assert!(launcher.contains("RunTermination::InfrastructureError("));
    assert!(launcher.contains("print_infrastructure_diagnostic("));
    assert!(launcher.contains("run_elf_elapsed_ticks(request.start_tick, end_tick)"));
    assert!(launcher.contains("syscall::reset_linux_process_state()"));
    assert_eq!(
        launcher
            .matches("syscall::reset_linux_process_state()")
            .count(),
        4,
        "start, prepare-return, completion, and explicit clear must share cleanup",
    );
    assert!(!launcher.contains("syscall::reset_linux_signal_timer_state()"));
    assert!(launcher.contains("posix_test::on_run_outcome(outcome)"));

    let validation = launcher
        .find("if validate_environment(&env).is_err()")
        .expect("environment is validated");
    let publication = launcher
        .find("run_elf_start_transition(state, request")
        .expect("validated request is published");
    assert!(validation < publication);

    let take = launcher
        .find("let (completion, exit_code) = take_active_request(launch_id)")
        .expect("terminal path takes the active request");
    let dispatch = launcher[take..]
        .find("dispatch_outcome(")
        .map(|offset| take + offset)
        .expect("terminal path dispatches an outcome");
    assert!(
        take < dispatch,
        "active state must be cleared before callback"
    );

    assert_eq!(
        syscall
            .matches("crate::user_level::run_elf::prepare_run_elf_return(exit_code)")
            .count(),
        1,
        "exit and exit_group must converge on one launcher completion hook"
    );
    assert!(syscall.contains("let exit_code = syscall_logic::linux_exit_status(exit_code);"));
    assert!(syscall.contains("pub fn sys_exit_group(exit_code: i32)"));
    assert!(syscall.contains("sys_exit(exit_code)"));

    assert!(posix.contains("pub fn on_run_outcome(outcome: RunOutcome)"));
}

#[test]
fn posix_guest_terminal_events_are_line_framed_after_arbitrary_test_output() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runner = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("read POSIX guest runner");

    for emitter in ["fn emit_test_end(", "fn emit_infrastructure_error("] {
        let start = runner.find(emitter).expect("terminal event emitter");
        let body = braced_body(&runner[start..]);
        let init = body.find("serial.init();").expect("serial initialization");
        let delimiter = body
            .find("serial.write_byte(b'\\n');")
            .expect("serial line delimiter");
        let event = body.find("begin_event(").expect("structured event start");

        assert!(
            init < delimiter && delimiter < event,
            "{emitter} must write a line delimiter after serial initialization and before the event"
        );
    }
}

#[test]
fn posix_guest_runner_is_serialized_bounded_and_fail_closed() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runner = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("read POSIX guest runner");
    let shared = std::fs::read_to_string(
        repository.join("src/user_level/services/posix_test_logic_shared.rs"),
    )
    .expect("read shared POSIX decisions");

    for declaration in [
        "pub const POSIX_EVENT_PREFIX: &str = \"SMROS_POSIX_EVENT \";",
        "pub const POSIX_EVENT_SCHEMA: u32 = 1;",
        "struct RunnerStateCell(UnsafeCell<Option<RunnerState>>);",
        "static RUNNER_STATE: RunnerStateCell",
        "pub fn start(filter: PosixFilter) -> Result<(), PosixTestError>",
        "AlreadyRunning",
        "EmptySelection",
        "pub status_counts: PosixStatusCounts",
    ] {
        assert!(
            runner.contains(declaration),
            "missing runner contract {declaration}"
        );
    }
    assert_eq!(
        runner
            .matches("static RUNNER_STATE: RunnerStateCell")
            .count(),
        1,
        "only one POSIX run state may exist"
    );

    let filter_start = runner
        .find("fn test_matches_filter(")
        .expect("exact manifest filter helper");
    let filter_body = braced_body(&runner[filter_start..]);
    assert!(filter_body.contains("posix_test_logic_shared::filter_matches("));
    assert!(filter_body.contains("PosixFilterKind::All"));
    assert!(filter_body.contains("PosixFilterKind::Group"));
    assert!(filter_body.contains("PosixFilterKind::Api"));
    assert!(filter_body.contains("PosixFilterKind::Test"));
    assert!(!filter_body.contains("contains("));
    assert!(!filter_body.contains("starts_with("));
    assert!(shared.contains("PosixFilterKind::All => $runnable && $complete"));
    assert!(shared.contains("PosixFilterKind::Group => $value == $group"));
    assert!(shared.contains("PosixFilterKind::Api => $value == $api"));
    assert!(shared.contains("PosixFilterKind::Test => $value == $test_id"));

    let action_start = runner
        .find("fn selected_test_action(")
        .expect("selected-test disposition helper");
    let action_body = braced_body(&runner[action_start..]);
    assert!(action_body.contains("PosixTestKind::Definition"));
    assert!(action_body.contains("PosixDisposition::ExcludedUpstreamStub"));
    assert!(action_body.contains("SelectedTestAction::EmitWithoutLaunch"));
    assert!(action_body.contains("SelectedTestAction::Launch"));
    assert!(action_body.contains("PosixDisposition::Complete"));
    assert!(!action_body.contains("spawn_observed"));

    let launch_start = runner
        .find("fn launch_current_test(harness_launcher_active: bool)")
        .expect("runner launch helper");
    let launch_body = braced_body(&runner[launch_start..]);
    assert!(launch_body.contains("run_elf::spawn_observed("));
    assert!(launch_body.contains("RunObserver::PosixTest"));
    assert!(launch_body.contains("RunTermination::LaunchError(err)"));
    assert!(launch_body.contains("loop {"));
    assert!(launch_body.contains("record_run_outcome("));
    assert!(!launch_body.contains("on_run_outcome("));
    assert!(launch_body.contains("binary_path.as_ref()"));
    assert!(launch_body.contains("infrastructure_error"));
    assert!(launch_body.contains("resource_snapshot(harness_launcher_active)"));
    assert!(launch_body.contains("record_unlaunched_test(harness_launcher_active)"));
    assert!(launch_body.contains("record_run_outcome(&outcome, harness_launcher_active)"));
    assert!(!launch_body.contains("RunTermination::Exit(5)"));
    assert!(
        !launch_body.contains("status: \"pass\"") && !launch_body.contains("\"pass\""),
        "a missing binary or launch failure must never become a pass"
    );

    assert!(runner.contains("launch_current_test(false);"));
    let callback_start = runner
        .find("pub fn on_run_outcome(outcome: RunOutcome)")
        .expect("POSIX completion callback");
    let callback_body = braced_body(&runner[callback_start..]);
    let record = callback_body
        .find("record_run_outcome(&outcome, true)")
        .expect("callback normalizes the active harness launcher");
    let next = callback_body
        .find("launch_current_test(true)")
        .expect("callback carries the active launcher into the next test");
    assert!(record < next);

    for recorder in ["fn record_unlaunched_test(", "fn record_run_outcome("] {
        let start = runner.find(recorder).expect("result recorder");
        let body = braced_body(&runner[start..]);
        assert!(body.contains("resource_snapshot(harness_launcher_active)"));
    }

    for contract in [
        "coverage: PosixCoverageTracker",
        "emit_selection_summary(state);",
        "fn emit_progress(",
        "posixtest: selection tests=",
        " apis=",
        " groups=",
        " interval=",
        " scope=selected",
        "posixtest: progress tests=",
        " apis-complete=",
        " apis-pass=",
        " groups-complete=",
        " groups-pass=",
        " launch-errors=",
        "should_emit_progress(",
    ] {
        assert!(
            runner.contains(contract),
            "missing live coverage contract {contract}"
        );
    }
    assert_eq!(
        runner
            .matches("pub const POSIX_EVENT_SCHEMA: u32 = 1;")
            .count(),
        1
    );

    for (recorder, terminal_event) in [
        ("fn record_unlaunched_test(", "emit_unlaunched_test_end("),
        ("fn record_run_outcome(", "emit_test_end("),
    ] {
        let start = runner.find(recorder).expect("coverage result recorder");
        let body = braced_body(&runner[start..]);
        let event = body.find(terminal_event).expect("terminal event emission");
        let coverage = body
            .find("state.coverage.record(")
            .expect("coverage result recording");
        let progress = body.find("emit_progress(").expect("progress emission");
        assert!(event < coverage && coverage < progress);
    }

    let finish_start = runner.find("fn finish_suite()").expect("suite finisher");
    let finish_body = braced_body(&runner[finish_start..]);
    let invariant = finish_body
        .find("tests_completed == state.selected.len()")
        .expect("suite coverage completion invariant");
    let suite_end = finish_body
        .find("emit_suite_end(state)")
        .expect("suite end emission");
    assert!(invariant < suite_end);

    assert!(runner.contains("\"pts_status\":null,\"launch_status\":\"not-launched\""));
    let unlaunched_start = runner
        .find("fn emit_unlaunched_test_end(")
        .expect("unlaunched event emitter");
    let unlaunched_body = braced_body(&runner[unlaunched_start..]);
    for execution_or_error_field in [
        "exit_code",
        "signal",
        "timed_out",
        "launch_error",
        "infrastructure_error",
        "RunOutcome",
        "RunTermination",
    ] {
        assert!(!unlaunched_body.contains(execution_or_error_field));
    }
    assert!(shared.contains("pub fn normalize_scheduler_threads("));
}

#[test]
fn posix_guest_events_match_the_versioned_host_schema() {
    let runner = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/posix_test.rs"
    ));
    let events = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/posix/events.py"
    ));

    for event in [
        "suite_start",
        "test_start",
        "test_end",
        "suite_end",
        "infrastructure_error",
    ] {
        assert!(runner.contains(event), "guest does not emit {event}");
        assert!(events.contains(&format!("\"{event}\"")));
    }
    for common in [
        "schema",
        "seq",
        "event",
        "run_id",
        "manifest_sha256",
        "architecture",
    ] {
        assert!(
            runner.contains(&format!("\\\"{common}\\\"")),
            "missing {common}"
        );
    }
    for test_field in [
        "test_id",
        "group",
        "api",
        "status",
        "exit_code",
        "launch_error",
        "elapsed_ticks",
        "resource_deltas",
    ] {
        assert!(
            runner.contains(&format!("\\\"{test_field}\\\"")),
            "missing test event field {test_field}"
        );
    }
    for resource in [
        "aio_requests",
        "ipc_objects",
        "kernel_handles",
        "linux_fds",
        "linux_mappings",
        "linux_shared_memory",
        "processes",
        "scheduler_threads",
        "timers",
    ] {
        assert!(runner.contains(&format!("\"{resource}\"")));
    }

    assert!(runner.contains("fn write_json_string("));
    assert!(!runner.contains("fn write_filter_value("));
    assert!(runner.contains("b'\\\"' | b'\\\\'"));
    assert!(runner.contains("fn derive_build_id("));
    for provenance in [
        "build_results_sha256",
        "manifest_sha256",
        "patch_sha256",
        "revision",
        "smros_commit",
    ] {
        assert!(runner.contains(provenance));
    }
    for pts in [
        "POSIX_STATUS_PASS => PosixRuntimeStatus::Pass",
        "POSIX_STATUS_FAIL => PosixRuntimeStatus::Fail",
        "POSIX_STATUS_UNRESOLVED => PosixRuntimeStatus::Unresolved",
        "POSIX_STATUS_UNSUPPORTED => PosixRuntimeStatus::Unsupported",
        "POSIX_STATUS_UNTESTED => PosixRuntimeStatus::Untested",
    ] {
        assert!(runner.contains(pts), "missing PTS status mapping {pts}");
    }
    assert!(runner.contains("posix_test_logic_shared::pts_status(exit_code)"));
    assert!(runner.contains("posix_resource_snapshot()"));
    assert!(runner.contains("posix_test_logic_shared::resource_delta("));
    assert!(runner.contains("fn write_i128("));
    assert!(!runner.contains("fn signed_delta("));
    assert!(runner.contains("status_counts"));
}

#[test]
fn posix_test_shell_command_is_strictly_wired_to_the_runner() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let shell = std::fs::read_to_string(repository.join("src/user_level/services/user_shell.rs"))
        .expect("read shell service");
    let runner = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("read POSIX runner");

    assert!(shell.contains(
        "name: \"posixtest\",\n        description: \"Run Open POSIX Test Suite manifest cases\",\n        handler: cmd_posix_test,"
    ));

    let handler_start = shell
        .find("fn cmd_posix_test(")
        .expect("posixtest handler must exist");
    let handler = braced_body(&shell[handler_start..]);
    assert!(handler.contains("[\"status\"]"));
    assert!(handler.contains("posix_test::status_snapshot()"));
    assert!(handler.contains("posix_test::parse_filter(args)"));
    assert!(handler.contains("posix_test::start(filter)"));
    for field in [
        " tests=",
        " apis-complete=",
        " apis-pass=",
        " groups-complete=",
        " groups-pass=",
        " scope=selected",
    ] {
        assert!(handler.contains(field), "status omits {field}");
    }
    assert!(handler.contains("status.coverage"));
    assert_eq!(
        handler
            .matches(
                "usage: posixtest all | group <group> | api <api> | test <test-id> | status\\n"
            )
            .count(),
        1,
        "invalid forms must converge on one usage line"
    );
    for output in [
        "posixtest: busy",
        "posixtest: manifest unavailable",
        "posixtest: manifest checksum/schema invalid",
        "posixtest: empty selection",
        "posixtest: launch-error",
        "posixtest: infrastructure-error",
        "posixtest: completed",
        "launch_errors=",
    ] {
        assert!(handler.contains(output), "missing distinct output {output}");
    }

    let parser_start = runner
        .find("pub fn parse_filter(")
        .expect("runner filter parser");
    let parser = braced_body(&runner[parser_start..]);
    for exact_form in [
        "[\"all\"]",
        "[\"group\", value]",
        "[\"api\", value]",
        "[\"test\", value]",
    ] {
        assert!(
            parser.contains(exact_form),
            "missing exact form {exact_form}"
        );
    }
    assert!(parser.contains("_ => Err(PosixTestError::InvalidFilter)"));

    assert!(runner.contains("LaunchError,"));
    assert!(runner.contains("InfrastructureError,"));
    assert!(runner.contains("PosixTestError::LaunchError => \"launch-error\""));
    assert!(runner.contains("PosixTestError::InfrastructureError => \"infrastructure-error\""));
    assert!(runner.contains("enum PosixLaunchLoopResult"));
    for variant in [
        "Running(usize)",
        "Completed(usize)",
        "InfrastructureError(usize)",
    ] {
        assert!(runner.contains(variant), "missing launch result {variant}");
    }
    let launch_start = runner
        .find("fn launch_current_test(")
        .expect("runner launch loop");
    let launch = braced_body(&runner[launch_start..]);
    assert!(
        runner[launch_start..launch_start + runner[launch_start..].find('{').unwrap()]
            .contains("-> PosixLaunchLoopResult")
    );
    assert!(launch.contains("synchronous_launch_errors"));
    assert!(launch.contains("saturating_add(1)"));
    assert!(launch.contains("runner-state-missing"));
    assert_eq!(
        launch
            .matches("return PosixLaunchLoopResult::InfrastructureError(")
            .count(),
        4,
        "every launch-loop invariant exit must stay distinct from completion"
    );

    let start_start = runner
        .find("pub fn start(filter: PosixFilter)")
        .expect("runner start");
    let start = braced_body(&runner[start_start..]);
    assert!(start.contains("let launch_result = launch_current_test(false)"));
    assert!(start.contains("start_result_after_launch(launch_result)"));

    let ok_start = handler.find("Ok(()) =>").expect("successful start branch");
    let ok = braced_body(&handler[ok_start..]);
    assert!(ok.contains("let status = posix_test::status_snapshot()"));
    assert!(ok.contains("status.status_counts.launch_errors > 0"));
    assert!(ok.contains("posixtest: launch-error count="));
    let running_guard = ok.find("if status.running").expect("active runner guard");
    let yield_now = ok
        .find("scheduler::yield_now()")
        .expect("active runner yields");
    assert!(running_guard < yield_now);
    assert!(handler.contains("Err(PosixTestError::InfrastructureError)"));
}

#[test]
fn shell_yields_before_waiting_for_uart_activity() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let shell = std::fs::read_to_string(repository.join("src/user_level/services/user_shell.rs"))
        .expect("read user shell");
    let read_start = shell
        .find("fn read_uart_byte() -> u8")
        .expect("UART read loop");
    let read = braced_body(&shell[read_start..]);
    let probe = read.find("Self::try_read_uart_byte()").expect("UART probe");
    let yield_now = read
        .find("scheduler::yield_now();")
        .expect("scheduler yield");
    let wait = read
        .find("crate::kernel_lowlevel::cpu::wait_for_event();")
        .expect("low-power UART wait");
    assert!(probe < yield_now && yield_now < wait);
}

#[test]
fn run_elf_launch_identity_is_bound_and_carried_through_aarch64_resume() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");
    let shared =
        std::fs::read_to_string(repository.join("src/user_level/services/user_logic_shared.rs"))
            .expect("read shared user logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let aarch64 = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/boot.rs"))
        .expect("read AArch64 exception entry");

    assert!(shared.contains("struct RunElfLaunchId"));
    assert!(shared.contains("enum RunElfStart"));
    assert!(shared.contains("enum RunElfTransition"));
    assert!(shared.contains("struct RunElfCpuBindings"));

    let from_raw_start = shared
        .find("fn from_raw(raw: u64)")
        .expect("launch IDs expose checked raw conversion");
    let from_raw = braced_body(&shared[from_raw_start..]);
    assert!(
        from_raw.contains("NonZeroU64::new(raw)")
            || (from_raw.contains("raw == 0") && from_raw.contains("None")),
        "raw launch-ID conversion must reject zero"
    );

    let from_usize_start = shared
        .find("fn from_usize(raw: usize)")
        .expect("launch IDs expose checked usize conversion");
    let from_usize = braced_body(&shared[from_usize_start..]);
    assert!(from_usize.contains("usize::BITS"));
    assert!(from_usize.contains("64"));
    assert!(
        from_usize.contains("None"),
        "non-64-bit usize conversion must fail closed"
    );

    for transition in [
        "fn request_for(",
        "fn run_elf_prepare_return_transition",
        "fn run_elf_take_completion_transition",
        "fn run_elf_clear_transition",
        "fn run_elf_attach_resource_transition",
    ] {
        let start = shared.find(transition).expect("ID-aware transition exists");
        let signature_end = shared[start..]
            .find('{')
            .map(|offset| start + offset)
            .expect("transition signature ends");
        assert!(
            shared[start..signature_end].contains("RunElfLaunchId"),
            "{transition} must require an expected launch ID"
        );
    }

    let create = launcher
        .find("create_thread_on_cpu(")
        .expect("ELF launcher uses pinned thread creation");
    let bind = launcher[..create]
        .rfind(".bind(")
        .expect("launch ID is bound before thread creation");
    assert!(bind < create);
    let create_call = &launcher[create..launcher.len().min(create + 400)];
    assert!(create_call.contains("run_elf_launcher_entry"));
    assert!(create_call.contains("Some(cpu)"));

    for expected_id_call in [
        "request_for(launch_id)",
        "run_elf_prepare_return_transition(state, launch_id,",
        "run_elf_take_completion_transition(state, launch_id,",
        "run_elf_clear_transition(state, launch_id,",
        "run_elf_attach_resource_transition(state, launch_id,",
    ] {
        assert!(
            launcher.contains(expected_id_call),
            "launcher is missing expected-ID call {expected_id_call}"
        );
    }

    let resume_start = launcher
        .find("pub extern \"C\" fn run_elf_launcher_resume(id_raw: usize) -> !")
        .expect("resume ABI carries the raw launch ID in x0");
    let resume = braced_body(&launcher[resume_start..]);
    assert!(resume.contains("RunElfLaunchId::from_usize(id_raw)"));

    let sys_exit_start = syscall
        .find("pub fn sys_exit(exit_code: i32)")
        .expect("sys_exit");
    let sys_exit = braced_body(&syscall[sys_exit_start..]);
    assert!(sys_exit.contains("if let Some(launch_id)"));
    assert!(sys_exit.contains("prepare_run_elf_return(exit_code)"));
    assert!(sys_exit.contains("return Ok(launch_id)"));

    let exception_start = aarch64
        .find("exception_handler:")
        .expect("AArch64 synchronous exception handler");
    let exception = &aarch64[exception_start..];
    let dispatch = exception
        .find("bl      handle_syscall_simple")
        .expect("AArch64 syscall dispatch");
    let save_result = exception[dispatch..]
        .find("str     x0, [sp, #0]")
        .map(|offset| dispatch + offset)
        .expect("syscall result is saved as resume x0");
    let complete_signal = exception[save_result..]
        .find("bl      complete_linux_signal_syscall_return")
        .map(|offset| save_result + offset)
        .expect("pending Linux signals complete before register restoration");
    let restore_result = exception[save_result..]
        .find("ldp     x0, x1, [sp, #0]")
        .map(|offset| save_result + offset)
        .expect("saved resume x0 is restored");
    let eret = exception[restore_result..]
        .find("eret")
        .map(|offset| restore_result + offset)
        .expect("exception return resumes launcher");
    assert!(
        dispatch < save_result
            && save_result < complete_signal
            && complete_signal < restore_result
            && restore_result < eret
    );
}

#[test]
fn scheduler_reclaims_thread_stacks_only_after_a_confirmed_context_switch() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scheduler = std::fs::read_to_string(repository.join("src/kernel_objects/scheduler.rs"))
        .expect("read scheduler");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_objects/scheduler_logic_shared.rs"))
            .expect("read shared scheduler lifecycle logic");

    assert!(shared.contains("struct DeferredThreadRetirements"));
    assert!(shared.contains("record_before_switch"));
    assert!(shared.contains("confirm_after_switch"));
    assert!(shared.contains("take_reclaimable"));
    assert!(shared.contains("DeallocateAndReuse"));
    assert!(shared.contains("has_stack_pointer != has_stack_size"));

    for function in ["pub fn schedule()", "pub fn schedule_on_cpu(cpu_id: usize)"] {
        let start = scheduler.find(function).expect("context switch function");
        let body = braced_body(&scheduler[start..]);
        assert!(body.contains(
            "let executing_cpu = crate::kernel_lowlevel::smp::current_cpu_id() as usize;"
        ));
        assert!(body.contains("reap_deferred_thread_for_cpu(executing_cpu)"));
        let masked = body
            .find("crate::kernel_lowlevel::cpu::mask_interrupts()")
            .expect("local interrupts are masked before retirement publication");
        let deferred = body
            .find("defer_terminated_thread_before_switch(executing_cpu, current_id)")
            .expect("outgoing terminated thread is deferred");
        let switched = body
            .find("thread::switch_context")
            .expect("context switch occurs");
        assert!(masked < deferred && deferred < switched);
    }

    assert!(scheduler.contains("self.reap_deferred_thread_for_cpu(current_cpu);"));
    let terminate_start = scheduler
        .find("pub fn terminate_thread(")
        .expect("targeted termination API");
    let terminate = braced_body(&scheduler[terminate_start..]);
    let current = terminate
        .find("if defer_current")
        .expect("current-thread retirement branch");
    let stack_capture = terminate
        .find("let stack = self.threads[id.0].stack.0;")
        .expect("non-current stack capture");
    assert!(current < stack_capture);
    assert!(!terminate[current..stack_capture].contains(".stack ="));
    let unreachable = terminate
        .find("self.threads[id.0] = ThreadControlBlock::new();")
        .expect("non-current TCB reset");
    let deallocate = terminate
        .find("alloc::alloc::dealloc(stack, layout)")
        .expect("non-current stack deallocation");
    assert!(stack_capture < unreachable && unreachable < deallocate);
    assert!(!scheduler.contains("fn reap_terminated_threads("));
}

#[test]
fn linux_child_exit_clears_tid_and_uses_deferred_stack_retirement() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let futex = std::fs::read_to_string(repository.join("src/syscall/linux_futex.rs"))
        .expect("read Linux futex runtime");
    let scheduler = std::fs::read_to_string(repository.join("src/kernel_objects/scheduler.rs"))
        .expect("read scheduler");
    let context = std::fs::read_to_string(repository.join("src/syscall/linux_syscall_context.rs"))
        .expect("read Linux syscall context");
    let context_logic = std::fs::read_to_string(
        repository.join("src/syscall/linux_syscall_context_logic_shared.rs"),
    )
    .expect("read Linux syscall context ownership model");

    let sys_exit_start = syscall
        .find("pub fn sys_exit(exit_code: i32)")
        .expect("sys_exit");
    let sys_exit = braced_body(&syscall[sys_exit_start..]);
    let child_check = sys_exit
        .find("linux_task::current_is_clone_child()")
        .expect("clone-child exit check");
    let child_exit = sys_exit
        .find("linux_task::exit_current(exit_code)")
        .expect("non-returning child exit");
    let launcher_exit = sys_exit
        .find("prepare_run_elf_return(exit_code)")
        .expect("root launcher completion");
    assert!(child_check < child_exit && child_exit < launcher_exit);

    let set_tid_start = syscall
        .find("pub fn sys_set_tid_address(tidptr: usize)")
        .expect("set_tid_address");
    let set_tid = braced_body(&syscall[set_tid_start..]);
    assert!(set_tid.contains("tidptr != 0"));
    assert!(set_tid.contains("linux_clone_tid_destination_valid(tidptr)"));
    assert!(set_tid.contains("linux_task::set_current_clear_child_tid(tidptr)"));

    let group_start = syscall
        .find("pub fn sys_exit_group(exit_code: i32)")
        .expect("exit_group");
    let group_exit = braced_body(&syscall[group_start..]);
    let futex_reset = group_exit
        .find("linux_futex::reset()")
        .expect("futex reset");
    let task_reset = group_exit.find("linux_task::reset()").expect("task reset");
    let shared_exit = group_exit
        .find("sys_exit(exit_code)")
        .expect("shared root exit");
    assert!(futex_reset < task_reset && task_reset < shared_exit);

    let exit_current_start = task
        .find("pub(crate) fn exit_current(exit_code: i32) -> !")
        .expect("child exit runtime");
    let exit_current = braced_body(&task[exit_current_start..]);
    let mask = exit_current
        .find("mask_interrupts()")
        .expect("interrupt mask");
    let transition = exit_current
        .find("begin_child_exit_by_scheduler")
        .expect("one-shot child exit transition");
    let remove_waiters = exit_current
        .find("linux_futex::remove_task_waiters")
        .expect("task futex cleanup");
    let retire = exit_current
        .find(".retire(transition.task.tid")
        .expect("task retirement");
    let restore = exit_current
        .find("restore_interrupts(interrupt_state)")
        .expect("interrupt restore");
    let checked_write = exit_current
        .find("linux_clone_tid_destination_valid(clear_child_tid)")
        .expect("checked clear-child-TID write");
    let zero_write = exit_current
        .find("core::ptr::write(clear_child_tid as *mut u32, 0)")
        .expect("clear-child-TID zero write");
    let wake = exit_current
        .find("linux_futex::wake_address(")
        .expect("pthread join futex wake");
    let finish = exit_current
        .find("finish_current_without_stack_free()")
        .expect("deferred current stack retirement");
    let schedule = exit_current
        .find("scheduler::schedule()")
        .expect("reschedule");
    let wait = exit_current
        .find("wait_for_interrupt()")
        .expect("no-runnable-thread wait");
    assert!(mask < transition && transition < remove_waiters && remove_waiters < retire);
    assert!(retire < checked_write && checked_write < zero_write && zero_write < restore);
    assert!(restore < wake && wake < finish && finish < schedule && schedule < wait);
    assert!(exit_current.contains("let _ = exit_code;"));

    assert!(futex.contains("pub(crate) fn remove_task_waiters("));
    assert!(futex.contains("pub(crate) fn wake_address("));
    assert!(context_logic.contains("pub(crate) fn clear_owner("));
    assert!(context.contains("pub(crate) fn retire_owner(owner: usize)"));

    let terminate_start = scheduler
        .find("pub fn terminate_thread(")
        .expect("scheduler termination");
    let terminate = braced_body(&scheduler[terminate_start..]);
    let retire_owner = terminate
        .find("linux_syscall_context::retire_owner(id.0)")
        .expect("syscall-frame owner retirement");
    let terminate_current = terminate
        .find("self.threads[id.0].state = ThreadState::Terminated")
        .expect("current scheduler termination");
    let stack_capture = terminate
        .find("let stack = self.threads[id.0].stack.0;")
        .expect("non-current stack retirement");
    assert!(retire_owner < terminate_current && retire_owner < stack_capture);
}

#[test]
fn scheduler_exposes_atomic_linux_task_transitions() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scheduler = std::fs::read_to_string(repository.join("src/kernel_objects/scheduler.rs"))
        .expect("read scheduler");
    let boot = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/boot.rs"))
        .expect("read AArch64 exception entry");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_objects/scheduler_logic_shared.rs"))
            .expect("read shared scheduler transitions");

    for api in [
        "pub fn create_suspended_thread_on_cpu(",
        "pub fn publish_suspended_thread(",
        "pub fn block_thread(",
        "pub fn wake_thread(",
        "pub fn terminate_thread(",
    ] {
        assert!(scheduler.contains(api), "missing scheduler API {api}");
    }
    assert!(shared.contains("smros_sched_terminate_transition_body"));
    let terminate_start = scheduler
        .find("pub fn terminate_thread(")
        .expect("targeted termination API");
    let terminate = braced_body(&scheduler[terminate_start..]);
    let gate = terminate
        .find("smros_sched_terminate_transition_body!")
        .expect("shared termination gate");
    let accounting = terminate
        .find("self.active_threads = next_active_threads;")
        .expect("proved one-time active-thread accounting");
    assert!(gate < accounting);
    assert_eq!(terminate.matches("self.active_threads =").count(), 1);
    assert!(!terminate.contains("saturating_sub"));

    let lower_start = boot
        .find("irq_handler_lower:")
        .expect("lower-EL timer handler");
    let lower_end = boot[lower_start..]
        .find("// Exception Handler")
        .expect("end of lower-EL timer handler");
    let lower = &boot[lower_start..lower_start + lower_end];
    let timer = lower
        .find("bl      timer_interrupt_handler")
        .expect("timer accounting");
    let signal = lower
        .find("bl      deliver_linux_timer_signal_from_irq")
        .expect("timer signal delivery");
    let preempt = lower
        .find("bl      check_preemption")
        .expect("lower-EL preemption");
    let restore = lower
        .find("// Restore registers")
        .expect("exception frame restore");
    assert!(timer < signal && signal < preempt && preempt < restore);

    let current_handlers = &boot[..lower_start];
    assert!(!current_handlers.contains("bl      check_preemption"));
}

#[test]
fn linux_root_task_and_syscall_frame_have_bounded_owners() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let boot = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/boot.rs"))
        .expect("read AArch64 exception entry");
    let riscv_boot =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/RISCV64/boot.rs"))
            .expect("read RISC-V exception entry");
    let dispatch = std::fs::read_to_string(repository.join("src/syscall/syscall_dispatch.rs"))
        .expect("read syscall dispatcher");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let run_elf = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let context = std::fs::read_to_string(repository.join("src/syscall/linux_syscall_context.rs"))
        .expect("read Linux syscall context");
    let context_logic = std::fs::read_to_string(
        repository.join("src/syscall/linux_syscall_context_logic_shared.rs"),
    )
    .expect("read Linux syscall context ownership model");
    let runtime_lock =
        std::fs::read_to_string(repository.join("src/syscall/linux_runtime_lock_shared.rs"))
            .expect("read Linux runtime lock");

    let exception_start = boot
        .find("exception_handler:")
        .expect("synchronous exception handler");
    let exception = &boot[exception_start..];
    let frame_arg = exception
        .find("mov     x0, sp")
        .expect("saved frame argument");
    let syscall_number = exception
        .find("ldr     x1, [sp, #64]")
        .expect("syscall number argument");
    let first_args = exception
        .find("ldp     x2, x3, [sp, #0]")
        .expect("first syscall arguments");
    let last_args = exception
        .find("ldp     x6, x7, [sp, #32]")
        .expect("last syscall arguments");
    let call = exception
        .find("bl      handle_syscall_simple")
        .expect("syscall dispatch call");
    assert!(frame_arg < syscall_number);
    assert!(syscall_number < first_args && first_args < last_args && last_args < call);

    let riscv_start = riscv_boot
        .find("trap_user_ecall:")
        .expect("RISC-V user syscall entry");
    let riscv_end = riscv_boot[riscv_start..]
        .find("trap_unknown:")
        .expect("end of RISC-V syscall entry");
    let riscv = &riscv_boot[riscv_start..riscv_start + riscv_end];
    let mut previous = 0usize;
    for instruction in [
        "mv      a0, sp",
        "ld      a1, 120(sp)",
        "ld      a2, 64(sp)",
        "ld      a3, 72(sp)",
        "ld      a4, 80(sp)",
        "ld      a5, 88(sp)",
        "ld      a6, 96(sp)",
        "ld      a7, 104(sp)",
        "call    handle_syscall_simple",
    ] {
        let offset = riscv
            .find(instruction)
            .unwrap_or_else(|| panic!("RISC-V syscall entry is missing {instruction}"));
        assert!(offset >= previous, "RISC-V syscall argument order");
        previous = offset;
    }

    assert!(dispatch.contains("saved_frame: usize"));
    assert!(dispatch.contains("linux_syscall_context::with_linux_syscall_frame("));
    let linux_branch = dispatch
        .find("if is_linux_syscall_number(syscall_num)")
        .expect("Linux dispatch branch");
    let zircon_branch = dispatch
        .find("else if is_zircon_syscall_number(syscall_num)")
        .expect("Zircon dispatch branch");
    let linux_dispatch = &dispatch[linux_branch..zircon_branch];
    assert!(linux_dispatch.contains("with_linux_syscall_frame"));
    assert!(!dispatch[zircon_branch..].contains("with_linux_syscall_frame"));

    assert!(context.contains("struct LinuxSyscallFrameRef"));
    assert!(context.contains("pub(crate) fn current() -> Option<LinuxSyscallFrameRef>"));
    assert!(context.contains("static FRAME_OWNERS: LinuxSyscallFrameOwners"));
    assert!(context.contains("let owner = scheduler::scheduler().current().0;"));
    assert!(context.contains("FRAME_OWNERS.install(owner"));
    assert!(context.contains("FRAME_OWNERS.current(owner)"));
    assert!(context.contains("FRAME_OWNERS.clear(self.owner, self.frame)"));
    assert!(context.contains("FRAME_OWNERS.clear_all()"));
    assert!(!context.contains("scheduler::MAX_CPUS"));
    assert!(!context.contains("current_cpu_id()"));
    for ownership_rule in [
        "struct LinuxSyscallFrameOwners<const N: usize>",
        "frames: [AtomicUsize; N]",
        "pub(crate) fn install(",
        "pub(crate) fn current(",
        "pub(crate) fn clear(",
        "compare_exchange(",
    ] {
        assert!(
            context_logic.contains(ownership_rule),
            "missing task-scoped frame ownership rule {ownership_rule}"
        );
    }

    assert!(task.contains("LinuxRuntimeLock<LinuxTaskRuntime>"));
    assert!(!task.contains("struct LinuxTaskRuntimeCell"));
    let runtime_start = task
        .find("fn with_runtime<R>(")
        .expect("runtime access helper");
    let runtime = braced_body(&task[runtime_start..]);
    let mask = runtime.find("mask_interrupts()").expect("interrupt mask");
    let lock = runtime
        .find("LINUX_TASK_RUNTIME.lock()")
        .expect("cross-CPU runtime lock");
    let operation = runtime
        .find("operation(&mut runtime)")
        .expect("runtime mutation");
    let unlock = runtime.find("drop(runtime)").expect("runtime unlock");
    let restore = runtime
        .find("restore_interrupts(interrupt_state)")
        .expect("interrupt restore");
    assert!(mask < lock && lock < operation && operation < unlock && unlock < restore);
    for lock_rule in [
        "locked: core::sync::atomic::AtomicBool",
        "value: core::cell::UnsafeCell<T>",
        "unsafe impl<T: Send> Sync for LinuxRuntimeLock<T>",
        "compare_exchange(",
        "core::sync::atomic::Ordering::Acquire",
        "core::sync::atomic::Ordering::Release",
    ] {
        assert!(
            runtime_lock.contains(lock_rule),
            "missing cross-CPU runtime lock rule {lock_rule}"
        );
    }
    for api in [
        "pub(crate) fn register_root(",
        "pub(crate) fn current_tid(",
        "pub(crate) fn current_tgid(",
        "pub(crate) fn lookup_tid(",
        "pub(crate) fn reset(",
    ] {
        assert!(task.contains(api), "missing Linux task API {api}");
    }
    let task_reset_start = task
        .find("pub(crate) fn reset()")
        .expect("Linux task reset");
    let task_reset = braced_body(&task[task_reset_start..]);
    assert!(task_reset.contains("scheduler_thread_for_reset"));
    assert!(task_reset.contains("linux_syscall_context::reset()"));

    assert!(run_elf.contains("const LINUX_RUNTIME_CPU: usize = 0;"));
    let spawn_start = run_elf
        .find("pub fn spawn_observed(")
        .expect("ELF spawn entry");
    let spawn = braced_body(&run_elf[spawn_start..]);
    assert!(spawn.contains("let cpu = LINUX_RUNTIME_CPU;"));
    assert!(spawn.contains("create_thread_on_cpu(run_elf_launcher_entry, \"run_elf\", Some(cpu))"));
    let launcher = run_elf
        .find("extern \"C\" fn run_elf_launcher_entry()")
        .expect("ELF launcher entry");
    let launcher = &run_elf[launcher..];
    let root = launcher
        .find("linux_task::register_root(scheduler_thread)")
        .expect("root task registration");
    let enter = launcher
        .find("user_process::switch_to_el0(entry, stack_top, 0)")
        .expect("EL0 transfer");
    assert!(root < enter);

    let reset = syscall
        .find("pub fn reset_linux_process_state()")
        .expect("Linux process reset");
    let reset = &syscall[reset..];
    let tasks = reset.find("linux_task::reset()").expect("Linux task reset");
    let descriptors = reset.find("sys_close(fd)").expect("descriptor reset");
    let mappings = reset
        .find("memory_state().reset_linux_process_state()")
        .expect("mapping reset");
    let signals = reset
        .find("reset_linux_signal_timer_state()")
        .expect("signal reset");
    assert!(tasks < descriptors && tasks < mappings && tasks < signals);

    assert!(syscall.contains("linux_task::current_tgid()"));
    assert!(syscall.contains("linux_task::current_tid()"));
}

#[test]
fn aarch64_clone_child_is_validated_before_publication() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let run_elf = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread transfer");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");

    let clone_start = syscall.find("pub fn sys_clone(").expect("clone syscall");
    let clone_end = syscall[clone_start..]
        .find("pub fn sys_clone3(")
        .expect("end of clone syscall");
    let clone = &syscall[clone_start..clone_start + clone_end];
    let mut previous = 0usize;
    for operation in [
        "linux_syscall_context::current()",
        "LinuxCloneRequest::validate(",
        "linux_clone_tid_destinations_valid(&request)",
        "create_suspended_thread_on_cpu(",
        "linux_task::reserve_clone(",
        "linux_task::copy_clone_tids(",
        "linux_task::commit_clone(",
    ] {
        let position = clone
            .find(operation)
            .unwrap_or_else(|| panic!("missing {operation}"));
        assert!(
            position >= previous,
            "clone operation out of order: {operation}"
        );
        previous = position;
    }
    assert!(clone.contains("linux_task::restore_clone_tid_destinations(reservation)"));
    assert!(clone.contains("linux_task::rollback_clone(reservation)"));
    assert!(clone.contains("scheduler::scheduler().terminate_thread(scheduler_id)"));

    let destination_validation = syscall
        .find("fn linux_clone_tid_destination_valid(")
        .expect("clone TID destination validation");
    let destination_validation_end = syscall[destination_validation..]
        .find("pub fn sys_clone3(")
        .expect("end of clone TID destination validation");
    let destination_validation =
        &syscall[destination_validation..destination_validation + destination_validation_end];
    assert!(destination_validation.contains("linux_user_range_writable("));
    assert!(destination_validation.contains("linux_mappings"));
    assert!(destination_validation.contains("state.brk"));
    assert!(destination_validation.contains("linux_initial_stack"));
    assert!(!destination_validation.contains("syscall_logic::user_buffer_valid"));

    let clone3_start = syscall.find("pub fn sys_clone3(").expect("clone3 syscall");
    let clone3_end = syscall[clone3_start..]
        .find("/// Linux sys_execve")
        .expect("end of clone3 syscall");
    let clone3 = &syscall[clone3_start..clone3_start + clone3_end];
    assert!(clone3.contains("Err(SysError::ENOSYS)"));
    assert!(!clone3.contains("core::ptr::read"));

    assert!(task.contains("struct Aarch64CloneStart"));
    assert!(task.contains("frame.regs[0] = 0"));
    assert!(task.contains("unsafe { context.frame.read() }"));
    assert!(task.contains("pub(crate) extern \"C\" fn linux_clone_child_entry() -> !"));
    let copy = task
        .find("pub(crate) fn copy_clone_tids(")
        .expect("clone TID copy");
    let copy_end = task[copy..]
        .find("pub(crate) fn restore_clone_tid_destinations(")
        .expect("end of clone TID copy");
    let copy = &task[copy..copy + copy_end];
    let tid_conversion = copy
        .find("linux_tid_to_user_value(reservation.tid)")
        .expect("checked clone TID conversion");
    let copy_validation = copy
        .find("linux_clone_tid_destination_valid(")
        .expect("clone TID destination revalidation");
    assert!(copy.contains("destination.address"));
    let first_raw_read = copy
        .find("core::ptr::read(destination.address as *const u32)")
        .expect("clone TID snapshot read");
    assert!(tid_conversion < first_raw_read);
    assert!(copy_validation < first_raw_read);
    assert!(!copy.contains("reservation.tid as u32"));

    let launcher = run_elf
        .find("extern \"C\" fn run_elf_launcher_entry() -> !")
        .expect("ELF launcher entry");
    let launcher = &run_elf[launcher..];
    let attach_stack = launcher
        .find("run_elf_attach_resource_transition(")
        .expect("initial stack ownership attachment");
    let register_stack = launcher
        .find("register_linux_initial_stack(")
        .expect("initial stack user-range registration");
    let enter_el0 = launcher
        .find("user_process::switch_to_el0(")
        .expect("EL0 entry");
    assert!(attach_stack < register_stack && register_stack < enter_el0);
    let commit = task
        .find("pub(crate) fn commit_clone(")
        .expect("clone commit");
    let commit = &task[commit..];
    let task_publish = commit
        .find("runtime.tasks.publish(reservation)")
        .expect("task publication");
    let scheduler_publish = commit
        .find("publish_suspended_thread(scheduler_id)")
        .expect("scheduler publication");
    assert!(task_publish < scheduler_publish);

    assert!(thread.contains("pub unsafe fn start_linux_clone_child("));
    let child_start = switch
        .find("start_linux_clone_child:")
        .expect("clone child assembly entry");
    let child = &switch[child_start..];
    for instruction in [
        "msr     sp_el0, x17",
        "msr     elr_el1, x17",
        "msr     spsr_el1, x17",
        "msr     tpidr_el0, x17",
        "msr     fpcr, x17",
        "msr     fpsr, x17",
        "ldp     q0, q1, [x16, #0x100]",
        "ldp     q30, q31, [x16, #0x2E0]",
        "ldr     x17, [x16, #0x88]",
        "ldr     x16, [x16, #0x80]",
        "eret",
    ] {
        assert!(
            child.contains(instruction),
            "missing clone transfer {instruction}"
        );
    }
}

#[test]
fn linux_futex_waits_block_and_wake_scheduler_tasks() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let module = std::fs::read_to_string(repository.join("src/syscall/mod.rs"))
        .expect("read syscall module declarations");
    let address = std::fs::read_to_string(repository.join("src/syscall/address_logic_shared.rs"))
        .expect("read shared address logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let futex = std::fs::read_to_string(repository.join("src/syscall/linux_futex.rs"))
        .expect("read Linux futex runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let main = std::fs::read_to_string(repository.join("src/main.rs"))
        .expect("read timer interrupt boundary");

    assert!(module.contains("pub(crate) mod linux_futex;"));
    assert!(task.contains("pub(crate) fn block_current("));
    assert!(task.contains("pub(crate) fn wake_blocked("));
    assert!(address.contains("macro_rules! smros_linux_user_range_readable_body"));
    let readable_start = syscall
        .find("pub(crate) fn linux_user_range_readable(")
        .expect("Linux readable-range helper");
    let readable = braced_body(&syscall[readable_start..]);
    assert!(readable.contains("mapping.prot & MmapProt::READ.bits()"));
    assert!(readable.contains("mapping.prot & MmapProt::WRITE.bits()"));
    assert!(readable.contains("state.brk.current - state.brk.start"));
    assert!(readable.contains("linux_initial_stack"));
    assert!(readable.contains("shared_linux_user_range_readable("));
    assert!(futex.contains("FutexQueue<"));
    assert!(futex.contains("include!(\"linux_runtime_lock_shared.rs\");"));
    assert!(futex.contains("LinuxRuntimeLock<FutexQueue<LINUX_FUTEX_LIMIT>>"));
    assert!(!futex.contains("LinuxFutexRuntimeCell"));
    let with_queue_start = futex
        .find("fn with_queue<")
        .expect("locked futex queue helper");
    let with_queue = braced_body(&futex[with_queue_start..]);
    let queue_lock = with_queue
        .find("LINUX_FUTEX_RUNTIME.lock()")
        .expect("futex queue lock");
    let queue_drop = with_queue.find("drop(queue)").expect("futex queue unlock");
    let interrupt_restore = with_queue
        .find("restore_interrupts(interrupt_state)")
        .expect("interrupt restore");
    assert!(queue_lock < queue_drop && queue_drop < interrupt_restore);
    assert!(!with_queue.contains("scheduler::schedule()"));
    assert!(!with_queue.contains("linux_task::wake_blocked("));
    assert!(futex.contains("core::ptr::read(uaddr as *const u32)"));
    assert!(futex.contains("linux_task::block_current(LinuxBlockReason::Futex)"));
    assert!(futex.contains("scheduler::schedule()"));
    assert!(futex.contains("linux_task::wake_blocked("));

    let futex_entry_start = futex
        .find("pub(crate) fn sys_futex(")
        .expect("Linux futex entry");
    let futex_entry = braced_body(&futex[futex_entry_start..]);
    assert!(futex_entry.contains("linux_user_range_readable("));
    let alignment_check = futex_entry
        .find("if uaddr % core::mem::align_of::<u32>() != 0")
        .expect("futex address alignment check");
    let alignment_error = braced_body(&futex_entry[alignment_check..]);
    assert!(alignment_error.contains("Err(SysError::EINVAL)"));
    let readable_check = futex_entry
        .find("if !futex_address_valid(uaddr)")
        .expect("futex readable-range check");
    let readable_error = braced_body(&futex_entry[readable_check..]);
    assert!(readable_error.contains("Err(SysError::EFAULT)"));
    assert!(alignment_check < readable_check);

    let wait_start = futex.find("fn wait(").expect("Linux futex wait helper");
    let wait = braced_body(&futex[wait_start..]);
    let interrupt_mask = wait.find("mask_interrupts()").expect("interrupt mask");
    let address_revalidation = wait
        .find("linux_user_range_readable(uaddr")
        .expect("futex address revalidation");
    let value_read = wait
        .find("core::ptr::read(uaddr as *const u32)")
        .expect("futex value read");
    assert!(interrupt_mask < address_revalidation && address_revalidation < value_read);
    let mismatch = wait
        .find("if !futex_wait_value_matches(observed, expected)")
        .expect("futex compare mismatch branch");
    let mismatch = braced_body(&wait[mismatch..]);
    assert!(mismatch.contains("Err(SysError::EAGAIN)"));
    assert!(!wait.contains("LINUX_FUTEX_RUNTIME.lock()"));
    let schedule = wait
        .find("scheduler::schedule();")
        .expect("wait schedules after blocking");
    let after_schedule = &wait[schedule..];
    assert!(after_schedule.contains("with_queue(|queue|"));
    assert!(after_schedule.contains("queue.remove(tid, scheduler_thread.0)"));
    assert!(after_schedule.contains("linux_task::wake_blocked("));
    assert!(wait.contains("Some(FutexWaitOutcome::Woken) => Ok(0)"));
    assert!(wait.contains("Some(FutexWaitOutcome::TimedOut) => Err(SysError::ETIMEDOUT)"));
    assert!(wait.contains("Some(FutexWaitOutcome::Interrupted) => Err(SysError::EINTR)"));
    assert!(wait.contains("match deadline.clock"));

    let deadline_start = futex
        .find("fn read_deadline(")
        .expect("Linux futex deadline reader");
    let deadline = braced_body(&futex[deadline_start..]);
    let timeout_revalidation = deadline
        .find("linux_user_range_readable(timeout_pointer")
        .expect("timeout range revalidation");
    let timeout_read = deadline
        .find("core::ptr::read_unaligned(")
        .expect("unaligned timeout read");
    assert!(timeout_revalidation < timeout_read);

    for helper in ["fn wake(", "pub(crate) fn on_timer_tick("] {
        let start = futex.find(helper).expect("futex wake helper");
        let body = braced_body(&futex[start..]);
        let queue_operation = body.find("with_queue(|queue|").expect("queue operation");
        let wake_task = body
            .find("linux_task::wake_blocked(")
            .expect("task wake operation");
        let queue_statement_end = body[queue_operation..]
            .find(";\n")
            .expect("completed queue operation")
            + queue_operation;
        assert!(queue_operation < queue_statement_end && queue_statement_end < wake_task);
    }

    let futex_syscall = syscall
        .find("pub fn sys_futex(")
        .expect("Linux futex syscall");
    let futex_syscall = braced_body(&syscall[futex_syscall..]);
    assert!(futex_syscall.contains("linux_futex::sys_futex("));

    let reset = syscall
        .find("pub fn reset_linux_process_state()")
        .expect("Linux process reset");
    let reset = braced_body(&syscall[reset..]);
    let futex_reset = reset.find("linux_futex::reset()").expect("futex reset");
    let task_reset = reset.find("linux_task::reset()").expect("task reset");
    assert!(futex_reset < task_reset);

    let timer_start = main
        .find("extern \"C\" fn timer_interrupt_handler()")
        .expect("timer interrupt handler");
    let timer = braced_body(&main[timer_start..]);
    assert!(timer.contains("if current_cpu_id() == 0"));
    assert!(!timer.contains("let scheduler ="));
    let scheduler_tick = timer
        .find("scheduler().on_timer_tick()")
        .expect("scheduler tick accounting");
    let futex_tick = timer
        .find("linux_futex::on_timer_tick(")
        .expect("Linux futex deadline expiry");
    let interrupt_end = timer
        .find("end_of_interrupt(interrupt_id)")
        .expect("timer interrupt completion");
    assert!(scheduler_tick < futex_tick && futex_tick < interrupt_end);
}

#[test]
fn linux_signal_state_is_owned_by_each_live_task() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let futex = std::fs::read_to_string(repository.join("src/syscall/linux_futex.rs"))
        .expect("read Linux futex runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal implementation");

    for field in [
        "pub mask: u64",
        "pub standard_pending: u64",
        "pub realtime_pending: [LinuxPendingSignal; LINUX_RT_QUEUE_LIMIT]",
        "pub realtime_len: usize",
        "pub alt_stack: LinuxSignalStack",
        "pub frames: [LinuxSignalFrame; LINUX_SIGNAL_FRAME_LIMIT]",
        "pub frame_depth: usize",
        "pub sigreturn_requested: bool",
    ] {
        assert!(
            task_logic.contains(field),
            "missing per-task signal field {field}"
        );
    }
    assert!(task_logic.contains("signal_states: [LinuxTaskSignalState; N]"));
    assert!(task_logic.contains("pub(crate) fn linux_signal_info_offset("));
    assert!(task_logic.contains("pub(crate) fn inherit_signal_mask("));
    assert!(task_logic.contains("pub(crate) fn route_signal("));
    assert!(task_logic.contains("pub(crate) fn process_signal_target("));
    assert!(task.contains("pub(crate) fn queue_task_signal("));
    assert!(task.contains("pub(crate) fn process_signal_target("));
    assert!(task.contains("pub(crate) fn with_current_signal_state_and_slot<R>("));
    assert!(task.contains("LinuxSignalRouteError::QueueFull => SysError::EAGAIN"));

    let reserve_clone_start = task
        .find("pub(crate) fn reserve_clone(")
        .expect("clone reservation");
    let reserve_clone = braced_body(&task[reserve_clone_start..]);
    let inherit = reserve_clone
        .find("runtime.tasks.inherit_signal_mask(reservation, current.0)")
        .expect("clone signal-mask inheritance");
    let clone_slot = reserve_clone
        .find("runtime.clone_slots[reservation.slot] = LinuxCloneSlot")
        .expect("clone startup slot");
    assert!(inherit < clone_slot);
    assert!(reserve_clone[inherit..clone_slot].contains("runtime.tasks.rollback(reservation)"));

    for (helper, scheduler_call, runtime_end) in [
        (
            "pub(crate) fn block_current(",
            "scheduler::scheduler().block_thread(",
            "})?;",
        ),
        (
            "pub(crate) fn wake_blocked(",
            "scheduler::scheduler().wake_thread(",
            "});",
        ),
    ] {
        let start = task.find(helper).expect("task scheduler transition");
        let body = braced_body(&task[start..]);
        let mask = body
            .find("mask_interrupts()")
            .expect("outer interrupt mask");
        let runtime = body
            .find("with_runtime(|runtime|")
            .expect("task runtime update");
        let runtime_end = body[runtime..]
            .find(runtime_end)
            .expect("task runtime lock release")
            + runtime;
        let scheduler = body
            .find(scheduler_call)
            .expect("scheduler state transition");
        let restore = body
            .rfind("restore_interrupts(interrupt_state)")
            .expect("outer interrupt restore");
        assert!(mask < runtime && runtime_end < scheduler && scheduler < restore);
    }

    for removed_global in [
        "static LINUX_SIGNAL_MASK:",
        "static LINUX_SIGNAL_FRAME_DEPTH:",
        "static LINUX_SIGRETURN_REQUESTED:",
        "static LINUX_SIGNAL_STACK_POINTER:",
        "static LINUX_SIGNAL_STACK_SIZE:",
        "static LINUX_SIGNAL_STACK_FLAGS:",
        "static mut LINUX_SIGNAL_FRAMES:",
    ] {
        assert!(
            !syscall.contains(removed_global),
            "task-owned signal state remains global: {removed_global}"
        );
    }
    assert!(syscall.contains("static LINUX_PROCESS_PENDING_SIGNALS:"));
    assert!(syscall.contains("static mut LINUX_PROCESS_REALTIME_PENDING:"));
    assert!(syscall.contains(
        "linux_task::LINUX_TASK_LIMIT * LINUX_SIGNAL_FRAME_LIMIT * LINUX_SIGNAL_INFO_BYTES"
    ));
    let trampoline_start = syscall
        .find("fn ensure_linux_signal_trampoline()")
        .expect("signal trampoline allocation");
    let trampoline = braced_body(&syscall[trampoline_start..]);
    assert!(trampoline.contains("LINUX_SIGNAL_TRAMPOLINE_BYTES"));
    let process_queue_start = syscall
        .find("fn with_linux_process_realtime_pending<R>(")
        .expect("process realtime queue guard");
    let process_queue = braced_body(&syscall[process_queue_start..]);
    assert!(process_queue.contains("crate::kernel_lowlevel::smp::is_boot_cpu()"));
    let process_mask = process_queue
        .find("mask_interrupts()")
        .expect("process queue interrupt mask");
    let process_access = process_queue
        .find("operation(&mut LINUX_PROCESS_REALTIME_PENDING, &mut count)")
        .expect("serialized process queue access");
    let process_restore = process_queue
        .find("restore_interrupts(interrupt_state)")
        .expect("process queue interrupt restore");
    assert!(process_mask < process_access && process_access < process_restore);

    let reset_start = syscall
        .find("pub fn reset_linux_signal_timer_state()")
        .expect("signal process reset");
    let reset = braced_body(&syscall[reset_start..]);
    assert!(reset.contains("LINUX_PROCESS_PENDING_SIGNALS.store(0"));
    assert!(reset.contains("with_linux_process_realtime_pending("));
    assert!(reset.contains("pending.fill(LinuxPendingSignal::EMPTY)"));
    assert!(reset.contains("*count = 0"));

    for syscall_name in [
        "pub fn sys_rt_sigprocmask(",
        "pub fn sys_rt_sigreturn(",
        "pub fn sys_rt_sigpending(",
        "pub fn sys_sigaltstack(",
    ] {
        let start = syscall.find(syscall_name).expect("signal syscall");
        let body = braced_body(&syscall[start..]);
        assert!(
            body.contains("linux_task::"),
            "{syscall_name} must resolve task-owned signal state"
        );
    }

    let tgkill_start = syscall.find("pub fn sys_tgkill(").expect("tgkill syscall");
    let tgkill = braced_body(&syscall[tgkill_start..]);
    assert!(tgkill.contains("queue_directed_linux_signal("));
    assert!(!tgkill.contains("sys_kill("));
    let rt_tgqueue_start = syscall
        .find("pub fn sys_rt_tgsigqueueinfo(")
        .expect("rt_tgsigqueueinfo syscall");
    let rt_tgqueue = braced_body(&syscall[rt_tgqueue_start..]);
    assert!(rt_tgqueue.contains("queue_directed_linux_signal("));
    assert!(rt_tgqueue.contains("|| {"));
    assert!(rt_tgqueue.contains("linux_signal_record_from_user(sig, info)"));
    assert!(!rt_tgqueue.contains("let record = linux_signal_record_from_user"));
    assert!(!rt_tgqueue.contains("sys_rt_sigqueueinfo("));

    let interrupt_start = futex
        .find("pub(crate) fn interrupt_task(")
        .expect("directed futex interruption");
    let interrupt = braced_body(&futex[interrupt_start..]);
    let queue = interrupt
        .find("with_queue(|queue|")
        .expect("queue interrupt");
    let wake = interrupt
        .find("linux_task::wake_blocked(")
        .expect("blocked task wake");
    assert!(queue < wake);

    let directed_start = syscall
        .find("fn queue_directed_linux_signal(")
        .expect("directed signal routing helper");
    let directed = braced_body(&syscall[directed_start..]);
    let route = directed
        .find("linux_task::queue_task_signal(tgid, tid, LinuxPendingSignal::EMPTY)")
        .expect("exact target validation");
    let disposition = directed
        .find("linux_signal_disposition(signum)")
        .expect("centralized directed disposition");
    let ignore = directed
        .find("LinuxSignalDisposition::Ignore => Ok(0)")
        .expect("ignored directed signal result");
    let build_record = directed
        .find("make_record()?")
        .expect("lazy queued record construction");
    let queue_record = directed[build_record..]
        .find("linux_task::queue_task_signal(tgid, tid, record)")
        .expect("directed pending queue")
        + build_record;
    let interrupt = directed
        .find("interrupt_linux_signal_target(")
        .expect("blocked target interruption");
    assert!(route < disposition && disposition < ignore && ignore < build_record);
    assert!(build_record < queue_record && queue_record < interrupt);
    assert!(directed.contains("LinuxSignalDisposition::Terminate"));
    assert!(directed.contains("LinuxSignalDisposition::Handled"));
    assert!(!directed.contains("terminate_process("));

    assert!(task_logic.contains("pub(crate) enum LinuxSignalDisposition"));
    assert!(task_logic.contains("pub(crate) fn linux_signal_disposition("));

    let process_take_start = syscall
        .find("fn take_process_linux_signal(")
        .expect("process signal selection");
    let process_take = braced_body(&syscall[process_take_start..]);
    let standard_selection = process_take
        .find("LINUX_PROCESS_PENDING_SIGNALS.load(")
        .expect("standard process signal selection");
    let realtime_selection = process_take
        .find("lowest_linux_pending_index(")
        .expect("ordered realtime process signal selection");
    assert!(standard_selection < realtime_selection);
    assert!(task_logic.contains("pub(crate) fn lowest_linux_pending_index("));
    assert!(task_logic.matches("lowest_linux_pending_index(").count() >= 2);
    let target_start = syscall
        .find("fn interrupt_linux_signal_target(")
        .expect("signal target wake helper");
    let target = braced_body(&syscall[target_start..]);
    assert!(target.contains("linux_futex::interrupt_task("));

    let delivery_start = syscall
        .find("fn deliver_next_linux_signal(")
        .expect("signal delivery helper");
    let delivery = braced_body(&syscall[delivery_start..]);
    let dequeue = delivery
        .find("take_unblocked_linux_signal()")
        .expect("unblocked signal dequeue");
    let disposition = delivery
        .find("linux_signal_disposition(signum)")
        .expect("delivery-time disposition");
    let current = delivery
        .find("linux_task::current_task()")
        .expect("current delivery task identity");
    let terminate = delivery
        .find("process_manager().terminate_process(current.tgid)")
        .expect("current process default action");
    assert!(dequeue < disposition && disposition < current && current < terminate);
    assert!(delivery.contains("linux_task::with_current_signal_state_and_slot("));
    assert!(delivery.contains("linux_task::linux_signal_info_offset(task_slot"));
    assert!(delivery.contains("checked_add(LINUX_SIGNAL_INFO_OFFSET)"));
    assert!(delivery.contains("checked_add(info_offset)"));
    assert!(!delivery.contains("depth * LINUX_SIGNAL_INFO_BYTES"));
    assert!(delivery.contains("signal_state.push_frame(frame)?"));
    assert!(delivery.contains("requeue_linux_signal(deliverable)"));
    assert!(delivery.contains("signal_state.alt_stack.flags = LINUX_SS_ONSTACK as u32"));

    for syscall_name in ["pub fn sys_tkill(", "pub fn sys_tgkill("] {
        let start = syscall.find(syscall_name).expect("directed signal syscall");
        let body = braced_body(&syscall[start..]);
        assert!(
            body.contains("queue_directed_linux_signal("),
            "{syscall_name} must use centralized directed-signal dispatch"
        );
    }

    let kill_start = syscall.find("pub fn sys_kill(").expect("kill syscall");
    let kill = braced_body(&syscall[kill_start..]);
    assert!(kill.contains("linux_signal_disposition(signum)"));
    assert!(kill.contains("LinuxSignalDisposition::Terminate"));
    assert!(kill.contains("queue_process_linux_signal_and_wake("));
    assert!(!kill.contains("terminate_process("));

    let rt_queue_start = syscall
        .find("pub fn sys_rt_sigqueueinfo(")
        .expect("process queued signal syscall");
    let rt_queue = braced_body(&syscall[rt_queue_start..]);
    let disposition = rt_queue
        .find("linux_signal_disposition(sig)")
        .expect("queued signal disposition");
    let record = rt_queue
        .find("linux_signal_record_from_user(sig, info)")
        .expect("queued siginfo copy");
    assert!(disposition < record);
    assert!(!rt_queue.contains("terminate_process("));

    let timer_start = syscall
        .find("pub extern \"C\" fn deliver_linux_timer_signal_from_irq(")
        .expect("timer signal delivery");
    let timer = braced_body(&syscall[timer_start..]);
    assert!(timer.contains("linux_signal_disposition(LINUX_SIGALRM)"));
    assert!(!timer.contains("action.handler == LINUX_SIG_DFL"));
    assert!(timer.contains("queue_process_linux_signal_and_wake("));
}

#[test]
fn aarch64_el0_context_abi_is_complete() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let boot = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/boot.rs"))
        .expect("read AArch64 exception entry");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread layout");

    assert!(thread.contains("assert!(core::mem::offset_of!(ThreadControlBlock, context) == 0x10)"));

    for (handler, next) in [
        ("irq_handler_sp:", "// IRQ Handler (Current EL with SP0)"),
        ("irq_handler:", "// IRQ Handler (Lower EL using AArch64)"),
        ("irq_handler_lower:", "// Exception Handler"),
        ("exception_handler:", "\n\"#,"),
    ] {
        let start = boot.find(handler).expect("AArch64 exception handler");
        let relative_end = boot[start..]
            .find(next)
            .expect("end of AArch64 exception handler");
        let body = &boot[start..start + relative_end];
        assert!(body.contains("sub     sp, sp, #0x310"), "{handler}");
        assert!(body.contains("add     sp, sp, #0x310"), "{handler}");
        for pair in 0..16 {
            let first = pair * 2;
            let offset = 0x100 + pair * 0x20;
            let save = format!("stp     q{first}, q{}, [sp, #{offset:#05x}]", first + 1);
            let restore = format!("ldp     q{first}, q{}, [sp, #{offset:#05x}]", first + 1);
            assert!(body.contains(&save), "{handler}: missing {save}");
            assert!(body.contains(&restore), "{handler}: missing {restore}");
        }
        assert!(body.contains("mrs     x16, fpcr"), "{handler}");
        assert!(body.contains("str     x16, [sp, #0x300]"), "{handler}");
        assert!(body.contains("mrs     x16, fpsr"), "{handler}");
        assert!(body.contains("str     x16, [sp, #0x308]"), "{handler}");
        assert!(body.contains("msr     fpcr, x16"), "{handler}");
        assert!(body.contains("msr     fpsr, x16"), "{handler}");

        if let Some(call) = body.find("bl      ") {
            for save in [
                "stp     q30, q31, [sp, #0x2e0]",
                "str     x16, [sp, #0x300]",
                "str     x16, [sp, #0x308]",
            ] {
                assert!(body.find(save).expect("complete pre-call state") < call);
            }
        }
    }

    for instruction in [
        "mrs     x17, sp_el0",
        "str     x17, [x16, #0x110]",
        "mrs     x17, elr_el1",
        "str     x17, [x16, #0x118]",
        "mrs     x17, spsr_el1",
        "str     x17, [x16, #0x120]",
        "mrs     x17, tpidr_el0",
        "str     x17, [x16, #0x128]",
        "mrs     x17, fpcr",
        "str     x17, [x16, #0x130]",
        "mrs     x17, fpsr",
        "str     x17, [x16, #0x138]",
        "ldr     x17, [x16, #0x110]",
        "msr     sp_el0, x17",
        "ldr     x17, [x16, #0x118]",
        "msr     elr_el1, x17",
        "ldr     x17, [x16, #0x120]",
        "msr     spsr_el1, x17",
        "ldr     x17, [x16, #0x128]",
        "msr     tpidr_el0, x17",
        "ldr     x17, [x16, #0x130]",
        "msr     fpcr, x17",
        "ldr     x17, [x16, #0x138]",
        "msr     fpsr, x17",
    ] {
        assert!(switch.contains(instruction), "missing {instruction}");
    }
    let scheduler_switches_end = switch
        .find(".globl start_linux_clone_child")
        .unwrap_or(switch.len());
    let scheduler_switches = &switch[..scheduler_switches_end];
    for instruction in [
        "msr     sp_el0, x17",
        "msr     elr_el1, x17",
        "msr     spsr_el1, x17",
        "msr     tpidr_el0, x17",
        "msr     fpcr, x17",
        "msr     fpsr, x17",
    ] {
        assert_eq!(
            scheduler_switches.matches(instruction).count(),
            2,
            "both context-switch entry paths must contain {instruction}"
        );
    }

    for pair in 0..16 {
        let first = pair * 2;
        let offset = 0x140 + pair * 0x20;
        let save = format!("stp     q{first}, q{}, [x16, #{offset:#05X}]", first + 1);
        let restore = format!("ldp     q{first}, q{}, [x16, #{offset:#05X}]", first + 1);
        assert!(switch.contains(&save), "missing {save}");
        assert_eq!(
            scheduler_switches.matches(&restore).count(),
            2,
            "both context-switch entry paths must contain {restore}"
        );
    }
}

#[test]
fn aarch64_context_switch_preserves_irq_mask_until_the_resumed_owner_restores_it() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");
    let scheduler = std::fs::read_to_string(repository.join("src/kernel_objects/scheduler.rs"))
        .expect("read scheduler");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread context");
    let cpu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/cpu.rs"))
        .expect("read AArch64 CPU helpers");

    let switch_start = switch
        .find("context_switch_start:")
        .expect("first-context entry");
    let resumed_switch = &switch[..switch_start];
    let first_switch = &switch[switch_start..];
    assert!(
        !resumed_switch.contains("bic     x17, x17, #0x80"),
        "a resumed context must retain the scheduler's IRQ mask"
    );
    assert!(
        first_switch.contains("bic     x17, x17, #0x80"),
        "the first context still needs to enable IRQs explicitly"
    );
    let trampoline = switch
        .find("thread_start_trampoline:")
        .expect("ordinary first-run thread trampoline");
    let trampoline_end = switch[trampoline..]
        .find(".size thread_start_trampoline")
        .expect("end of ordinary first-run thread trampoline");
    let trampoline = &switch[trampoline..trampoline + trampoline_end];
    let irq_enable = trampoline
        .find("msr     daifclr, #2")
        .expect("first-run thread enables IRQs");
    let entry_branch = trampoline
        .find("br      x19")
        .expect("first-run thread transfers to its entry");
    assert!(irq_enable < entry_branch);
    assert!(thread.contains("x19: entry as *const () as u64"));
    assert!(thread.contains("pc: thread_start_trampoline as *const () as u64"));

    let clone_start = switch
        .find("start_linux_clone_child:")
        .expect("clone-child EL0 transfer");
    let clone_end = switch[clone_start..]
        .find(".size start_linux_clone_child")
        .expect("end of clone-child EL0 transfer");
    let clone = &switch[clone_start..clone_start + clone_end];
    let clone_mask = clone
        .find("msr     daifset, #2")
        .expect("clone child masks IRQs before return-state installation");
    let clone_sp = clone
        .find("msr     sp_el0, x17")
        .expect("clone child installs user stack");
    assert!(clone_mask < clone_sp);

    let user_start = cpu
        .find("pub unsafe fn switch_to_user(")
        .expect("generic EL0 transfer");
    let user = braced_body(&cpu[user_start..]);
    let user_mask = user
        .find("let _interrupt_state = mask_interrupts();")
        .expect("generic EL0 transfer masks IRQs");
    let user_stack = user
        .find("msr sp_el0")
        .expect("generic EL0 transfer installs user stack");
    assert!(user_mask < user_stack);

    for (entry, next_entry) in [
        ("pub fn schedule()", "fn current_logical_cpu("),
        (
            "pub fn schedule_on_cpu(",
            "pub fn start_first_thread_for_cpu(",
        ),
    ] {
        let start = scheduler.find(entry).expect("scheduler switch entry");
        let end = scheduler[start..]
            .find(next_entry)
            .expect("end of scheduler switch entry");
        let body = &scheduler[start..start + end];
        let switch_call = body
            .find("thread::switch_context(current_tcb_ptr, next_tcb_ptr);")
            .expect("context-switch call");
        let restore = body[switch_call..]
            .find("crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);")
            .expect("captured DAIF restore");
        assert!(restore > 0, "{entry} restores DAIF too early");
    }
}

#[test]
fn x86_system_reset_uses_hardware_reset_ports_before_halting() {
    let smp = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/X86_64/smp.rs"
    ));

    assert!(smp.contains("outb(0xcf9, 0x06)"));
    assert!(smp.contains("outb(0x64, 0xfe)"));
    assert!(smp.contains("System reset returned; halting"));
}

#[test]
fn hermes_safe_gateway_authorizes_before_shell_dispatch() {
    let shell = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/user_shell.rs"
    ));

    let gateway = shell
        .find("fn execute_hermes_command(")
        .expect("Hermes gateway must exist");
    let policy = shell[gateway..]
        .find("hermes_shell_logic_shared::classify")
        .expect("gateway must consult the shared policy");
    let dispatch = shell[gateway..]
        .find("for shell_command in SHELL_COMMANDS")
        .expect("gateway must use the existing command registry");

    assert!(policy < dispatch);
    assert!(shell.contains("\"exec\" =>"));
    assert!(shell.contains("Hermes denied forbidden command: "));
}

#[test]
fn hermes_host_tests_use_fixed_enum_jobs_and_protocol() {
    let client = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/vm_host.rs"
    ));
    let launcher = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/smros-vm-launcher.py"
    ));
    let starter = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/start-smros-vm-launcher.sh"
    ));

    assert!(client.contains("enum HermesHostTestJob"));
    assert!(client.contains("Self::Ut => \"ut\""));
    assert!(client.contains("Self::It => \"it\""));
    assert!(client.contains("Self::St => \"st\""));
    assert!(client.contains("SMROS_TEST_RUN 1\\njob="));
    assert!(launcher.contains("if job not in {\"ut\", \"it\", \"st\"}"));
    assert!(!launcher.contains("shell=True"));
    assert!(starter.contains("REQUIRED_VERSION=6"));
    assert!(starter.contains("fields.get(\"hermes_test_jobs\") != \"1\""));
}

#[test]
fn hermes_test_orchestration_is_documented_and_smoke_wired() {
    let shell = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/user_shell.rs"
    ));
    let readme = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"));
    let docs = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/USER_SHELL.md"
    ));
    let smoke = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/smoke-qemu.sh"
    ));

    assert!(shell.contains("\"test-all\" => run_hermes_test_all"));
    let test_all_start = shell
        .find("fn run_hermes_test_all(")
        .expect("test-all function");
    let test_all_end = shell[test_all_start..]
        .find("fn run_hermes_random_campaign(")
        .map(|offset| test_all_start + offset)
        .expect("random campaign function");
    let test_all = &shell[test_all_start..test_all_end];
    let round_loop = "for round in 0..options.iterations {";
    let round_pos = test_all.find(round_loop).expect("test-all iteration loop");
    let round_body = braced_body(&test_all[round_pos..]);
    assert!(test_all[..round_pos].contains("run_hermes_agent_tests(ctx)"));
    assert!(round_body.contains("execute_hermes_campaign_round"));
    assert_eq!(
        round_body
            .matches("for (job_index, job) in jobs.iter().copied().enumerate()")
            .count(),
        1
    );
    for job in [
        "HermesHostTestJob::Ut",
        "HermesHostTestJob::It",
        "HermesHostTestJob::St",
    ] {
        assert_eq!(test_all.matches(job).count(), 1);
    }
    assert!(test_all.contains("campaign_report_omitted_rounds(options.iterations)"));
    for command in ["hermes exec", "hermes random", "hermes test-all"] {
        assert!(readme.contains(command));
        assert!(docs.contains(command));
    }
    assert!(readme.contains("each host job once per iteration"));
    assert!(docs.contains("each host job once per iteration"));
    assert!(shell.contains("details_omitted="));
    assert!(!shell.contains("iterations=<1..64>"));
    assert!(!docs.contains("iterations=<1..64>"));
    assert!(docs.contains("permanently forbidden"));
    assert!(smoke.contains("hermes random seed=1 iterations=1"));
    assert!(smoke.contains("hermes exec reboot"));
    assert!(smoke.contains("Hermes denied forbidden command: reboot"));
}
