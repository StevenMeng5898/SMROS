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
        assert!(!smros_range_overlaps_body!(
            usize::MAX - 1,
            4usize,
            0usize,
            8usize
        ));
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

        assert!(!smros_is_zircon_syscall_number_body!(
            threshold - 1,
            threshold
        ));
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
    fn linux_socket_rules_match_domain_and_type_matrix() {
        let unix = 1u32;
        let local = 1u32;
        let inet = 2u32;
        let netlink = 16u32;
        let packet = 17u32;
        let stream = 1u32;
        let dgram = 2u32;
        let raw = 3u32;
        let type_mask = 0xfu32;

        assert!(smros_linux_socket_type_supported_body!(
            stream | 0x80000,
            type_mask,
            stream,
            dgram,
            raw
        ));
        assert!(!smros_linux_socket_type_supported_body!(
            5u32, type_mask, stream, dgram, raw
        ));
        assert!(smros_linux_socket_domain_type_supported_body!(
            inet, raw, unix, local, inet, netlink, packet, stream, dgram, raw
        ));
        assert!(!smros_linux_socket_domain_type_supported_body!(
            netlink, stream, unix, local, inet, netlink, packet, stream, dgram, raw
        ));
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
        assert!(!smros_ko_duplicate_rights_allowed_body!(
            read,
            read,
            duplicate,
            same_rights,
            known
        ));
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
        assert_eq!(
            smros_sched_policy_from_match_flags_body!(
                false, false, true, true, true, 1u8, 2u8, 3u8, 4u8
            ),
            Some(2)
        );
        assert_eq!(
            smros_sched_policy_from_match_flags_body!(
                false, false, false, true, true, 1u8, 2u8, 3u8, 4u8
            ),
            Some(3)
        );
        assert_eq!(
            smros_sched_policy_from_match_flags_body!(
                false, false, false, false, true, 1u8, 2u8, 3u8, 4u8
            ),
            Some(4)
        );
        assert_eq!(
            smros_sched_policy_from_match_flags_body!(
                false, false, false, false, false, 1u8, 2u8, 3u8, 4u8
            ),
            None
        );
    }

    #[test]
    fn preemption_policy_follows_active_thread_and_policy_rules() {
        let rr = 1u8;
        let edf = 2u8;
        let credit = 3u8;
        let fair = 4u8;

        assert!(!smros_sched_should_preempt_body!(
            rr, rr, edf, credit, fair, 0u32, 1usize, 0u64, 0u64, 0i32
        ));
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
