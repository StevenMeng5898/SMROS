#![allow(unused_comparisons, unused_macros)]

mod main_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/main_logic_shared.rs"
    ));

    #[test]
    fn align_up_handles_exact_offsets_and_overflow() {
        assert_eq!(smros_main_align_up_body!(0usize, 8usize), Some(0));
        assert_eq!(smros_main_align_up_body!(1usize, 8usize), Some(8));
        assert_eq!(smros_main_align_up_body!(16usize, 8usize), Some(16));
        assert_eq!(smros_main_align_up_body!(16usize, 0usize), None);
        assert_eq!(smros_main_align_up_body!(usize::MAX - 1, 8usize), None);
    }

    #[test]
    fn bump_allocator_respects_alignment_and_heap_limit() {
        assert_eq!(
            smros_main_bump_alloc_next_body!(3usize, 5usize, 4usize, 16usize),
            Some((4, 9))
        );
        assert_eq!(
            smros_main_bump_alloc_next_body!(8usize, 8usize, 8usize, 16usize),
            Some((8, 16))
        );
        assert_eq!(
            smros_main_bump_alloc_next_body!(9usize, 8usize, 8usize, 16usize),
            None
        );
    }
}

mod syscall_address_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/address_logic_shared.rs"
    ));

    #[test]
    fn checked_end_rejects_overflow() {
        assert_eq!(smros_checked_end_body!(10usize, 5usize), Some(15));
        assert_eq!(
            smros_checked_end_body!(usize::MAX - 4, 4usize),
            Some(usize::MAX)
        );
        assert_eq!(smros_checked_end_body!(usize::MAX - 4, 5usize), None);
    }

    #[test]
    fn range_overlap_treats_touching_and_overflow_as_non_overlap() {
        assert!(smros_range_overlaps_body!(10usize, 5usize, 14usize, 2usize));
        assert!(!smros_range_overlaps_body!(
            10usize, 5usize, 15usize, 2usize
        ));
        let overflow_non_overlap =
            smros_range_overlaps_body!(usize::MAX - 1, 4usize, 0usize, 8usize);
        assert!(!overflow_non_overlap);
    }

    #[test]
    fn fixed_mmap_requires_page_alignment_and_window_bounds() {
        assert!(smros_fixed_linux_mmap_request_ok_body!(
            0x2000usize,
            0x1000usize,
            0x1000usize,
            0x1000usize,
            0x5000usize
        ));
        assert!(!smros_fixed_linux_mmap_request_ok_body!(
            0x2001usize,
            0x1000usize,
            0x1000usize,
            0x1000usize,
            0x5000usize
        ));
        assert!(!smros_fixed_linux_mmap_request_ok_body!(
            0x4000usize,
            0x2000usize,
            0x1000usize,
            0x1000usize,
            0x5000usize
        ));
    }
}

mod syscall_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_logic_shared.rs"
    ));

    #[test]
    fn zircon_syscall_numbers_round_trip_from_raw_threshold() {
        fn from_raw(syscall_num: u64, threshold: u64) -> u32 {
            smros_zircon_syscall_from_raw_body!(syscall_num, threshold)
        }

        let threshold = 0x8000_0000u64;

        let below_threshold = smros_is_zircon_syscall_number_body!(threshold - 1, threshold);
        assert!(!below_threshold);
        assert!(smros_is_zircon_syscall_number_body!(threshold, threshold));
        assert!(smros_is_zircon_syscall_number_body!(
            threshold + u32::MAX as u64,
            threshold
        ));
        assert!(!smros_is_zircon_syscall_number_body!(
            threshold + u32::MAX as u64 + 1,
            threshold
        ));

        assert_eq!(from_raw(threshold + 42, threshold), 42);
        assert_eq!(from_raw(threshold - 1, threshold), u32::MAX);
    }

    #[test]
    fn buffer_validation_allows_null_only_for_zero_length_buffers() {
        assert!(smros_syscall_user_buffer_valid_body!(0usize, 0usize));
        assert!(!smros_syscall_user_buffer_valid_body!(0usize, 1usize));
        assert!(smros_syscall_user_buffer_valid_body!(0x1000usize, 1usize));
        assert!(smros_syscall_channel_buffers_valid_body!(
            0x1000usize,
            4usize,
            0usize,
            0usize
        ));
    }

    #[test]
    fn signal_updates_and_allowed_masks_are_exact() {
        assert_eq!(
            smros_syscall_signal_update_body!(0b1111u32, 0b0101u32, 0b1000u32),
            0b1010u32
        );
        assert!(smros_syscall_signal_mask_allowed_body!(
            0b0010u32, 0b0100u32, 0b0110u32
        ));
        assert!(!smros_syscall_signal_mask_allowed_body!(
            0b0010u32, 0b1000u32, 0b0110u32
        ));
    }

    #[test]
    #[rustfmt::skip]
    fn linux_socket_rules_match_domain_and_type_matrix() {
        let unix = 1u32;
        let local = 1u32;
        let inet = 2u32;
        let netlink = 16u32;
        let packet = 17u32;
        let s = 1u32;
        let d = 2u32;
        let r = 3u32;
        let tm = 0xfu32;

        let stream_with_flags = smros_linux_socket_type_supported_body!(s | 0x80000, tm, s, d, r);
        assert!(stream_with_flags);
        assert!(!smros_linux_socket_type_supported_body!(5u32, tm, s, d, r));
        assert!(smros_linux_socket_domain_type_supported_body!(inet, r, unix, local, inet, netlink, packet, s, d, r));
        assert!(!smros_linux_socket_domain_type_supported_body!(netlink, s, unix, local, inet, netlink, packet, s, d, r));
    }

    #[test]
    fn linux_iov_bytes_rejects_zero_elem_size_and_overflow() {
        assert!(smros_linux_iov_bytes_valid_body!(4usize, 16usize, 8usize));
        assert!(!smros_linux_iov_bytes_valid_body!(4usize, 0usize, 8usize));
        assert!(!smros_linux_iov_bytes_valid_body!(9usize, 16usize, 8usize));
        assert!(!smros_linux_iov_bytes_valid_body!(
            usize::MAX,
            2usize,
            usize::MAX
        ));
    }
}

