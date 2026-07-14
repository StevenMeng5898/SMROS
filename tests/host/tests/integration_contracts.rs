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
    assert!(makefile.contains("test: host-fmt-check script-check launcher-test ut it build-test"));

    assert!(docs.contains("make ut"));
    assert!(docs.contains("make it"));
    assert!(docs.contains("SMROS_ST_REQUIRED_PATTERNS"));

    assert!(smoke.contains("SMROS_ST_REQUIRED_PATTERNS"));
    assert!(smoke.contains("[INFO] Fast boot complete. Starting shell"));
    assert!(smoke.contains("smros:/>"));
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