mod kernel_object_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/object_logic_shared.rs"
    ));

    #[test]
    fn page_rounding_saturates_on_overflow() {
        assert_eq!(smros_ko_pages_body!(0usize, 4096usize), 0);
        assert_eq!(smros_ko_pages_body!(1usize, 4096usize), 1);
        assert_eq!(smros_ko_pages_body!(4096usize, 4096usize), 1);
        assert_eq!(smros_ko_roundup_pages_body!(4097usize, 4096usize), 8192);
        assert_eq!(
            smros_ko_roundup_pages_body!(usize::MAX, 4096usize),
            usize::MAX
        );
    }

    #[test]
    fn rights_helpers_allow_only_valid_subsets() {
        let read = 0b0001u32;
        let write = 0b0010u32;
        let duplicate = 0b0100u32;
        let known = read | write | duplicate;
        let same_rights = u32::MAX;

        assert_eq!(smros_ko_intersect_rights_body!(read | write, read), read);
        assert!(smros_ko_rights_subset_body!(read, read | write));
        assert!(!smros_ko_rights_subset_body!(duplicate, read | write));
        assert!(smros_ko_duplicate_rights_allowed_body!(
            read | duplicate,
            read,
            duplicate,
            same_rights,
            known
        ));
        let duplicate_without_right =
            smros_ko_duplicate_rights_allowed_body!(read, read, duplicate, same_rights, known);
        assert!(!duplicate_without_right);
    }

    #[test]
    fn scheduler_candidate_wraps_without_overflowing() {
        assert_eq!(
            smros_ko_scheduler_candidate_index_body!(0usize, 0usize, 0usize),
            0
        );
        assert_eq!(
            smros_ko_scheduler_candidate_index_body!(2usize, 3usize, 4usize),
            1
        );
        assert_eq!(
            smros_ko_scheduler_candidate_index_body!(3usize, 1usize, 4usize),
            0
        );
        assert!(smros_ko_scheduler_can_run_body!(2usize, 1usize, true));
        assert!(!smros_ko_scheduler_can_run_body!(1usize, 1usize, true));
        assert!(!smros_ko_scheduler_can_run_body!(0usize, 1usize, true));
    }

    #[test]
    fn channel_signal_state_reports_readable_and_peer_closed() {
        let readable = 1u32 << 0;
        let peer_closed = 1u32 << 1;

        assert_eq!(
            smros_ko_channel_signal_state_body!(false, false, readable, peer_closed),
            0
        );
        assert_eq!(
            smros_ko_channel_signal_state_body!(true, false, readable, peer_closed),
            readable
        );
        assert_eq!(
            smros_ko_channel_signal_state_body!(false, true, readable, peer_closed),
            peer_closed
        );
        assert_eq!(
            smros_ko_channel_signal_state_body!(true, true, readable, peer_closed),
            readable | peer_closed
        );
    }
}

mod fifo_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/fifo_logic_shared.rs"
    ));

    #[test]
    fn fifo_capacity_validation_checks_limits_and_byte_overflow() {
        assert!(smros_fifo_capacity_valid_body!(
            16usize, 4usize, 32usize, 8usize, 128usize
        ));
        assert!(!smros_fifo_capacity_valid_body!(
            0usize, 4usize, 32usize, 8usize, 128usize
        ));
        assert!(!smros_fifo_capacity_valid_body!(
            16usize, 16usize, 32usize, 8usize, 128usize
        ));
        assert!(!smros_fifo_capacity_valid_body!(
            usize::MAX,
            2usize,
            usize::MAX,
            usize::MAX,
            usize::MAX
        ));
    }

    #[test]
    fn ring_index_and_signal_refresh_handle_edges() {
        assert_eq!(smros_fifo_ring_index_body!(3usize, 2usize, 4usize), 1);
        assert_eq!(smros_fifo_ring_index_body!(3usize, 0usize, 0usize), 0);
        assert_eq!(smros_fifo_remaining_capacity_body!(4usize, 4usize), 0);
        assert_eq!(smros_fifo_remaining_capacity_body!(3usize, 4usize), 1);

        let readable = 0b0001u32;
        let writable = 0b0010u32;
        assert_eq!(
            smros_fifo_refresh_read_signals_body!(readable, 0usize, readable),
            0
        );
        assert_eq!(
            smros_fifo_refresh_read_signals_body!(0u32, 1usize, readable),
            readable
        );
        assert_eq!(
            smros_fifo_refresh_write_signals_body!(writable, 0usize, writable),
            0
        );
        assert_eq!(
            smros_fifo_refresh_write_signals_body!(0u32, 1usize, writable),
            writable
        );
    }
}

mod scheduler_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/scheduler_logic_shared.rs"
    ));

    #[test]
    fn policy_match_prefers_round_robin_then_edf_credit_then_fair() {
        assert_eq!(
            smros_sched_policy_from_match_flags_body!(
                true, false, true, true, true, 1u8, 2u8, 3u8, 4u8
            ),
            Some(1)
        );
        let edf = smros_sched_policy_from_match_flags_body!(
            false, false, true, true, true, 1u8, 2u8, 3u8, 4u8
        );
        let credit = smros_sched_policy_from_match_flags_body!(
            false, false, false, true, true, 1u8, 2u8, 3u8, 4u8
        );
        let fair = smros_sched_policy_from_match_flags_body!(
            false, false, false, false, true, 1u8, 2u8, 3u8, 4u8
        );
        let none = smros_sched_policy_from_match_flags_body!(
            false, false, false, false, false, 1u8, 2u8, 3u8, 4u8
        );
        assert_eq!(edf, Some(2));
        assert_eq!(credit, Some(3));
        assert_eq!(fair, Some(4));
        assert_eq!(none, None);
    }

    #[test]
    #[rustfmt::skip]
    fn preemption_policy_follows_active_thread_and_policy_rules() {
        let rr = 1u8;
        let edf = 2u8;
        let credit = 3u8;
        let fair = 4u8;

        let single_active = smros_sched_should_preempt_body!(rr, rr, edf, credit, fair, 0u32, 1usize, 0u64, 0u64, 0i32);
        assert!(!single_active);
        assert!(smros_sched_should_preempt_body!(
            rr, rr, edf, credit, fair, 0u32, 2usize, 10u64, 1u64, 1i32
        ));
        assert!(smros_sched_should_preempt_body!(
            edf, rr, edf, credit, fair, 5u32, 2usize, 10u64, 10u64, 1i32
        ));
        assert!(smros_sched_should_preempt_body!(
            credit, rr, edf, credit, fair, 5u32, 2usize, 10u64, 1u64, 0i32
        ));
        assert!(smros_sched_should_preempt_body!(
            fair, rr, edf, credit, fair, 0u32, 2usize, 10u64, 1u64, 1i32
        ));
        assert!(!smros_sched_should_preempt_body!(
            fair, rr, edf, credit, fair, 3u32, 2usize, 10u64, 1u64, 1i32
        ));
    }

    #[test]
    fn priority_ordering_and_cut_in_are_explicit() {
        assert!(smros_sched_priority_better_body!(17u8, false, 0u8));
        assert!(smros_sched_priority_better_body!(20u8, true, 17u8));
        assert!(!smros_sched_priority_better_body!(16u8, true, 17u8));

        assert!(smros_sched_priority_should_preempt_body!(16u8, true, 20u8));
        assert!(!smros_sched_priority_should_preempt_body!(20u8, true, 20u8));
        assert!(!smros_sched_priority_should_preempt_body!(20u8, true, 16u8));
        assert!(!smros_sched_priority_should_preempt_body!(
            16u8, false, 20u8
        ));
    }

    #[test]
    fn fair_better_uses_weighted_cpu_ticks() {
        assert!(smros_sched_fair_better_body!(
            20u32, 5u32, true, 12u32, 1u32
        ));
        assert!(!smros_sched_fair_better_body!(
            18u32, 1u32, true, 20u32, 5u32
        ));
        assert!(smros_sched_fair_better_body!(
            0u32, 0u32, false, 20u32, 5u32
        ));
    }
}

mod lowlevel_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/lowlevel_logic_shared.rs"
    ));

    #[test]
    fn lowlevel_alignment_and_segments_reject_overflow() {
        assert_eq!(smros_ll_align_up_body!(5usize, 4usize), Some(8));
        assert_eq!(smros_ll_align_up_body!(8usize, 4usize), Some(8));
        assert_eq!(smros_ll_align_up_body!(8usize, 0usize), None);
        assert_eq!(smros_ll_segment_size_body!(4usize, 4096usize), Some(16_384));
        assert_eq!(
            smros_ll_segment_end_body!(true, usize::MAX - 1, 4usize, 1usize),
            None
        );
        assert!(smros_ll_segment_contains_body!(
            true,
            0x1000usize,
            2usize,
            0x1000usize,
            0x1fffusize
        ));
        assert!(!smros_ll_segment_contains_body!(
            true,
            0x1000usize,
            2usize,
            0x1000usize,
            0x3000usize
        ));
    }

    #[test]
    fn page_table_helpers_preserve_flags_and_address_bits() {
        assert_eq!(
            smros_ll_pte_output_address_body!(0x1234_5678_9abcu64),
            0x1234_5678_9000u64
        );
        assert_eq!(
            smros_ll_pte_set_output_address_body!(0x555u64, 0x1234_5000u64),
            0x1234_5555u64
        );
        assert_eq!(
            smros_ll_pte_set_flag_body!(0b1010u64, 0b0100u64, true),
            0b1110
        );
        assert_eq!(
            smros_ll_pte_set_flag_body!(0b1110u64, 0b0100u64, false),
            0b1010
        );
    }

    #[test]
    fn fdt_and_interrupt_helpers_check_bounds() {
        assert!(smros_ll_fdt_range_valid_body!(4usize, 4usize, 8usize));
        assert!(!smros_ll_fdt_range_valid_body!(5usize, 4usize, 8usize));
        assert_eq!(smros_ll_fdt_align4_body!(5usize), Some(8));
        assert_eq!(smros_ll_dt_gic_irq_body!(0u32, 1u32, 64u32), Some(33));
        assert_eq!(smros_ll_dt_gic_irq_body!(1u32, 1u32, 64u32), Some(17));
        assert_eq!(smros_ll_dt_gic_irq_body!(2u32, 1u32, 64u32), None);
    }
}

mod user_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/user_logic_shared.rs"
    ));

    #[test]
    fn page_math_and_ascii_parsing_handle_invalid_inputs() {
        assert_eq!(
            smros_user_page_offset_body!(0x1000usize, 2usize, 0x1000usize),
            Some(0x3000)
        );
        assert_eq!(
            smros_user_page_down_body!(0x1234usize, 0x1000usize),
            Some(0x1000)
        );
        assert_eq!(
            smros_user_page_up_body!(0x1234usize, 0x1000usize),
            Some(0x2000)
        );
        assert_eq!(smros_user_page_up_body!(0x1234usize, 0usize), None);
        assert!(smros_user_ascii_shell_input_body!(b'~'));
        assert!(!smros_user_ascii_shell_input_body!(b'\n'));
        assert_eq!(smros_user_decimal_digit_value_body!(b'7'), Some(7));
        assert_eq!(smros_user_decimal_digit_value_body!(b'x'), None);
    }

    #[test]
    fn dns_and_svc_validation_cover_common_rejects() {
        assert!(smros_user_dns_host_len_valid_body!(3usize, 253usize));
        assert!(!smros_user_dns_host_len_valid_body!(0usize, 253usize));
        assert!(smros_user_dns_label_byte_valid_body!(b'a'));
        assert!(smros_user_dns_label_byte_valid_body!(b'-'));
        assert!(!smros_user_dns_label_byte_valid_body!(b'_'));

        assert!(smros_user_svc_rights_valid_body!(0b0011u32, 0b0111u32));
        assert!(!smros_user_svc_rights_valid_body!(0u32, 0b0111u32));
        assert!(smros_user_svc_ipc_header_valid_body!(
            0x5356_4321u32,
            1u32,
            0x5356_4321u32,
            1u32
        ));
    }

    #[test]
    fn elf_metadata_checks_reject_bad_headers_and_ranges() {
        assert!(smros_user_elf_magic_valid_body!(0x7f, b'E', b'L', b'F'));
        assert!(!smros_user_elf_magic_valid_body!(0, b'E', b'L', b'F'));
        assert!(smros_user_elf_class_data_valid_body!(2u8, 1u8, 1u8));
        assert!(smros_user_elf_phdr_table_valid_body!(
            64usize, 56usize, 2usize, 256usize, 56usize, 8usize
        ));
        assert!(!smros_user_elf_phdr_table_valid_body!(
            240usize, 56usize, 2usize, 256usize, 56usize, 8usize
        ));
        assert!(smros_user_elf_segment_bounds_valid_body!(
            16usize, 16usize, 32usize, 64usize
        ));
        assert!(!smros_user_elf_segment_bounds_valid_body!(
            16usize, 32usize, 16usize, 64usize
        ));
    }
}

mod hermes_shell_logic {
    #![allow(dead_code)]
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/hermes_shell_logic_shared.rs"
    ));
}

#[test]
fn hermes_shell_policy_permanently_denies_dangerous_commands() {
    use hermes_shell_logic::{classify, HermesShellPolicy};

    for (command, args) in [
        ("rm", &[][..]),
        ("kill", &["1"][..]),
        ("reboot", &[][..]),
        ("exit", &[][..]),
        ("clear", &[][..]),
        ("vi", &["/tmp/a"][..]),
        ("run", &["app.elf"][..]),
        ("write", &["a", "b"][..]),
        ("mkdir", &["a"][..]),
        ("mv", &["a", "b"][..]),
        ("cp", &["a", "b"][..]),
        ("mount", &[][..]),
        ("vm", &["-k", "demo"][..]),
        ("docker", &["rm", "smros0001"][..]),
        ("docker", &["stop", "smros0001"][..]),
    ] {
        assert_eq!(classify(command, args), HermesShellPolicy::Forbidden);
    }
}

#[test]
fn hermes_shell_policy_allows_only_bounded_safe_forms() {
    use hermes_shell_logic::{classify, HermesShellPolicy, HERMES_MAX_ARG_LEN};

    for (command, args) in [
        ("help", &[][..]),
        ("version", &[][..]),
        ("ps", &["-a"][..]),
        ("meminfo", &[][..]),
        ("testsc", &[][..]),
        ("fuzzsc", &["seed=7", "iterations=4"][..]),
        ("vm", &["-s"][..]),
        ("docker", &["images"][..]),
        ("docker", &["ps", "-a"][..]),
        ("docker", &["inspect", "smros0001"][..]),
        ("docker", &["logs", "smros0001"][..]),
    ] {
        assert_eq!(classify(command, args), HermesShellPolicy::Allowed);
    }

    let oversized = "x".repeat(HERMES_MAX_ARG_LEN + 1);
    assert_eq!(
        classify("echo", &[oversized.as_str()]),
        HermesShellPolicy::Invalid
    );
    assert_eq!(classify("unknown", &[]), HermesShellPolicy::Forbidden);
    assert_eq!(
        classify("fuzzsc", &["iterations=999"]),
        HermesShellPolicy::Invalid
    );
}

#[test]
fn hermes_campaign_selection_is_reproducible_and_bounded() {
    use hermes_shell_logic::{
        campaign_case, campaign_case_index, campaign_iterations_valid,
        campaign_report_includes_round, campaign_report_omitted_rounds, parse_campaign_options,
        HermesCampaignOptions, HERMES_CAMPAIGN_CASES,
    };

    let first: Vec<_> = (0..8)
        .map(|round| campaign_case_index(1234, round))
        .collect();
    let second: Vec<_> = (0..8)
        .map(|round| campaign_case_index(1234, round))
        .collect();
    assert_eq!(first, second);
    assert!(first.iter().all(|index| *index < HERMES_CAMPAIGN_CASES));
    assert!(!campaign_iterations_valid(0));
    assert!(campaign_iterations_valid(64));
    assert!(campaign_iterations_valid(65));
    assert!(campaign_iterations_valid(usize::MAX));
    assert_eq!(
        parse_campaign_options(&["seed=9393", "iterations=65"]),
        Some(HermesCampaignOptions {
            seed: Some(9393),
            iterations: 65,
        })
    );
    let maximum = format!("iterations={}", usize::MAX);
    assert_eq!(
        parse_campaign_options(&[maximum.as_str()]),
        Some(HermesCampaignOptions {
            seed: None,
            iterations: usize::MAX,
        })
    );
    assert_eq!(parse_campaign_options(&["iterations=0"]), None);
    assert_eq!(
        parse_campaign_options(&["iterations=18446744073709551616"]),
        None
    );
    assert_eq!(parse_campaign_options(&["seed=1", "seed=2"]), None);
    assert!(campaign_report_includes_round(0));
    assert!(campaign_report_includes_round(63));
    assert!(!campaign_report_includes_round(64));
    assert_eq!(campaign_report_omitted_rounds(64), 0);
    assert_eq!(campaign_report_omitted_rounds(1000), 936);

    for index in 0..HERMES_CAMPAIGN_CASES {
        let case = campaign_case(index, 1234, index).expect("catalog index");
        assert_eq!(
            hermes_shell_logic::classify(case.command, &case.args[..case.arg_count]),
            hermes_shell_logic::HermesShellPolicy::Allowed
        );
    }
}

mod syscall_bridge_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_bridge_shared.rs"
    ));

    #[test]
    fn syscall_number_routing_has_exact_linux_boundary() {
        assert_eq!(smros_syscall_linux_threshold_u64!(), 1000);
        assert!(smros_is_linux_syscall_number_u64_body!(0u64));
        assert!(smros_is_linux_syscall_number_u64_body!(999u64));
        assert!(!smros_is_linux_syscall_number_u64_body!(1000u64));
        assert!(!smros_is_linux_syscall_number_u64_body!(u64::MAX));
    }

    #[test]
    fn saved_register_frame_layout_and_extraction_are_stable() {
        let mut regs = [0u64; smros_saved_reg_count!()];
        for (idx, reg) in regs.iter_mut().enumerate() {
            *reg = (idx as u64) * 10;
        }
        regs[smros_syscall_number_reg_index!()] = 600;

        assert_eq!(smros_saved_reg_frame_bytes!(), 256);
        assert_eq!(smros_saved_reg_words!(), 32);
        assert_eq!(smros_saved_reg_count!(), 32);
        assert_eq!(smros_syscall_number_reg_index!(), 8);
        assert_eq!(smros_syscall_num_from_regs_body!(regs), 600);
        assert_eq!(smros_syscall_arg_from_reg_body!(regs, 5usize), 50usize);
        assert_eq!(smros_syscall_arg_from_u64_body!(u64::MAX), usize::MAX);
    }

    #[test]
    fn linux_errno_encoding_matches_negative_return_values() {
        assert_eq!(smros_linux_errno_to_u64_body!(0u32), 0);
        assert_eq!(smros_linux_errno_to_u64_body!(1u32), u64::MAX);
        assert_eq!(smros_linux_errno_to_u64_body!(38u32), u64::MAX - 37);
    }
}

mod socket_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/socket_logic_shared.rs"
    ));

    #[test]
    fn options_and_ring_math_handle_masking_and_wraparound() {
        assert!(smros_socket_options_valid_body!(0b0011u32, 0b0111u32));
        assert!(!smros_socket_options_valid_body!(0b1000u32, 0b0111u32));
        assert_eq!(
            smros_socket_mask_options_body!(0b1011u32, 0b0110u32),
            0b0010
        );

        assert_eq!(smros_socket_ring_index_body!(3usize, 2usize, 4usize), 1);
        assert_eq!(smros_socket_ring_index_body!(usize::MAX, 3usize, 8usize), 2);
        assert_eq!(smros_socket_ring_index_body!(9usize, 5usize, 0usize), 0);
        assert_eq!(smros_socket_remaining_capacity_body!(3usize, 8usize), 5);
        assert_eq!(smros_socket_remaining_capacity_body!(8usize, 8usize), 0);
        assert_eq!(smros_socket_min_count_body!(7usize, 3usize), 3);
    }

    #[test]
    fn thresholds_control_socket_read_and_write_signals() {
        let r = 0b0001u32;
        let rt = 0b0010u32;
        let w = 0b0100u32;
        let wt = 0b1000u32;

        assert_eq!(
            smros_socket_refresh_read_signals_body!(r | rt, 0usize, 2usize, r, rt),
            0
        );
        assert_eq!(
            smros_socket_refresh_read_signals_body!(0u32, 3usize, 2usize, r, rt),
            r | rt
        );
        assert_eq!(
            smros_socket_refresh_read_signals_body!(0u32, 3usize, 0usize, r, rt),
            r
        );

        assert_eq!(
            smros_socket_refresh_write_signals_body!(w | wt, true, 8usize, 4usize, w, wt),
            wt
        );
        assert_eq!(
            smros_socket_refresh_write_signals_body!(0u32, false, 8usize, 4usize, w, wt),
            w | wt
        );
        assert_eq!(
            smros_socket_refresh_write_signals_body!(w | wt, false, 0usize, 4usize, w, wt),
            0
        );
    }
}

mod futex_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/futex_logic_shared.rs"
    ));

    #[test]
    fn futex_pointer_validation_checks_null_alignment_and_align_zero() {
        assert!(smros_futex_ptr_valid_body!(0x1000usize, 4usize));
        assert!(!smros_futex_ptr_valid_body!(0usize, 4usize));
        assert!(!smros_futex_ptr_valid_body!(0x1002usize, 4usize));
        assert!(!smros_futex_ptr_valid_body!(0x1000usize, 0usize));
    }

    #[test]
    fn futex_counts_and_values_use_exact_comparisons() {
        assert!(smros_futex_value_matches_body!(42u32, 42u32));
        assert!(!smros_futex_value_matches_body!(41u32, 42u32));
        assert_eq!(smros_futex_min_count_body!(3usize, 5usize), 3);
        assert_eq!(
            smros_futex_saturating_add_body!(usize::MAX - 1, 10usize),
            usize::MAX
        );
    }
}

mod port_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/port_logic_shared.rs"
    ));

    #[test]
    fn port_option_and_packet_guards_reject_invalid_inputs() {
        assert!(smros_port_options_valid_body!(0b0010u32, 0b0110u32));
        assert!(!smros_port_options_valid_body!(0b1000u32, 0b0110u32));
        assert!(smros_port_packet_ptr_valid_body!(0x1000usize, 32usize));
        assert!(!smros_port_packet_ptr_valid_body!(0usize, 32usize));
        assert!(!smros_port_packet_ptr_valid_body!(0x1000usize, 0usize));
        assert!(smros_port_queue_has_space_body!(3usize, 4usize));
        assert!(!smros_port_queue_has_space_body!(4usize, 4usize));
    }

    #[test]
    #[rustfmt::skip]
    fn wait_async_options_disallow_unknown_bits_and_timestamp_conflicts() {
        let edge = 0b0001u32;
        let timestamp = 0b0010u32;
        let boot_timestamp = 0b0100u32;
        let allowed = edge | timestamp | boot_timestamp;

        assert!(smros_port_wait_async_options_valid_body!(edge | timestamp, allowed, timestamp, boot_timestamp));
        assert!(!smros_port_wait_async_options_valid_body!(timestamp | boot_timestamp, allowed, timestamp, boot_timestamp));
        let unknown_bits = smros_port_wait_async_options_valid_body!(0b1000u32, allowed, timestamp, boot_timestamp);
        assert!(!unknown_bits);
    }

    #[test]
    fn observers_distinguish_edge_triggered_from_level_triggered() {
        let readable = 0b0001u32;

        assert!(smros_port_observer_should_queue_body!(
            0u32, readable, readable, true
        ));
        assert!(!smros_port_observer_should_queue_body!(
            readable, readable, readable, true
        ));
        assert!(smros_port_observer_should_queue_body!(
            readable, readable, readable, false
        ));
        assert!(!smros_port_observer_should_queue_body!(
            0u32, 0u32, 0u32, true
        ));
        assert!(smros_port_observer_should_queue_body!(
            0u32, 0u32, 0u32, false
        ));
    }
}

mod log_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/log_logic_shared.rs"
    ));

    #[test]
    fn log_level_parsing_prefers_alias_order_and_fallbacks() {
        let debug = 1u8;
        let info = 2u8;
        let warning = 3u8;
        let err = 4u8;
        let fatal = 5u8;

        assert_eq!(
            smros_log_level_from_raw_body!(
                3u8, 1u8, debug, 2u8, warning, 3u8, err, 4u8, fatal, info
            ),
            err
        );
        assert_eq!(
            smros_log_level_from_raw_body!(
                99u8, 1u8, debug, 2u8, warning, 3u8, err, 4u8, fatal, info
            ),
            info
        );
        let warning_alias = smros_log_level_from_match_flags_body!(
            false, false, true, false, true, false, false, debug, info, warning, err, fatal
        );
        let error_alias = smros_log_level_from_match_flags_body!(
            false, false, false, false, false, true, true, debug, info, warning, err, fatal
        );
        let no_match = smros_log_level_from_match_flags_body!(
            false, false, false, false, false, false, false, debug, info, warning, err, fatal
        );
        assert_eq!(warning_alias, Some(warning));
        assert_eq!(error_alias, Some(err));
        assert_eq!(no_match, None);
    }

    #[test]
    fn log_threshold_is_inclusive() {
        assert!(!smros_log_should_log_body!(2u8, 3u8));
        assert!(smros_log_should_log_body!(3u8, 3u8));
        assert!(smros_log_should_log_body!(5u8, 3u8));
    }
}

mod hypervisor_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/hypervisor_logic_shared.rs"
    ));

    #[test]
    fn hypervisor_name_validation_accepts_only_documented_bytes() {
        assert!(smros_hypervisor_name_len_valid_body!(1usize, 32usize));
        assert!(smros_hypervisor_name_len_valid_body!(32usize, 32usize));
        assert!(!smros_hypervisor_name_len_valid_body!(0usize, 32usize));
        assert!(!smros_hypervisor_name_len_valid_body!(33usize, 32usize));

        for byte in [b'a', b'Z', b'0', b'_', b'-', b'.'] {
            assert!(smros_hypervisor_name_byte_valid_body!(byte));
        }
        for byte in [b'/', b' ', b':', b'\n'] {
            assert!(!smros_hypervisor_name_byte_valid_body!(byte));
        }
    }

    #[test]
    fn hypervisor_uptime_and_cpu_usage_saturate_at_edges() {
        let running = 1u8;
        let stopped = 2u8;

        assert_eq!(
            smros_hypervisor_uptime_ticks_body!(running, running, 10u64, 20u64, 30u64),
            20
        );
        assert_eq!(
            smros_hypervisor_uptime_ticks_body!(stopped, running, 10u64, 25u64, 30u64),
            15
        );
        assert_eq!(
            smros_hypervisor_uptime_ticks_body!(stopped, running, 30u64, 25u64, 40u64),
            0
        );
        assert_eq!(
            smros_hypervisor_cpu_usage_percent_body!(0u64, 10_000u32, 99u32),
            0
        );
        assert_eq!(
            smros_hypervisor_cpu_usage_percent_body!(10u64, 5_000u32, 0u32),
            50
        );
        assert_eq!(
            smros_hypervisor_cpu_usage_percent_body!(10u64, 20_000u32, 200u32),
            100
        );
    }

    #[test]
    fn hypervisor_state_transitions_report_restarts_and_stops() {
        let running = 1u8;
        let stopped = 2u8;
        let crashed = 3u8;

        assert_eq!(
            smros_hypervisor_state_count_delta_body!(running, running, stopped, crashed),
            (1, 0, 0)
        );
        assert_eq!(
            smros_hypervisor_state_count_delta_body!(stopped, running, stopped, crashed),
            (0, 1, 0)
        );
        assert_eq!(
            smros_hypervisor_crash_transition_body!(
                true, 0u32, 2u32, 10u64, 42u64, running, crashed
            ),
            (running, 1, 42, true)
        );
        assert_eq!(
            smros_hypervisor_crash_transition_body!(
                true, 2u32, 2u32, 10u64, 42u64, running, crashed
            ),
            (crashed, 2, 10, false)
        );
        assert_eq!(
            smros_hypervisor_kill_transition_body!(55u64, stopped),
            (stopped, 55, true)
        );
        assert_eq!(
            smros_hypervisor_saturating_inc_u32_body!(u32::MAX),
            u32::MAX
        );
    }
}

mod posix_test_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/user_level/services/posix_test_logic_shared.rs"
    ));

    #[test]
    fn manifest_atoms_reject_unsafe_path_syntax() {
        assert!(manifest_atom_valid("conformance/interfaces/mmap/1-1"));
        assert!(manifest_atom_valid("pthread_mutex/2-1"));

        for atom in [
            "",
            "/mmap",
            "mmap/",
            "../mmap",
            "mmap/../1-1",
            "./mmap",
            "mmap//1-1",
            "mmap\\1-1",
            "mmap\t1-1",
            "mmap\n1-1",
            "mmap\u{001f}1-1",
            "mmap\u{007f}1-1",
        ] {
            assert!(!manifest_atom_valid(atom), "unexpectedly accepted {atom:?}");
        }
    }

    #[test]
    fn manifest_atoms_reject_non_ascii_unicode() {
        assert!(!manifest_atom_valid("mmap\u{00e9}/1-1"));
    }

    #[test]
    fn manifest_atoms_reject_bidi_control_unicode() {
        assert!(!manifest_atom_valid("mmap\u{202e}/1-1"));
    }

    #[test]
    fn staged_binaries_stay_below_the_approved_boundary() {
        assert!(staged_binary_path_valid(
            "/shared/posixtest/bin/conformance/interfaces/mmap/1-1"
        ));
        assert!(!staged_binary_path_valid("/shared/posixtest/bin"));
        assert!(!staged_binary_path_valid("/shared/posixtest/bin/../mmap"));
        assert!(!staged_binary_path_valid("/shared/posixtest/bin/mmap//1-1"));
        assert!(!staged_binary_path_valid(
            "/shared/posixtest/bin-other/mmap"
        ));
        assert!(!staged_binary_path_valid("/shared/lib/mmap"));
    }

    #[test]
    fn filters_match_only_their_exact_manifest_field() {
        let test_id = "conformance/interfaces/mmap/1-1";
        let group = "memory";
        let api = "mmap";

        assert!(filter_matches(
            PosixFilterKind::All,
            "",
            test_id,
            group,
            api,
            true,
            true
        ));
        assert!(!filter_matches(
            PosixFilterKind::All,
            "",
            test_id,
            group,
            api,
            false,
            true
        ));
        assert!(!filter_matches(
            PosixFilterKind::All,
            "",
            test_id,
            group,
            api,
            true,
            false
        ));

        assert!(filter_matches(
            PosixFilterKind::Test,
            test_id,
            test_id,
            group,
            api,
            false,
            false
        ));
        assert!(!filter_matches(
            PosixFilterKind::Test,
            "conformance/interfaces/mmap/1-2",
            test_id,
            group,
            api,
            true,
            true
        ));
        assert!(filter_matches(
            PosixFilterKind::Group,
            group,
            test_id,
            group,
            api,
            true,
            true
        ));
        assert!(!filter_matches(
            PosixFilterKind::Group,
            "mmap",
            test_id,
            group,
            api,
            true,
            true
        ));
        assert!(filter_matches(
            PosixFilterKind::Api,
            api,
            test_id,
            group,
            api,
            true,
            true
        ));
        assert!(!filter_matches(
            PosixFilterKind::Api,
            "memory",
            test_id,
            group,
            api,
            true,
            true
        ));
    }

    #[test]
    fn pts_exits_map_to_the_guest_status_categories() {
        assert_eq!(pts_status(0), POSIX_STATUS_PASS);
        assert_eq!(pts_status(1), POSIX_STATUS_FAIL);
        assert_eq!(pts_status(2), POSIX_STATUS_UNRESOLVED);
        assert_eq!(pts_status(4), POSIX_STATUS_UNSUPPORTED);
        assert_eq!(pts_status(5), POSIX_STATUS_UNTESTED);
        assert_eq!(POSIX_STATUS_INTERRUPTED, 5);
    }

    #[test]
    fn unknown_positive_pts_exits_are_failures() {
        for exit_code in [3, 6, 9, 127, i32::MAX] {
            assert_eq!(pts_status(exit_code), POSIX_STATUS_FAIL);
        }
    }

    #[test]
    fn negative_exit_codes_are_failures_not_interruptions() {
        for exit_code in [-1, i32::MIN] {
            assert_eq!(pts_status(exit_code), POSIX_STATUS_FAIL);
        }
    }

    #[test]
    fn resource_deltas_are_signed_across_the_usize_range() {
        assert_eq!(resource_delta(4, 7), 3);
        assert_eq!(resource_delta(7, 4), -3);
        assert_eq!(resource_delta(0, usize::MAX), usize::MAX as i128);
        assert_eq!(resource_delta(usize::MAX, 0), -(usize::MAX as i128));
    }
}
