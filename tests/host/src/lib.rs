#![allow(unused_comparisons, unused_macros)]

mod alloc {
    #[allow(unused_imports)]
    pub mod collections {
        pub use std::collections::{BTreeMap, BTreeSet};
    }

    pub mod string {
        pub use std::string::String;
    }

    #[allow(unused_imports)]
    pub mod vec {
        pub use std::vec::Vec;
    }
}

#[cfg(test)]
mod fxfs {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FxfsError {
        Unavailable,
    }

    pub fn ensure_host_share() -> Result<(), FxfsError> {
        Err(FxfsError::Unavailable)
    }

    pub fn read_file(_path: &str, _out: &mut [u8]) -> Result<usize, FxfsError> {
        Err(FxfsError::Unavailable)
    }
}

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

    #[test]
    fn writable_user_range_requires_one_complete_writable_mapping() {
        let mappings = [
            (0x1000usize, 0x1000usize, true),
            (0x3000usize, 0x1000usize, false),
        ];

        assert!(smros_linux_user_range_writable_body!(
            0x1800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_writable_body!(
            0x2800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_writable_body!(
            0x3800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
    }

    #[test]
    fn writable_user_range_rejects_mapping_end_crossing_and_overflow() {
        let mappings = [(0x1000usize, 0x1000usize, true)];

        assert!(!smros_linux_user_range_writable_body!(
            0x1ffeusize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_writable_body!(
            usize::MAX - 1,
            core::mem::size_of::<u32>(),
            mappings
        ));
    }

    #[test]
    fn writable_user_range_accepts_active_brk_and_initial_stack_storage() {
        let active_user_ranges = [
            (0x6000_0000usize, 0x8000usize, true),
            (0x7000_0000usize, 0x20_000usize, true),
        ];

        assert!(smros_linux_user_range_writable_body!(
            0x6000_1000usize,
            core::mem::size_of::<u32>(),
            active_user_ranges
        ));
        assert!(smros_linux_user_range_writable_body!(
            0x7001_fffcusize,
            core::mem::size_of::<u32>(),
            active_user_ranges
        ));
    }

    #[test]
    fn readable_user_range_accepts_read_or_write_permission() {
        let mappings = [
            (0x1000usize, 0x1000usize, true, true),
            (0x3000usize, 0x1000usize, true, false),
            (0x5000usize, 0x1000usize, false, true),
            (0x7000usize, 0x1000usize, false, false),
        ];

        assert!(smros_linux_user_range_readable_body!(
            0x1800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(smros_linux_user_range_readable_body!(
            0x3800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(smros_linux_user_range_readable_body!(
            0x5800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_readable_body!(
            0x7800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
    }

    #[test]
    fn readable_user_range_rejects_gaps_end_crossing_and_overflow() {
        let mappings = [(0x1000usize, 0x1000usize, true, false)];

        assert!(!smros_linux_user_range_readable_body!(
            0x2800usize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_readable_body!(
            0x1ffeusize,
            core::mem::size_of::<u32>(),
            mappings
        ));
        assert!(!smros_linux_user_range_readable_body!(
            usize::MAX - 1,
            core::mem::size_of::<u32>(),
            mappings
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
    fn linux_signal_actions_reject_sigkill_and_sigstop() {
        let max_signal = 64usize;

        assert!(smros_linux_signal_action_valid_body!(1usize, max_signal));
        assert!(smros_linux_signal_action_valid_body!(64usize, max_signal));
        assert!(!smros_linux_signal_action_valid_body!(0usize, max_signal));
        assert!(!smros_linux_signal_action_valid_body!(9usize, max_signal));
        assert!(!smros_linux_signal_action_valid_body!(19usize, max_signal));
        assert!(!smros_linux_signal_action_valid_body!(65usize, max_signal));
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

    #[test]
    fn absent_memory_state_matches_initialized_permanent_handle_baseline() {
        let absent = logical_memory_handle_count(None);
        let initialized = logical_memory_handle_count(Some(MEMORY_PERMANENT_HANDLE_COUNT));

        assert_eq!(absent, MEMORY_PERMANENT_HANDLE_COUNT);
        assert_eq!(initialized, MEMORY_PERMANENT_HANDLE_COUNT);
        assert_eq!(initialized - absent, 0);
    }

    #[test]
    fn linux_exit_status_uses_the_low_eight_bits() {
        for (raw, expected) in [
            (0, 0),
            (1, 1),
            (255, 255),
            (256, 0),
            (-1, 255),
            (i32::MIN, 0),
            (i32::MAX, 255),
        ] {
            assert_eq!(linux_exit_status(raw), expected);
        }
    }
}

mod linux_task_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_task_logic_shared.rs"
    ));

    const PTHREAD_BASE_FLAGS: usize =
        CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM;

    #[test]
    fn clone_validation_accepts_the_pthread_flag_and_pointer_matrix() {
        let request = LinuxCloneRequest::validate(
            PTHREAD_BASE_FLAGS
                | CLONE_SETTLS
                | CLONE_PARENT_SETTID
                | CLONE_CHILD_SETTID
                | CLONE_CHILD_CLEARTID,
            0x8000,
            0x1000,
            0x2000,
            0x3000,
        )
        .expect("glibc pthread clone request");
        assert_eq!(request.user_sp, 0x8000);
        assert_eq!(request.parent_tid, Some(0x1000));
        assert_eq!(request.tls, Some(0x2000));
        assert_eq!(request.child_tid, Some(0x3000));
        assert!(request.clear_child_tid);

        assert!(LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS, 0x9000, 0, 0, 0).is_ok());
    }

    #[test]
    fn clone_validation_rejects_flags_stack_tls_and_tid_pointer_errors() {
        assert_eq!(
            LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS | 17, 0x8000, 0, 0, 0),
            Err(LinuxCloneValidationError::Flags)
        );
        assert_eq!(
            LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS | 0x8000_0000, 0x8000, 0, 0, 0),
            Err(LinuxCloneValidationError::Flags)
        );
        for missing in [CLONE_VM, CLONE_SIGHAND] {
            assert_eq!(
                LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS & !missing, 0x8000, 0, 0, 0),
                Err(LinuxCloneValidationError::Flags)
            );
        }
        for stack in [0, 0x8008] {
            assert_eq!(
                LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS, stack, 0, 0, 0),
                Err(LinuxCloneValidationError::Stack)
            );
        }
        assert_eq!(
            LinuxCloneRequest::validate(PTHREAD_BASE_FLAGS | CLONE_SETTLS, 0x8000, 0, 0, 0,),
            Err(LinuxCloneValidationError::Tls)
        );
        for parent_tid in [0, 0x1002] {
            assert_eq!(
                LinuxCloneRequest::validate(
                    PTHREAD_BASE_FLAGS | CLONE_PARENT_SETTID,
                    0x8000,
                    parent_tid,
                    0,
                    0,
                ),
                Err(LinuxCloneValidationError::ParentTid)
            );
        }
        for child_tid in [0, 0x1002] {
            assert_eq!(
                LinuxCloneRequest::validate(
                    PTHREAD_BASE_FLAGS | CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID,
                    0x8000,
                    0,
                    0,
                    child_tid,
                ),
                Err(LinuxCloneValidationError::ChildTid)
            );
        }
    }

    #[test]
    fn root_registration_publishes_one_runnable_thread_group_leader() {
        let mut tasks = LinuxTaskTable::<2>::new();

        assert_eq!(tasks.register_root(7), Ok(LINUX_ROOT_TID));
        assert_eq!(tasks.register_root(8), Err(LinuxTaskError::DuplicateRoot));
        assert_eq!(
            tasks.by_tid(LINUX_ROOT_TID),
            Some(LinuxTaskCore {
                tid: LINUX_ROOT_TID,
                tgid: LINUX_ROOT_TID,
                scheduler_thread: 7,
                state: LinuxTaskState::Runnable,
                block_reason: LinuxBlockReason::None,
            })
        );
        assert_eq!(tasks.by_scheduler(7), tasks.by_tid(LINUX_ROOT_TID));
    }

    #[test]
    fn root_registration_reports_zero_capacity() {
        let mut tasks = LinuxTaskTable::<0>::new();

        assert_eq!(tasks.register_root(7), Err(LinuxTaskError::Capacity));
        assert_eq!(tasks.by_tid(LINUX_ROOT_TID), None);
    }

    #[test]
    fn task_slots_publish_atomically_and_tid_values_do_not_reuse() {
        let mut tasks = LinuxTaskTable::<3>::new();
        assert_eq!(tasks.register_root(7), Ok(LINUX_ROOT_TID));

        let first = tasks.reserve_child(8).expect("first child reservation");
        assert_eq!(first.tid, 2);
        assert_eq!(tasks.by_tid(first.tid), None);
        assert_eq!(tasks.by_scheduler(8), None);
        assert!(tasks.publish(first));
        assert_eq!(tasks.by_scheduler(8).map(|task| task.tid), Some(2));

        assert!(tasks.exit(first.tid, 8));
        assert!(tasks.retire(first.tid, 8));
        let second = tasks.reserve_child(9).expect("reused table slot");
        assert_eq!(second.tid, 3);
        assert_ne!(first.tid, second.tid);
        assert!(!tasks.publish(first), "stale reservation must not publish");
        assert!(tasks.publish(second));
    }

    #[test]
    fn task_state_and_scheduler_identity_move_together() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(8).unwrap();
        assert!(tasks.publish(child));
        assert!(tasks.block(child.tid, 8, LinuxBlockReason::Futex));
        assert_eq!(
            tasks.by_tid(child.tid).unwrap().state,
            LinuxTaskState::Blocked
        );
        assert_eq!(
            tasks.by_scheduler(8).unwrap().block_reason,
            LinuxBlockReason::Futex
        );
        assert!(tasks.wake(child.tid, 8));
        assert_eq!(
            tasks.by_tid(child.tid).unwrap().state,
            LinuxTaskState::Runnable
        );
        assert_eq!(
            tasks.by_scheduler(8).unwrap().block_reason,
            LinuxBlockReason::None
        );
        assert!(!tasks.wake(child.tid, 99));
    }

    #[test]
    fn rollback_releases_only_the_matching_starting_reservation() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let first = tasks.reserve_child(8).unwrap();

        assert_eq!(tasks.scheduler_thread_for_reset(first.slot), Some(8));
        assert_eq!(tasks.scheduler_thread_for_reset(usize::MAX), None);

        assert!(!tasks.rollback(LinuxTaskReservation {
            scheduler_thread: 99,
            ..first
        }));
        assert!(tasks.rollback(first));
        assert!(!tasks.rollback(first));

        let second = tasks.reserve_child(9).expect("rolled-back slot");
        assert_eq!(second.slot, first.slot);
        assert_eq!(second.tid, first.tid + 1);
        assert!(!tasks.publish(first));
        assert!(tasks.publish(second));
    }

    #[test]
    fn invalid_and_stale_transitions_leave_the_live_task_unchanged() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(8).unwrap();

        assert!(!tasks.block(child.tid, 8, LinuxBlockReason::Futex));
        assert!(tasks.publish(child));
        assert!(!tasks.publish(child));
        assert!(!tasks.block(child.tid, 99, LinuxBlockReason::SignalWait));
        assert!(!tasks.block(child.tid, 8, LinuxBlockReason::None));
        assert!(!tasks.wake(child.tid, 8));
        assert!(!tasks.retire(child.tid, 8));
        assert!(tasks.exit(child.tid, 8));
        assert!(!tasks.block(child.tid, 8, LinuxBlockReason::SignalSuspend));
        assert!(!tasks.wake(child.tid, 8));
        assert!(!tasks.exit(child.tid, 8));
        assert!(!tasks.retire(child.tid, 99));
        assert_eq!(
            tasks.by_tid(child.tid).unwrap().state,
            LinuxTaskState::Exited
        );
    }

    #[test]
    fn child_exit_takes_clear_tid_once_and_clears_pending_signal_state() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(8).unwrap();
        assert!(tasks.publish(child));
        assert!(tasks.set_clear_child_tid(child.tid, 8, 0x3000));
        assert!(tasks.set_clear_child_tid(child.tid, 8, 0x4000));

        let signal_state = tasks.signal_state_mut(child.tid, 8).unwrap();
        signal_state.mask = 0xaa;
        signal_state
            .queue(LinuxPendingSignal::standard(15))
            .unwrap();

        assert_eq!(tasks.exit_with_clear_child_tid(child.tid, 8), Some(0x4000));
        assert_eq!(tasks.signal_states[child.slot].mask, 0);
        assert_eq!(tasks.signal_states[child.slot].pending_mask(), 0);
        assert_eq!(tasks.clear_child_tids[child.slot], 0);
        assert_eq!(tasks.exit_with_clear_child_tid(child.tid, 8), None);
        assert_eq!(tasks.exit_with_clear_child_tid(child.tid, 99), None);
    }

    #[test]
    fn capacity_does_not_consume_a_tid_and_reset_starts_a_new_launch() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(8).unwrap();
        assert_eq!(child.tid, 2);
        assert_eq!(tasks.reserve_child(9), None);
        assert!(tasks.rollback(child));
        assert_eq!(tasks.reserve_child(9).unwrap().tid, 3);

        tasks.reset();
        assert_eq!(tasks.by_scheduler(7), None);
        assert_eq!(tasks.register_root(10), Ok(LINUX_ROOT_TID));
        assert_eq!(tasks.reserve_child(11).unwrap().tid, 2);
    }

    #[test]
    fn linux_tid_allocation_boundaries_stop_at_positive_pid_t_ceiling() {
        assert_eq!(LINUX_ROOT_TID, 1);
        assert_eq!(LINUX_MAX_TID, i32::MAX as usize);

        assert_eq!(linux_task_tid_allocation(0), None);
        assert_eq!(linux_task_tid_allocation(LINUX_ROOT_TID), None);
        assert_eq!(
            linux_task_tid_allocation(LINUX_MAX_TID - 1),
            Some((LINUX_MAX_TID - 1, Some(LINUX_MAX_TID)))
        );
        assert_eq!(
            linux_task_tid_allocation(LINUX_MAX_TID),
            Some((LINUX_MAX_TID, None))
        );
        assert_eq!(linux_task_tid_allocation(LINUX_MAX_TID + 1), None);
    }

    #[test]
    fn linux_tid_copyout_conversion_accepts_only_positive_pid_t_values() {
        assert_eq!(linux_tid_to_user_value(0), None);
        assert_eq!(linux_tid_to_user_value(LINUX_ROOT_TID), Some(1));
        assert_eq!(
            linux_tid_to_user_value(LINUX_MAX_TID),
            Some(i32::MAX as u32)
        );
        assert_eq!(linux_tid_to_user_value(LINUX_MAX_TID + 1), None);
    }

    #[test]
    fn allocator_reserves_the_tid_ceiling_once_then_exhausts_permanently() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        tasks.next_tid = LINUX_MAX_TID;

        let last = tasks.reserve_child(8).expect("last valid Linux TID");
        assert_eq!(last.tid, LINUX_MAX_TID);
        assert!(tasks.rollback(last));
        assert_eq!(tasks.reserve_child(9), None);

        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(9), None);

        tasks.reset();
        tasks.register_root(10).unwrap();
        assert_eq!(tasks.reserve_child(11).unwrap().tid, 2);
    }

    #[test]
    fn out_of_range_next_tid_does_not_mutate_a_slot_and_exhausts_permanently() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        tasks.next_tid = LINUX_MAX_TID + 1;
        let slots_before = tasks.tasks;

        assert_eq!(tasks.reserve_child(8), None);
        assert_eq!(tasks.tasks, slots_before);

        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(8), None);
        assert_eq!(tasks.tasks, slots_before);
    }

    #[test]
    fn allocator_exhaustion_is_permanent_until_reset() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        tasks.next_tid = usize::MAX;

        assert_eq!(tasks.reserve_child(8), None);
        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(8), None);

        tasks.reset();
        tasks.register_root(9).unwrap();
        assert_eq!(tasks.reserve_child(10).unwrap().tid, 2);
    }

    #[cfg(test)]
    fn signal_record(signum: usize, marker: u8) -> LinuxPendingSignal {
        LinuxPendingSignal {
            signum,
            has_info: true,
            info: [marker; LINUX_SIGNAL_INFO_BYTES],
        }
    }

    #[test]
    fn signal_disposition_centralizes_ignored_and_default_actions() {
        const SIG_DFL: u64 = 0;
        const SIG_IGN: u64 = 1;

        assert_eq!(
            linux_signal_disposition(SIG_IGN, 15),
            LinuxSignalDisposition::Ignore
        );
        for signum in [17, 23, 28] {
            assert_eq!(
                linux_signal_disposition(SIG_DFL, signum),
                LinuxSignalDisposition::Ignore
            );
        }
        for signum in [9, 15] {
            assert_eq!(
                linux_signal_disposition(SIG_DFL, signum),
                LinuxSignalDisposition::Terminate
            );
        }
        assert_eq!(
            linux_signal_disposition(0x1000, 15),
            LinuxSignalDisposition::Handled
        );
    }

    #[cfg(test)]
    fn three_live_tasks() -> (
        LinuxTaskTable<3>,
        LinuxTaskReservation,
        LinuxTaskReservation,
    ) {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let first = tasks.reserve_child(8).unwrap();
        let second = tasks.reserve_child(9).unwrap();
        assert!(tasks.publish(first));
        assert!(tasks.publish(second));
        (tasks, first, second)
    }

    #[test]
    fn signal_masks_pending_stacks_and_frames_are_isolated_by_live_task_identity() {
        let (mut tasks, first, second) = three_live_tasks();
        let identities = [
            (LINUX_ROOT_TID, 7, 0x11u64, 0x1000u64, 2usize, 34usize),
            (first.tid, 8, 0x22u64, 0x2000u64, 3usize, 35usize),
            (second.tid, 9, 0x44u64, 0x3000u64, 4usize, 36usize),
        ];

        for (tid, scheduler_thread, mask, stack_pointer, standard, realtime) in identities {
            let state = tasks
                .signal_state_mut(tid, scheduler_thread)
                .expect("live task signal state");
            state.mask = mask;
            state.alt_stack = LinuxSignalStack {
                sp: stack_pointer,
                flags: 0,
                _padding: 0,
                size: 0x1800,
            };
            state
                .queue(signal_record(standard, standard as u8))
                .unwrap();
            state
                .queue(signal_record(realtime, realtime as u8))
                .unwrap();
            assert_eq!(
                state.push_frame(LinuxSignalFrame {
                    regs: [tid as u64; 32],
                    return_pc: 0x8000 + tid as u64,
                    previous_mask: mask - 1,
                    user_sp: stack_pointer + 0x800,
                    previous_stack_flags: 0,
                    restart: None,
                }),
                Some(0)
            );
            assert_eq!(
                state.push_frame(LinuxSignalFrame {
                    regs: [(tid + 10) as u64; 32],
                    return_pc: 0x9000 + tid as u64,
                    previous_mask: mask,
                    user_sp: stack_pointer + 0x1000,
                    previous_stack_flags: LINUX_SS_ONSTACK,
                    restart: None,
                }),
                Some(1)
            );
        }

        let first_state = tasks.signal_state_mut(first.tid, 8).unwrap();
        first_state.mask = 0xaa;
        assert!(first_state.request_sigreturn());
        let popped = first_state
            .take_requested_frame()
            .expect("first child nested frame");
        assert_eq!(popped.regs[0], (first.tid + 10) as u64);

        let root = tasks.signal_state(LINUX_ROOT_TID, 7).unwrap();
        assert_eq!(root.mask, 0x11);
        assert_eq!(root.alt_stack.sp, 0x1000);
        assert_eq!(root.standard_pending, linux_signal_bit(2));
        assert_eq!(root.realtime_len, 1);
        assert_eq!(root.realtime_pending[0].info, [34; LINUX_SIGNAL_INFO_BYTES]);
        assert_eq!(root.frame_depth, 2);
        assert!(!root.sigreturn_requested);

        let first_state = tasks.signal_state(first.tid, 8).unwrap();
        assert_eq!(first_state.mask, 0xaa);
        assert_eq!(first_state.alt_stack.sp, 0x2000);
        assert_eq!(first_state.standard_pending, linux_signal_bit(3));
        assert_eq!(first_state.realtime_len, 1);
        assert_eq!(
            first_state.realtime_pending[0].info,
            [35; LINUX_SIGNAL_INFO_BYTES]
        );
        assert_eq!(first_state.frame_depth, 1);
        assert!(!first_state.sigreturn_requested);

        let second_state = tasks.signal_state(second.tid, 9).unwrap();
        assert_eq!(second_state.mask, 0x44);
        assert_eq!(second_state.alt_stack.sp, 0x3000);
        assert_eq!(second_state.standard_pending, linux_signal_bit(4));
        assert_eq!(second_state.realtime_len, 1);
        assert_eq!(
            second_state.realtime_pending[0].info,
            [36; LINUX_SIGNAL_INFO_BYTES]
        );
        assert_eq!(second_state.frame_depth, 2);
        assert!(!second_state.sigreturn_requested);

        assert!(tasks.signal_state(first.tid, 99).is_none());
    }

    #[test]
    fn standard_signals_coalesce_and_realtime_signals_are_ordered_and_bounded() {
        let (mut tasks, first, _) = three_live_tasks();
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();

        state.queue(signal_record(10, 0x10)).unwrap();
        state.queue(signal_record(10, 0x20)).unwrap();
        assert_eq!(state.standard_pending, linux_signal_bit(10));

        for marker in 0..LINUX_RT_QUEUE_LIMIT {
            state
                .queue(signal_record(34 + marker % 2, marker as u8))
                .unwrap();
        }
        assert_eq!(
            state.queue(signal_record(34, 0xff)),
            Err(LinuxSignalRouteError::QueueFull)
        );
        let standard = state.take_unblocked().expect("coalesced standard signal");
        assert_eq!(standard, signal_record(10, 0x10));
        for signum_offset in 0..2 {
            for marker in (signum_offset..LINUX_RT_QUEUE_LIMIT).step_by(2) {
                let record = state.take_unblocked().expect("queued realtime signal");
                assert_eq!(record.signum, 34 + signum_offset);
                assert_eq!(record.info, [marker as u8; LINUX_SIGNAL_INFO_BYTES]);
            }
        }
        assert!(state.take_unblocked().is_none());
    }

    #[test]
    fn pending_selection_prefers_standard_signals_over_realtime_signals() {
        let (mut tasks, first, _) = three_live_tasks();
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.queue(signal_record(34, 0x34)).unwrap();
        state.queue(signal_record(10, 0x10)).unwrap();

        assert_eq!(state.take_unblocked().unwrap().signum, 10);
        assert_eq!(state.take_unblocked().unwrap().signum, 34);
    }

    #[test]
    fn realtime_selection_prefers_lowest_signum_and_fifo_within_equal_signums() {
        let (mut tasks, first, _) = three_live_tasks();
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.queue(signal_record(36, 0x11)).unwrap();
        state.queue(signal_record(34, 0x22)).unwrap();
        state.queue(signal_record(36, 0x33)).unwrap();

        assert_eq!(state.take_unblocked().unwrap(), signal_record(34, 0x22));
        assert_eq!(state.take_unblocked().unwrap(), signal_record(36, 0x11));
        assert_eq!(state.take_unblocked().unwrap(), signal_record(36, 0x33));
    }

    #[test]
    fn signal_info_storage_is_distinct_by_task_slot_and_frame_depth() {
        let (tasks, first_task, second_task) = three_live_tasks();
        assert!(tasks.signal_state(first_task.tid, 8).is_some());
        assert!(tasks.signal_state(second_task.tid, 9).is_some());
        assert_ne!(first_task.slot, second_task.slot);

        let first = linux_signal_info_offset(first_task.slot, 0).expect("first task signal info");
        let nested = linux_signal_info_offset(first_task.slot, 1).expect("nested signal info");
        let second =
            linux_signal_info_offset(second_task.slot, 0).expect("second task signal info");
        assert_eq!(nested, first + LINUX_SIGNAL_INFO_BYTES);
        assert_eq!(
            second,
            second_task.slot * LINUX_SIGNAL_FRAME_LIMIT * LINUX_SIGNAL_INFO_BYTES
        );

        let mut storage = [0u8; 3 * LINUX_SIGNAL_FRAME_LIMIT * LINUX_SIGNAL_INFO_BYTES];
        let first_info = core::array::from_fn::<_, LINUX_SIGNAL_INFO_BYTES, _>(|index| index as u8);
        let nested_info =
            core::array::from_fn::<_, LINUX_SIGNAL_INFO_BYTES, _>(|index| (index as u8) ^ 0x5a);
        let second_info =
            core::array::from_fn::<_, LINUX_SIGNAL_INFO_BYTES, _>(|index| !(index as u8));
        storage[first..first + LINUX_SIGNAL_INFO_BYTES].copy_from_slice(&first_info);
        storage[nested..nested + LINUX_SIGNAL_INFO_BYTES].copy_from_slice(&nested_info);
        storage[second..second + LINUX_SIGNAL_INFO_BYTES].copy_from_slice(&second_info);

        assert_eq!(
            &storage[first..first + LINUX_SIGNAL_INFO_BYTES],
            &first_info
        );
        assert_eq!(
            &storage[nested..nested + LINUX_SIGNAL_INFO_BYTES],
            &nested_info
        );
        assert_eq!(
            &storage[second..second + LINUX_SIGNAL_INFO_BYTES],
            &second_info
        );
        assert_eq!(linux_signal_info_offset(0, LINUX_SIGNAL_FRAME_LIMIT), None);
        assert_eq!(linux_signal_info_offset(usize::MAX, 0), None);
    }

    #[test]
    fn process_and_directed_signal_routing_select_only_the_addressed_live_task() {
        let (mut tasks, first, second) = three_live_tasks();
        let signal = 12usize;
        let bit = linux_signal_bit(signal);
        tasks.signal_state_mut(LINUX_ROOT_TID, 7).unwrap().mask = bit;
        tasks.signal_state_mut(first.tid, 8).unwrap().mask = bit;

        assert_eq!(
            tasks.process_signal_target(signal),
            Some(LinuxTaskCore {
                tid: second.tid,
                tgid: LINUX_ROOT_TID,
                scheduler_thread: 9,
                state: LinuxTaskState::Runnable,
                block_reason: LinuxBlockReason::None,
            })
        );

        let target = tasks
            .route_signal(Some(LINUX_ROOT_TID), first.tid, signal_record(signal, 0x5a))
            .expect("tgkill target");
        assert_eq!(target.tid, first.tid);
        assert_eq!(
            tasks.signal_state(first.tid, 8).unwrap().standard_pending,
            bit
        );
        assert_eq!(
            tasks
                .signal_state(LINUX_ROOT_TID, 7)
                .unwrap()
                .standard_pending,
            0,
            "directed signal must never be queued on the caller"
        );

        let realtime = signal_record(35, 0xa5);
        let target = tasks
            .route_signal(Some(LINUX_ROOT_TID), second.tid, realtime)
            .expect("rt_tgsigqueueinfo target");
        assert_eq!(target.tid, second.tid);
        assert_eq!(
            tasks.signal_state(second.tid, 9).unwrap().realtime_pending[0],
            realtime
        );
        assert_eq!(
            tasks.signal_state(LINUX_ROOT_TID, 7).unwrap().realtime_len,
            0
        );

        assert_eq!(
            tasks.route_signal(Some(2), first.tid, signal_record(signal, 1)),
            Err(LinuxSignalRouteError::NoSuchTask)
        );
        assert_eq!(
            tasks.route_signal(None, 999, signal_record(signal, 1)),
            Err(LinuxSignalRouteError::NoSuchTask)
        );

        let before = tasks.signal_state(first.tid, 8).unwrap().pending_mask();
        assert_eq!(
            tasks.route_signal(Some(LINUX_ROOT_TID), first.tid, LinuxPendingSignal::EMPTY),
            Ok(target_for(first))
        );
        assert_eq!(
            tasks.signal_state(first.tid, 8).unwrap().pending_mask(),
            before,
            "signal zero checks existence without queueing"
        );
    }

    #[test]
    fn blocked_signals_remain_pending_and_requeue_restores_the_selected_rt_record() {
        let (mut tasks, first, _) = three_live_tasks();
        let blocked = 34usize;
        let selected = signal_record(35, 0x35);
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.mask = linux_signal_bit(blocked) | linux_signal_bit(10);
        state.queue(signal_record(blocked, 0x34)).unwrap();
        state.queue(selected).unwrap();
        state.queue(signal_record(10, 0x10)).unwrap();

        assert_eq!(state.take_unblocked(), Some(selected));
        state.requeue_front(selected).unwrap();
        assert_eq!(state.take_unblocked(), Some(selected));
        assert!(state.take_unblocked().is_none());
        assert_eq!(
            state.pending_mask(),
            linux_signal_bit(blocked) | linux_signal_bit(10)
        );

        state.mask = 0;
        assert_eq!(state.take_unblocked().unwrap().signum, 10);
        assert_eq!(state.take_unblocked().unwrap().signum, blocked);
        assert!(state.take_unblocked().is_none());
    }

    #[test]
    fn blocked_directed_sigterm_remains_on_its_target_until_unblocked() {
        let (mut tasks, first, second) = three_live_tasks();
        let sigterm = 15usize;
        let bit = linux_signal_bit(sigterm);
        tasks.signal_state_mut(first.tid, 8).unwrap().mask = bit;

        let target = tasks
            .route_signal(
                Some(LINUX_ROOT_TID),
                first.tid,
                LinuxPendingSignal::standard(sigterm),
            )
            .expect("directed SIGTERM target");
        assert_eq!(target.tid, first.tid);
        assert_eq!(
            tasks.signal_state(first.tid, 8).unwrap().standard_pending,
            bit
        );
        assert_eq!(
            tasks
                .signal_state(second.tid, second.scheduler_thread)
                .unwrap()
                .standard_pending,
            0
        );
        assert!(tasks
            .signal_state_mut(first.tid, 8)
            .unwrap()
            .take_unblocked()
            .is_none());

        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.mask = 0;
        assert_eq!(
            state.take_unblocked(),
            Some(LinuxPendingSignal::standard(sigterm))
        );
        assert_eq!(state.standard_pending, 0);
    }

    #[test]
    fn signal_wait_dequeues_matching_pending_records_with_complete_siginfo() {
        let (mut tasks, first, _) = three_live_tasks();
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        let complete_info = LinuxPendingSignal {
            signum: 35,
            has_info: true,
            info: core::array::from_fn(|index| index as u8 ^ 0x5a),
        };
        state.queue(signal_record(34, 0x34)).unwrap();
        state.queue(complete_info).unwrap();

        assert_eq!(
            state.take_matching(linux_signal_bit(35)),
            Some(complete_info)
        );
        assert_eq!(state.realtime_len, 1);
        assert_eq!(state.realtime_pending[0], signal_record(34, 0x34));
    }

    #[test]
    fn signal_wait_zero_deadlines_expire_and_report_eagain_outcomes() {
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 1, 10_000_000),
            Some(41)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 10_000_001, 10_000_000),
            Some(42)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 0, 10_000_000),
            Some(40)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, -1, 0, 10_000_000),
            None
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 1_000_000_000, 10_000_000),
            None
        );
        let mut state = LinuxTaskSignalState::new();
        assert!(state.install_signal_wait(LinuxSignalWait::timed(
            linux_signal_bit(34),
            Some(40),
            0x4000,
        )));

        assert!(!state.expire_signal_wait(39));
        assert!(state.expire_signal_wait(40));
        let completed = state
            .take_signal_wait_outcome()
            .expect("expired signal wait");
        assert_eq!(completed.outcome, LinuxSignalWaitOutcome::TimedOut);
        assert_eq!(completed.output_address, 0x4000);
        assert!(state.signal_wait.is_none());
    }

    #[test]
    fn directed_signals_complete_and_wake_only_matching_signal_waiters() {
        let (mut tasks, first, second) = three_live_tasks();
        let expected = signal_record(35, 0xa5);
        assert!(tasks
            .signal_state_mut(first.tid, 8)
            .unwrap()
            .install_signal_wait(LinuxSignalWait::timed(linux_signal_bit(35), None, 0x5000,)));
        assert!(tasks.block(first.tid, 8, LinuxBlockReason::SignalWait));

        let (target, wake_reason) = tasks
            .route_signal_and_complete_wait(Some(LINUX_ROOT_TID), first.tid, expected)
            .expect("directed waiter target");
        assert_eq!(target.tid, first.tid);
        assert_eq!(wake_reason, Some(LinuxBlockReason::SignalWait));
        assert_eq!(
            tasks
                .signal_state_mut(first.tid, 8)
                .unwrap()
                .take_signal_wait_outcome()
                .unwrap()
                .signal,
            expected
        );
        assert_eq!(tasks.signal_state(first.tid, 8).unwrap().pending_mask(), 0);
        assert_eq!(
            tasks
                .signal_state(second.tid, second.scheduler_thread)
                .unwrap()
                .pending_mask(),
            0
        );
    }

    #[test]
    fn nonmatching_unblocked_signals_interrupt_timed_waits_but_blocked_signals_do_not() {
        let (mut tasks, first, _) = three_live_tasks();
        let blocked_signum = 12;
        let interrupting = signal_record(11, 0x5a);
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.mask = linux_signal_bit(blocked_signum);
        assert!(state.install_signal_wait(LinuxSignalWait::timed(
            linux_signal_bit(10),
            None,
            0x5100,
        )));
        assert!(tasks.block(first.tid, 8, LinuxBlockReason::SignalWait));

        let (_, wake_reason) = tasks
            .route_signal_and_complete_wait(Some(LINUX_ROOT_TID), first.tid, interrupting)
            .expect("route nonmatching unblocked signal");
        assert_eq!(wake_reason, Some(LinuxBlockReason::SignalWait));
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        assert_eq!(
            state.take_signal_wait_outcome().map(|wait| wait.outcome),
            Some(LinuxSignalWaitOutcome::Interrupted)
        );
        assert_eq!(state.take_unblocked(), Some(interrupting));

        let (mut tasks, first, _) = three_live_tasks();
        let blocked = signal_record(blocked_signum, 0x6b);
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        state.mask = linux_signal_bit(blocked_signum);
        assert!(state.install_signal_wait(LinuxSignalWait::timed(
            linux_signal_bit(10),
            None,
            0x5200,
        )));
        assert!(tasks.block(first.tid, 8, LinuxBlockReason::SignalWait));

        let (_, wake_reason) = tasks
            .route_signal_and_complete_wait(Some(LINUX_ROOT_TID), first.tid, blocked)
            .expect("route nonmatching blocked signal");
        assert_eq!(wake_reason, None);
        let state = tasks.signal_state_mut(first.tid, 8).unwrap();
        assert_eq!(
            state.signal_wait.map(|wait| wait.outcome),
            Some(LinuxSignalWaitOutcome::Waiting)
        );
        assert_eq!(state.pending_mask(), linux_signal_bit(blocked_signum));
    }

    #[test]
    fn standard_signal_coalescing_preserves_the_first_complete_siginfo_record() {
        let mut state = LinuxTaskSignalState::new();
        let first = signal_record(10, 0x31);
        let duplicate = signal_record(10, 0x92);

        state.queue(first).unwrap();
        state.queue(duplicate).unwrap();

        assert_eq!(state.pending_mask(), linux_signal_bit(10));
        assert_eq!(state.take_matching(linux_signal_bit(10)), Some(first));
        assert_eq!(state.take_matching(linux_signal_bit(10)), None);

        state.queue(first).unwrap();
        state.discard(10);
        assert_eq!(state.take_matching(linux_signal_bit(10)), None);
    }

    #[test]
    fn sigsuspend_keeps_the_temporary_mask_until_frame_setup_then_restores_the_old_mask() {
        let mut state = LinuxTaskSignalState::new();
        let previous_mask = linux_signal_bit(10);
        let temporary_mask = linux_signal_bit(12);
        state.mask = previous_mask;
        state.mask = temporary_mask;
        assert!(
            state.install_signal_wait(LinuxSignalWait::suspend(!temporary_mask, previous_mask,))
        );

        assert!(state.interrupt_signal_suspend(10));
        let completed = state
            .take_signal_wait_outcome()
            .expect("interrupted sigsuspend");
        assert_eq!(completed.outcome, LinuxSignalWaitOutcome::Interrupted);
        assert_eq!(state.mask, temporary_mask);
        assert_eq!(state.take_suspend_restore_mask(), Some(previous_mask));
        assert_eq!(state.take_suspend_restore_mask(), None);
    }

    #[test]
    fn restart_blocks_preserve_original_futex_inputs_and_attach_only_for_sa_restart() {
        let restart = LinuxRestartBlock {
            syscall_number: 98,
            arguments: [0x10, 0x20, 0x30, 0x40, 0x50, 0x60],
            svc_address: 0x7ffc,
            timeout: LinuxRestartTimeout::Unset,
        };
        let mut state = LinuxTaskSignalState::new();

        assert!(state.install_restart_block(restart));
        assert_eq!(state.take_restart_for_signal(false), None);
        assert!(state.restart_block.is_none());

        assert!(state.install_restart_block(restart));
        let deadline = LinuxRestartTimeout::Deadline {
            ticks: 91,
            realtime: false,
        };
        assert!(state.set_restart_timeout(deadline));
        let attached = state.take_restart_for_signal(true);
        assert_eq!(attached.map(|block| block.timeout), Some(deadline));
        assert!(state.restart_block.is_none());

        let frame = LinuxSignalFrame {
            regs: [0xaa; 32],
            return_pc: 0x9000,
            previous_mask: 0x55,
            user_sp: 0x8000,
            previous_stack_flags: 0,
            restart: attached,
        };
        state.push_frame(frame).unwrap();
        assert!(state.request_sigreturn());
        assert_eq!(
            state
                .take_requested_frame()
                .unwrap()
                .restart
                .map(|block| block.timeout),
            Some(deadline)
        );
    }

    #[test]
    fn signal_state_is_cleared_on_rollback_retire_and_reset() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();

        let rolled_back = tasks.reserve_child(8).unwrap();
        tasks.signal_states[rolled_back.slot].mask = 0x55;
        tasks.signal_states[rolled_back.slot]
            .queue(signal_record(34, 0x34))
            .unwrap();
        assert!(tasks.rollback(rolled_back));
        assert_eq!(tasks.signal_states[rolled_back.slot].mask, 0);
        assert_eq!(tasks.signal_states[rolled_back.slot].realtime_len, 0);

        let retired = tasks.reserve_child(9).unwrap();
        assert_eq!(retired.slot, rolled_back.slot);
        assert!(tasks.publish(retired));
        tasks.signal_state_mut(retired.tid, 9).unwrap().mask = 0xaa;
        assert!(tasks.exit(retired.tid, 9));
        assert!(tasks.retire(retired.tid, 9));
        assert_eq!(tasks.signal_states[retired.slot].mask, 0);

        tasks
            .signal_state_mut(LINUX_ROOT_TID, 7)
            .unwrap()
            .queue(signal_record(12, 0x12))
            .unwrap();
        tasks.reset();
        assert!(tasks.signal_states.iter().all(|state| {
            state.mask == 0
                && state.pending_mask() == 0
                && state.frame_depth == 0
                && !state.sigreturn_requested
                && state.signal_wait.is_none()
                && state.restart_block.is_none()
                && state.suspend_restore_mask.is_none()
                && state.alt_stack == LinuxSignalStack::DISABLED
        }));
    }

    #[test]
    fn clone_inherits_only_the_live_creators_signal_mask() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let parent = tasks.signal_state_mut(LINUX_ROOT_TID, 7).unwrap();
        parent.mask = 0x55aa;
        parent.queue(signal_record(12, 0x12)).unwrap();
        parent.queue(signal_record(34, 0x34)).unwrap();
        parent.alt_stack = LinuxSignalStack {
            sp: 0x4000,
            flags: 0,
            _padding: 0,
            size: 0x2000,
        };
        parent
            .push_frame(LinuxSignalFrame {
                regs: [0x77; 32],
                return_pc: 0x8000,
                previous_mask: 0x11,
                user_sp: 0x5000,
                previous_stack_flags: 0,
                restart: None,
            })
            .unwrap();
        assert!(parent.request_sigreturn());

        let stale = tasks.reserve_child(8).unwrap();
        assert!(tasks.rollback(stale));
        let child = tasks.reserve_child(9).unwrap();
        assert_eq!(stale.slot, child.slot);
        assert_ne!(stale.tid, child.tid);
        assert!(!tasks.inherit_signal_mask(stale, 7));
        assert!(!tasks.inherit_signal_mask(child, 99));
        assert!(tasks.inherit_signal_mask(child, 7));

        let inherited = &tasks.signal_states[child.slot];
        assert_eq!(inherited.mask, 0x55aa);
        assert_eq!(inherited.pending_mask(), 0);
        assert_eq!(inherited.realtime_len, 0);
        assert_eq!(inherited.alt_stack, LinuxSignalStack::DISABLED);
        assert_eq!(inherited.frame_depth, 0);
        assert!(!inherited.sigreturn_requested);

        let parent = tasks.signal_state(LINUX_ROOT_TID, 7).unwrap();
        assert_eq!(
            parent.pending_mask(),
            linux_signal_bit(12) | linux_signal_bit(34)
        );
        assert_eq!(parent.alt_stack.sp, 0x4000);
        assert_eq!(parent.frame_depth, 1);
        assert!(parent.sigreturn_requested);
    }

    #[test]
    fn ignored_signal_discard_clears_every_live_task_queue() {
        let (mut tasks, first, second) = three_live_tasks();
        for (tid, scheduler_thread) in [(LINUX_ROOT_TID, 7), (first.tid, 8), (second.tid, 9)] {
            let state = tasks.signal_state_mut(tid, scheduler_thread).unwrap();
            state.queue(signal_record(12, 0x12)).unwrap();
            state.queue(signal_record(34, 0x34)).unwrap();
            state.queue(signal_record(35, 0x35)).unwrap();
        }

        tasks.discard_signal(12);
        tasks.discard_signal(34);
        for (tid, scheduler_thread) in [(LINUX_ROOT_TID, 7), (first.tid, 8), (second.tid, 9)] {
            let state = tasks.signal_state(tid, scheduler_thread).unwrap();
            assert_eq!(state.standard_pending, 0);
            assert_eq!(state.realtime_len, 1);
            assert_eq!(state.realtime_pending[0].signum, 35);
        }
    }

    #[cfg(test)]
    fn target_for(reservation: LinuxTaskReservation) -> LinuxTaskCore {
        LinuxTaskCore {
            tid: reservation.tid,
            tgid: LINUX_ROOT_TID,
            scheduler_thread: reservation.scheduler_thread,
            state: LinuxTaskState::Runnable,
            block_reason: LinuxBlockReason::None,
        }
    }
}

mod linux_syscall_context_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_syscall_context_logic_shared.rs"
    ));

    #[test]
    fn syscall_frames_follow_scheduler_tasks_across_context_switches() {
        let owners = LinuxSyscallFrameOwners::<4>::new();
        let task_a = 1;
        let task_b = 2;

        assert!(!owners.clear(3, 0), "empty is not a valid frame identity");
        assert!(owners.install(task_a, 0x1000, 0xaaaa, 0x1111));
        assert_eq!(
            owners.current(task_a),
            Some(LinuxSyscallFrameSnapshot {
                frame: 0x1000,
                return_pc: 0xaaaa,
                pstate: 0x1111,
            })
        );
        assert!(
            !owners.install(task_a, 0x3000, 0xcccc, 0x3333),
            "nested installation for one scheduler task must fail"
        );

        assert!(owners.install(task_b, 0x2000, 0xbbbb, 0x2222));
        assert_eq!(
            owners.current(task_b),
            Some(LinuxSyscallFrameSnapshot {
                frame: 0x2000,
                return_pc: 0xbbbb,
                pstate: 0x2222,
            })
        );
        assert!(owners.clear(task_b, 0x2000));
        assert_eq!(owners.current(task_b), None);

        assert_eq!(
            owners.current(task_a).map(|frame| frame.frame),
            Some(0x1000)
        );
        assert!(owners.clear(task_a, 0x1000));
        assert_eq!(owners.current(task_a), None);
    }

    #[test]
    fn syscall_frame_reset_clears_abandoned_task_contexts() {
        let owners = LinuxSyscallFrameOwners::<4>::new();
        assert!(owners.install(1, 0x1000, 0xaaaa, 0x1111));
        assert!(owners.install(2, 0x2000, 0xbbbb, 0x2222));

        owners.clear_all();

        assert_eq!(owners.current(1), None);
        assert_eq!(owners.current(2), None);
    }

    #[test]
    fn syscall_frame_owner_is_retired_before_scheduler_slot_reuse() {
        let owners = LinuxSyscallFrameOwners::<4>::new();
        assert!(owners.install(2, 0x1000, 0xaaaa, 0x1111));

        assert!(owners.clear_owner(2));
        assert_eq!(owners.current(2), None);
        assert!(owners.install(2, 0x2000, 0xbbbb, 0x2222));
        assert_eq!(owners.current(2).map(|frame| frame.frame), Some(0x2000));
        assert!(!owners.clear_owner(3), "an unused owner is already retired");
    }
}

mod linux_runtime_lock_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_runtime_lock_shared.rs"
    ));

    #[test]
    fn runtime_lock_prevents_mutable_aliases_across_cpus() {
        use std::sync::Arc;

        let runtime = Arc::new(LinuxRuntimeLock::new(7usize));
        let mut cpu0 = runtime.try_lock().expect("CPU0 owns runtime");
        *cpu0 = 11;

        let cpu1_runtime = Arc::clone(&runtime);
        let cpu1_was_excluded = std::thread::spawn(move || cpu1_runtime.try_lock().is_none())
            .join()
            .expect("CPU1 lock probe");
        assert!(cpu1_was_excluded);

        drop(cpu0);
        let mut cpu1 = runtime.try_lock().expect("CPU1 acquires after release");
        assert_eq!(*cpu1, 11);
        *cpu1 = 13;
        drop(cpu1);
        assert_eq!(*runtime.lock(), 13);
    }
}

mod linux_futex_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_futex_logic_shared.rs"
    ));

    fn waiter(
        tid: usize,
        scheduler_thread: usize,
        address: usize,
        bitset: u32,
        deadline: Option<FutexDeadline>,
    ) -> FutexWaiter {
        FutexWaiter {
            address,
            bitset,
            tid,
            scheduler_thread,
            deadline,
            sequence: 0,
            outcome: FutexWaitOutcome::Waiting,
        }
    }

    #[test]
    fn futex_decoder_accepts_only_the_supported_linux_operation_matrix() {
        assert_eq!(FUTEX_WAIT, 0);
        assert_eq!(FUTEX_WAKE, 1);
        assert_eq!(FUTEX_WAIT_BITSET, 9);
        assert_eq!(FUTEX_WAKE_BITSET, 10);
        assert_eq!(FUTEX_PRIVATE_FLAG, 128);
        assert_eq!(FUTEX_CLOCK_REALTIME, 256);
        assert_eq!(FUTEX_CMD_MASK, 0x7f);

        assert_eq!(
            decode_futex_op(FUTEX_WAIT | FUTEX_PRIVATE_FLAG),
            Some(DecodedFutexOp {
                command: FutexCommand::Wait,
                private: true,
                realtime: false,
            })
        );
        assert_eq!(
            decode_futex_op(FUTEX_WAIT_BITSET | FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME),
            Some(DecodedFutexOp {
                command: FutexCommand::WaitBitset,
                private: true,
                realtime: true,
            })
        );
        assert_eq!(
            decode_futex_op(FUTEX_WAKE),
            Some(DecodedFutexOp {
                command: FutexCommand::Wake,
                private: false,
                realtime: false,
            })
        );
        assert_eq!(
            decode_futex_op(FUTEX_WAKE_BITSET),
            Some(DecodedFutexOp {
                command: FutexCommand::WakeBitset,
                private: false,
                realtime: false,
            })
        );
        assert_eq!(decode_futex_op(FUTEX_WAIT | FUTEX_CLOCK_REALTIME), None);
        assert_eq!(decode_futex_op(FUTEX_WAKE | FUTEX_CLOCK_REALTIME), None);
        assert_eq!(
            decode_futex_op(FUTEX_WAKE_BITSET | FUTEX_CLOCK_REALTIME),
            None
        );
        assert_eq!(decode_futex_op(2), None);
        assert_eq!(decode_futex_op(FUTEX_WAIT | 0x400), None);
    }

    #[test]
    fn futex_wait_inputs_reject_bad_addresses_bitsets_and_timespecs() {
        assert!(futex_address_valid(0x1000));
        assert!(!futex_address_valid(0));
        assert!(!futex_address_valid(0x1002));
        assert!(!futex_address_valid(usize::MAX - 1));
        assert!(futex_bitset_valid(FUTEX_BITSET_MATCH_ANY));
        assert!(!futex_bitset_valid(0));
        assert!(futex_timespec_valid(0, 0));
        assert!(futex_timespec_valid(1, 999_999_999));
        assert!(!futex_timespec_valid(-1, 0));
        assert!(!futex_timespec_valid(0, -1));
        assert!(!futex_timespec_valid(0, 1_000_000_000));
        assert!(futex_wait_value_matches(17, 17));
        assert!(!futex_wait_value_matches(16, 17));
    }

    #[test]
    fn futex_queue_wakes_matching_waiters_in_fifo_order() {
        let mut queue = FutexQueue::<4>::new();
        queue
            .push(waiter(2, 8, 0x1000, 0x1, None))
            .expect("first waiter");
        queue
            .push(waiter(3, 9, 0x1000, 0x2, None))
            .expect("second waiter");
        queue
            .push(waiter(4, 10, 0x1000, 0x2, None))
            .expect("third waiter");

        assert_eq!(queue.wake(0x1000, 1, 0x2), [Some((3, 9)), None, None, None]);
        assert_eq!(
            queue.wake(0x1000, 2, FUTEX_BITSET_MATCH_ANY),
            [Some((2, 8)), Some((4, 10)), None, None]
        );
        assert_eq!(queue.take_outcome(3, 99), None);
        assert_eq!(queue.take_outcome(3, 9), Some(FutexWaitOutcome::Woken));
    }

    #[test]
    fn futex_interrupt_requires_the_complete_waiter_identity() {
        let mut queue = FutexQueue::<2>::new();
        queue
            .push(waiter(2, 8, 0x1000, FUTEX_BITSET_MATCH_ANY, None))
            .expect("waiter");

        assert!(!queue.interrupt(2, 99));
        assert_eq!(queue.take_outcome(2, 99), None);
        assert!(!queue.remove(2, 99));
        assert!(queue.interrupt(2, 8));
        assert_eq!(
            queue.take_outcome(2, 8),
            Some(FutexWaitOutcome::Interrupted)
        );
    }

    #[test]
    fn futex_task_removal_drains_every_registration_and_preserves_join_waiters() {
        let mut queue = FutexQueue::<4>::new();
        queue.waiters[0] = Some(waiter(2, 8, 0x1000, FUTEX_BITSET_MATCH_ANY, None));
        queue.waiters[1] = Some(waiter(2, 8, 0x2000, FUTEX_BITSET_MATCH_ANY, None));
        queue.waiters[2] = Some(waiter(1, 7, 0x3000, FUTEX_BITSET_MATCH_ANY, None));
        queue.waiters[3] = Some(waiter(3, 9, 0x3000, FUTEX_BITSET_MATCH_ANY, None));

        assert_eq!(queue.remove_task(2, 8), 2);
        assert_eq!(queue.remove_task(2, 8), 0);
        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue.wake(0x3000, 1, FUTEX_BITSET_MATCH_ANY),
            [Some((1, 7)), None, None, None]
        );
        assert_eq!(
            queue.len(),
            2,
            "wake outcome remains registered for collection"
        );
    }

    #[test]
    fn futex_deadlines_round_up_expire_on_the_selected_clock_and_reset_drains() {
        assert_eq!(futex_timespec_to_ticks_ceil(0, 1, 10_000_000), Some(1));
        assert_eq!(
            futex_timespec_to_ticks_ceil(0, 10_000_000, 10_000_000),
            Some(1)
        );
        assert_eq!(
            futex_timespec_to_ticks_ceil(0, 10_000_001, 10_000_000),
            Some(2)
        );

        let mut queue = FutexQueue::<4>::new();
        queue
            .push(waiter(
                2,
                8,
                0x1000,
                FUTEX_BITSET_MATCH_ANY,
                Some(FutexDeadline {
                    ticks: 5,
                    clock: FutexClock::Monotonic,
                }),
            ))
            .unwrap();
        queue
            .push(waiter(
                3,
                9,
                0x2000,
                FUTEX_BITSET_MATCH_ANY,
                Some(FutexDeadline {
                    ticks: 12,
                    clock: FutexClock::Realtime,
                }),
            ))
            .unwrap();

        assert_eq!(queue.expire(4, 11), [None, None, None, None]);
        assert_eq!(queue.expire(5, 11), [Some((2, 8)), None, None, None]);
        assert_eq!(queue.expire(5, 12), [Some((3, 9)), None, None, None]);
        assert_eq!(queue.take_outcome(2, 99), None);
        assert_eq!(queue.take_outcome(2, 8), Some(FutexWaitOutcome::TimedOut));
        assert_eq!(queue.reset(), 1);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn futex_wait_timeout_modes_distinguish_relative_and_absolute_deadlines() {
        let now = 100;
        let tick_nanoseconds = 10_000_000;

        let zero_relative =
            futex_deadline_from_timeout(FutexCommand::Wait, false, now, 0, 0, tick_nanoseconds);
        assert_eq!(
            zero_relative,
            Some(FutexDeadline {
                ticks: now,
                clock: FutexClock::Monotonic,
            })
        );
        assert_eq!(
            futex_deadline_from_timeout(
                FutexCommand::Wait,
                false,
                now,
                0,
                20_000_000,
                tick_nanoseconds,
            ),
            Some(FutexDeadline {
                ticks: 103,
                clock: FutexClock::Monotonic,
            })
        );
        assert_eq!(
            futex_deadline_from_timeout(
                FutexCommand::WaitBitset,
                false,
                now,
                0,
                20_000_000,
                tick_nanoseconds,
            ),
            Some(FutexDeadline {
                ticks: 2,
                clock: FutexClock::Monotonic,
            })
        );
        assert_eq!(
            futex_deadline_from_timeout(
                FutexCommand::WaitBitset,
                true,
                now,
                0,
                20_000_000,
                tick_nanoseconds,
            ),
            Some(FutexDeadline {
                ticks: 2,
                clock: FutexClock::Realtime,
            })
        );
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

    #[test]
    fn linux_task_transitions_never_revive_nonblocked_or_exited_threads() {
        let empty = 0u8;
        let ready = 1u8;
        let running = 2u8;
        let blocked = 3u8;
        let terminated = 4u8;

        for state in [empty, ready, running, blocked, terminated] {
            assert_eq!(
                smros_sched_wake_transition_body!(state, blocked, ready),
                (state == blocked).then_some(ready)
            );
            assert_eq!(
                smros_sched_publish_transition_body!(state, true, blocked, ready),
                (state == blocked).then_some(ready)
            );
            assert_eq!(
                smros_sched_publish_transition_body!(state, false, blocked, ready),
                None
            );
        }

        for state in [empty, ready, running, blocked, terminated] {
            let expected = if state == empty || state == terminated {
                None
            } else {
                Some((false, 1usize))
            };
            assert_eq!(
                smros_sched_terminate_transition_body!(
                    2usize, 1usize, state, 2usize, empty, terminated
                ),
                expected
            );
        }
        assert_eq!(
            smros_sched_terminate_transition_body!(
                2usize, 2usize, running, 2usize, empty, terminated
            ),
            Some((true, 1usize))
        );
        assert_eq!(
            smros_sched_terminate_transition_body!(
                0usize, 2usize, running, 2usize, empty, terminated
            ),
            None
        );
        assert_eq!(
            smros_sched_terminate_transition_body!(
                2usize, 2usize, running, 0usize, empty, terminated
            ),
            None
        );

        let mut active_threads = 2usize;
        if let Some((_, next_active_threads)) = smros_sched_terminate_transition_body!(
            0usize,
            2usize,
            running,
            active_threads,
            empty,
            terminated
        ) {
            active_threads = next_active_threads;
        }
        assert_eq!(
            active_threads, 2,
            "rejected termination preserves accounting"
        );
        if let Some((_, next_active_threads)) = smros_sched_terminate_transition_body!(
            2usize,
            2usize,
            running,
            active_threads,
            empty,
            terminated
        ) {
            active_threads = next_active_threads;
        }
        assert_eq!(active_threads, 1, "accepted termination decrements once");
        if let Some((_, next_active_threads)) = smros_sched_terminate_transition_body!(
            2usize,
            2usize,
            terminated,
            active_threads,
            empty,
            terminated
        ) {
            active_threads = next_active_threads;
        }
        assert_eq!(
            active_threads, 1,
            "repeated termination preserves accounting"
        );
    }

    #[derive(Clone, Copy)]
    enum TestSlotState {
        Empty,
        Running,
        Terminated,
    }

    #[derive(Clone, Copy)]
    struct TestSlot {
        state: TestSlotState,
        has_stack_pointer: bool,
        has_stack_size: bool,
    }

    impl TestSlot {
        const fn empty() -> Self {
            Self {
                state: TestSlotState::Empty,
                has_stack_pointer: false,
                has_stack_size: false,
            }
        }

        fn retired_action(
            self,
            slot: usize,
            current_thread: usize,
            retired_thread: Option<usize>,
        ) -> SchedulerSlotReuse {
            scheduler_retired_slot_reuse_action(
                matches!(self.state, TestSlotState::Terminated),
                slot,
                current_thread,
                retired_thread,
                self.has_stack_pointer,
                self.has_stack_size,
            )
        }
    }

    fn confirm_retired_slot(
        slots: &mut [TestSlot],
        current_thread: usize,
        retired_thread: Option<usize>,
    ) -> SchedulerSlotReuse {
        let Some(slot) = retired_thread else {
            return SchedulerSlotReuse::Unavailable;
        };
        let action = slots[slot].retired_action(slot, current_thread, retired_thread);
        if matches!(
            action,
            SchedulerSlotReuse::ResetOnly | SchedulerSlotReuse::DeallocateAndReuse
        ) {
            slots[slot] = TestSlot::empty();
        }
        action
    }

    #[test]
    fn terminated_slot_reuse_distinguishes_stack_ownership() {
        let already_freed = TestSlot {
            state: TestSlotState::Terminated,
            has_stack_pointer: false,
            has_stack_size: false,
        };
        let deferred_stack = TestSlot {
            state: TestSlotState::Terminated,
            has_stack_pointer: true,
            has_stack_size: true,
        };

        assert_eq!(
            already_freed.retired_action(1, 2, Some(1)),
            SchedulerSlotReuse::ResetOnly
        );
        assert_eq!(
            deferred_stack.retired_action(1, 2, Some(1)),
            SchedulerSlotReuse::DeallocateAndReuse
        );
    }

    #[test]
    fn terminated_slot_reuse_requires_post_switch_confirmation() {
        let deferred_stack = TestSlot {
            state: TestSlotState::Terminated,
            has_stack_pointer: true,
            has_stack_size: true,
        };

        assert_eq!(
            deferred_stack.retired_action(3, 3, Some(3)),
            SchedulerSlotReuse::Unavailable,
            "the current thread may still be executing on its terminated stack"
        );
        assert_eq!(
            deferred_stack.retired_action(3, 4, None),
            SchedulerSlotReuse::Unavailable,
            "termination alone does not prove that the stack switch completed"
        );
        assert_eq!(
            deferred_stack.retired_action(3, 4, Some(3)),
            SchedulerSlotReuse::DeallocateAndReuse
        );

        let retirements = DeferredThreadRetirements::<2>::new();
        assert!(retirements.record_before_switch(1, 3));
        assert_eq!(
            retirements.take_reclaimable(1),
            None,
            "publishing retirement is not proof that the stack switch completed"
        );
        assert!(!retirements.confirm_after_switch(1, 3));
        assert_eq!(
            retirements.take_reclaimable(1),
            None,
            "the outgoing thread cannot confirm its own stack switch"
        );
        assert!(!retirements.confirm_after_switch(0, 4));
        assert!(retirements.confirm_after_switch(1, 4));
        assert_eq!(retirements.take_reclaimable(1), Some(3));
        assert_eq!(retirements.take_reclaimable(1), None);
    }

    #[test]
    fn terminated_slot_reuse_rejects_inconsistent_stack_metadata() {
        for (has_stack_pointer, has_stack_size) in [(true, false), (false, true)] {
            let malformed = TestSlot {
                state: TestSlotState::Terminated,
                has_stack_pointer,
                has_stack_size,
            };
            assert_eq!(
                malformed.retired_action(1, 2, Some(1)),
                SchedulerSlotReuse::Unavailable
            );
        }
    }

    #[test]
    fn reentrant_launches_remain_sustainable_without_reclaiming_current_stack() {
        const MAX_THREADS: usize = 32;
        let mut slots = [TestSlot::empty(); MAX_THREADS];
        let mut current_thread = 1usize;
        slots[current_thread] = TestSlot {
            state: TestSlotState::Running,
            has_stack_pointer: true,
            has_stack_size: true,
        };
        let retirements = DeferredThreadRetirements::<1>::new();
        let mut deallocations = 0usize;

        for _ in 0..10_000 {
            let _ = retirements.confirm_after_switch(0, current_thread);
            let retired_thread = retirements.take_reclaimable(0);
            if confirm_retired_slot(&mut slots, current_thread, retired_thread)
                == SchedulerSlotReuse::DeallocateAndReuse
            {
                deallocations += 1;
            }
            let next_thread = (1..MAX_THREADS)
                .find(|slot| matches!(slots[*slot].state, TestSlotState::Empty))
                .expect("a sequential launcher slot remains available");

            assert_ne!(next_thread, current_thread);
            slots[next_thread] = TestSlot {
                state: TestSlotState::Running,
                has_stack_pointer: true,
                has_stack_size: true,
            };

            slots[current_thread].state = TestSlotState::Terminated;
            assert!(retirements.record_before_switch(0, current_thread));
            current_thread = next_thread;
        }

        assert_eq!(deallocations, 9_999);
    }

    #[test]
    fn post_switch_confirmation_supports_cross_cpu_and_alternating_creators() {
        const MAX_THREADS: usize = 32;
        let mut slots = [TestSlot::empty(); MAX_THREADS];
        let retirements = DeferredThreadRetirements::<2>::new();
        let mut running_by_cpu = [1usize, 2usize];
        for slot in running_by_cpu.iter().copied() {
            slots[slot] = TestSlot {
                state: TestSlotState::Running,
                has_stack_pointer: true,
                has_stack_size: true,
            };
        }

        for launch in 0..10_000 {
            let creator_cpu = launch % 2;
            let retired_cpu = 1 - creator_cpu;
            let retiring_slot = running_by_cpu[retired_cpu];
            slots[retiring_slot].state = TestSlotState::Terminated;
            assert!(retirements.record_before_switch(retired_cpu, retiring_slot));

            assert_eq!(
                retirements.take_reclaimable(creator_cpu),
                None,
                "the creator cannot confirm another CPU's pre-switch retirement"
            );

            assert!(retirements.confirm_after_switch(retired_cpu, running_by_cpu[creator_cpu]));
            let retired_thread = retirements.take_reclaimable(retired_cpu);
            assert_eq!(
                confirm_retired_slot(&mut slots, running_by_cpu[creator_cpu], retired_thread),
                SchedulerSlotReuse::DeallocateAndReuse
            );
            let next = (1..MAX_THREADS)
                .find(|slot| matches!(slots[*slot].state, TestSlotState::Empty))
                .expect("confirmed retired slots remain globally reusable");
            slots[next] = TestSlot {
                state: TestSlotState::Running,
                has_stack_pointer: true,
                has_stack_size: true,
            };
            running_by_cpu[retired_cpu] = next;
        }
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

mod aarch64_context_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/ARM64/context_shared.rs"
    ));

    #[test]
    fn aarch64_exception_and_context_layouts_are_locked() {
        use core::mem::{offset_of, size_of};

        assert_eq!(offset_of!(Aarch64ExceptionFrame, regs), 0x000);
        assert_eq!(offset_of!(Aarch64ExceptionFrame, simd), 0x100);
        assert_eq!(offset_of!(Aarch64ExceptionFrame, fpcr), 0x300);
        assert_eq!(offset_of!(Aarch64ExceptionFrame, fpsr), 0x308);
        assert_eq!(size_of::<Aarch64ExceptionFrame>(), 0x310);

        assert_eq!(offset_of!(CpuContext, x19), 0x098);
        assert_eq!(offset_of!(CpuContext, x21), 0x0a8);
        assert_eq!(offset_of!(CpuContext, x23), 0x0b8);
        assert_eq!(offset_of!(CpuContext, x25), 0x0c8);
        assert_eq!(offset_of!(CpuContext, x27), 0x0d8);
        assert_eq!(offset_of!(CpuContext, fp), 0x0e8);
        assert_eq!(offset_of!(CpuContext, lr), 0x0f0);
        assert_eq!(offset_of!(CpuContext, sp), 0x0f8);
        assert_eq!(offset_of!(CpuContext, pc), 0x100);
        assert_eq!(offset_of!(CpuContext, sp_el0), 0x110);
        assert_eq!(offset_of!(CpuContext, elr_el1), 0x118);
        assert_eq!(offset_of!(CpuContext, spsr_el1), 0x120);
        assert_eq!(offset_of!(CpuContext, tpidr_el0), 0x128);
        assert_eq!(offset_of!(CpuContext, fpcr), 0x130);
        assert_eq!(offset_of!(CpuContext, fpsr), 0x138);
        assert_eq!(offset_of!(CpuContext, simd), 0x140);
        assert_eq!(size_of::<CpuContext>(), 0x340);
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

    #[test]
    fn run_elf_environment_entries_are_strict_and_bounded() {
        assert!(run_elf_environment_entry_valid("PATH=/bin", 4 * 1024));
        assert!(run_elf_environment_entry_valid(
            "LD_LIBRARY_PATH=/shared/lib:/lib",
            4 * 1024
        ));
        assert!(run_elf_environment_entry_valid("TOKEN=a=b", 4 * 1024));

        for invalid in [
            "",
            "PATH",
            "=value",
            "BAD-KEY=value",
            "1BAD=value",
            "BAD KEY=value",
            "BAD\0KEY=value",
            "KEY=bad\0value",
        ] {
            assert!(
                !run_elf_environment_entry_valid(invalid, 4 * 1024),
                "accepted malformed environment entry {invalid:?}"
            );
        }

        assert!(run_elf_environment_entry_valid(
            &format!("KEY={}", "x".repeat(4 * 1024 - 4)),
            4 * 1024
        ));
        assert!(!run_elf_environment_entry_valid(
            &format!("KEY={}", "x".repeat(4 * 1024 - 3)),
            4 * 1024
        ));
    }

    #[test]
    fn run_elf_environment_limits_and_exact_keys_are_unambiguous() {
        assert!(run_elf_environment_totals_valid(
            64,
            32 * 1024,
            64,
            32 * 1024
        ));
        assert!(!run_elf_environment_totals_valid(
            65,
            32 * 1024,
            64,
            32 * 1024
        ));
        assert!(!run_elf_environment_totals_valid(
            64,
            32 * 1024 + 1,
            64,
            32 * 1024
        ));

        assert!(run_elf_environment_entry_has_key(
            "LD_LIBRARY_PATH=/caller/lib",
            "LD_LIBRARY_PATH"
        ));
        assert!(!run_elf_environment_entry_has_key(
            "LD_LIBRARY_PATH_EXTRA=/caller/lib",
            "LD_LIBRARY_PATH"
        ));
        assert!(!run_elf_environment_entry_has_key(
            "XLD_LIBRARY_PATH=/caller/lib",
            "LD_LIBRARY_PATH"
        ));
        assert!(run_elf_environment_keys_equal("LANG=C", "LANG=en_US"));
        assert!(!run_elf_environment_keys_equal("LANGUAGE=C", "LANG=C"));

        let entries = ["LANG=C", "PATH=/bin", "LANG=en_US"];
        assert!(entries.iter().enumerate().any(|(index, entry)| {
            entries[..index]
                .iter()
                .any(|previous| run_elf_environment_keys_equal(previous, entry))
        }));

        assert!(run_elf_environment_valid(
            &["LANG=C", "PATH=/bin"],
            "LD_LIBRARY_PATH",
            60,
            64,
            4 * 1024,
            32 * 1024,
        ));
        for invalid in [
            &["BROKEN"][..],
            &["=empty-key"][..],
            &["LANG=C", "LANG=en_US"][..],
        ] {
            assert!(!run_elf_environment_valid(
                invalid,
                "LD_LIBRARY_PATH",
                60,
                64,
                4 * 1024,
                32 * 1024,
            ));
        }
    }

    #[test]
    fn run_elf_effective_environment_includes_or_suppresses_default() {
        let with_default =
            run_elf_environment_effective_totals(63, 100, false, 60).expect("bounded totals");
        assert_eq!(with_default.entry_count, 64);
        assert_eq!(with_default.total_bytes, 160);
        assert!(with_default.append_default);

        let caller_override =
            run_elf_environment_effective_totals(63, 100, true, 60).expect("bounded totals");
        assert_eq!(caller_override.entry_count, 63);
        assert_eq!(caller_override.total_bytes, 100);
        assert!(!caller_override.append_default);

        assert!(run_elf_environment_effective_totals(usize::MAX, 0, false, 1).is_none());
        assert!(run_elf_environment_effective_totals(0, usize::MAX, false, 1).is_none());
    }

    #[test]
    fn run_elf_environment_sources_preserve_caller_order_and_default_position() {
        assert_eq!(
            run_elf_environment_source_at(0, 2, false),
            Some(RunElfEnvironmentSource::Caller(0))
        );
        assert_eq!(
            run_elf_environment_source_at(1, 2, false),
            Some(RunElfEnvironmentSource::Caller(1))
        );
        assert_eq!(
            run_elf_environment_source_at(2, 2, false),
            Some(RunElfEnvironmentSource::Default)
        );
        assert_eq!(run_elf_environment_source_at(3, 2, false), None);
        assert_eq!(run_elf_environment_source_at(2, 2, true), None);
    }

    #[test]
    fn run_elf_completion_helpers_classify_and_saturate() {
        assert!(run_elf_exit_succeeded(0));
        assert!(!run_elf_exit_succeeded(1));
        assert!(!run_elf_exit_succeeded(-1));
        assert_eq!(run_elf_elapsed_ticks(10, 25), 15);
        assert_eq!(run_elf_elapsed_ticks(25, 10), 0);
    }

    #[test]
    fn run_elf_lifecycle_uses_the_request_as_its_single_activity_source() {
        let state = RunElfStateCell::new(RunElfLifecycleState::new());
        let mut reset_count = 0usize;

        let first_id =
            match run_elf_start_transition(&mut state.lock(), 11usize, || reset_count += 1) {
                RunElfStart::Started(id) => id,
                _ => panic!("first request must start"),
            };
        assert!(matches!(
            run_elf_start_transition(&mut state.lock(), 12usize, || reset_count += 1),
            RunElfStart::Busy(12)
        ));
        assert_eq!(reset_count, 1, "busy rejection must not reset active state");
        assert_eq!(
            run_elf_prepare_return_transition(&mut state.lock(), first_id, 37, || reset_count += 1),
            RunElfTransition::Matched
        );
        assert_eq!(
            run_elf_prepare_return_transition(&mut state.lock(), first_id, 38, || reset_count += 1),
            RunElfTransition::Repeated
        );

        let mut callback_count = 0usize;
        let taken = {
            let mut locked = state.lock();
            run_elf_take_completion_transition(&mut locked, first_id, || reset_count += 1)
        };
        match taken.completion {
            RunElfCompletion::Requested(request) => {
                assert_eq!(request, 11);
                assert_eq!(taken.exit_code, 37);
                assert!(state.lock().request().is_none());
                callback_count += 1;
            }
            _ => panic!("accepted request must produce its requested outcome"),
        }
        assert_eq!(callback_count, 1);
        assert_eq!(reset_count, 3);

        assert_eq!(
            run_elf_take_completion_transition(&mut state.lock(), first_id, || reset_count += 1)
                .completion,
            RunElfCompletion::Repeated
        );
        assert_eq!(callback_count, 1);
        assert_eq!(reset_count, 3, "repeated completion must not reset state");

        let second_id =
            match run_elf_start_transition(&mut state.lock(), 13usize, || reset_count += 1) {
                RunElfStart::Started(id) => id,
                _ => panic!("second request must start"),
            };
        assert_ne!(first_id, second_id);
        assert_eq!(
            run_elf_clear_transition(&mut state.lock(), second_id, || reset_count += 1),
            RunElfCompletion::Requested(13)
        );
        assert!(state.lock().request().is_none());
        assert_eq!(
            run_elf_take_completion_transition(&mut state.lock(), second_id, || reset_count += 1)
                .completion,
            RunElfCompletion::Repeated
        );
        assert_eq!(
            run_elf_take_completion_transition(&mut state.lock(), first_id, || reset_count += 1)
                .completion,
            RunElfCompletion::Stale
        );
        let unknown = RunElfLaunchId::from_raw(99).expect("nonzero launch ID");
        assert_eq!(
            run_elf_take_completion_transition(&mut state.lock(), unknown, || reset_count += 1)
                .completion,
            RunElfCompletion::MissingRequest
        );
        assert_eq!(reset_count, 5);
    }

    #[test]
    fn run_elf_launch_identity_exhaustion_is_fail_closed() {
        let maximum = RunElfLaunchId::from_raw(u64::MAX).expect("maximum is nonzero");
        let mut state = RunElfLifecycleState::with_next_launch_id(maximum);
        let mut resets = 0usize;

        assert_eq!(RunElfLaunchId::from_raw(0), None);
        assert_eq!(RunElfLaunchId::from_usize(0), None);
        assert_eq!(maximum.raw(), u64::MAX);
        assert_eq!(maximum.to_usize(), Some(usize::MAX));
        assert_eq!(RunElfLaunchId::from_usize(usize::MAX), Some(maximum));

        let issued = match run_elf_start_transition(&mut state, 1usize, || resets += 1) {
            RunElfStart::Started(id) => id,
            _ => panic!("last launch ID must be issued once"),
        };
        assert_eq!(issued, maximum);
        assert!(matches!(
            run_elf_take_completion_transition(&mut state, issued, || resets += 1).completion,
            RunElfCompletion::Requested(1)
        ));
        assert!(matches!(
            run_elf_start_transition(&mut state, 2usize, || resets += 1),
            RunElfStart::Exhausted(2)
        ));
        assert!(matches!(
            run_elf_start_transition(&mut state, 3usize, || resets += 1),
            RunElfStart::Exhausted(3)
        ));
        assert!(state.request().is_none());
        assert_eq!(resets, 2, "exhaustion must not reset or reuse state");
    }

    #[test]
    fn run_elf_cpu_bindings_reject_bounds_occupancy_and_stale_clear() {
        let bindings = RunElfCpuBindings::<2>::new();
        let first = RunElfLaunchId::from_raw(1).expect("first ID");
        let second = RunElfLaunchId::from_raw(2).expect("second ID");

        assert_eq!(bindings.bind(2, first), Err(RunElfBindingError::OutOfRange));
        assert_eq!(bindings.get(2), None);
        assert_eq!(bindings.bind(0, first), Ok(()));
        assert_eq!(bindings.get(0), Some(first));
        assert_eq!(bindings.bind(0, second), Err(RunElfBindingError::Occupied));
        assert!(!bindings.clear(0, second));
        assert_eq!(bindings.get(0), Some(first));
        assert!(bindings.clear(0, first));
        assert_eq!(bindings.get(0), None);
        assert_eq!(bindings.bind(0, second), Ok(()));
        assert_eq!(bindings.get(0), Some(second));
    }

    #[test]
    fn run_elf_successful_start_cannot_publish_before_reset_finishes() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;

        let state = Arc::new(RunElfStateCell::new(RunElfLifecycleState::new()));
        let (reset_entered_tx, reset_entered_rx) = mpsc::channel();
        let (release_reset_tx, release_reset_rx) = mpsc::channel();
        let (observer_attempt_tx, observer_attempt_rx) = mpsc::channel();
        let (observer_acquired_tx, observer_acquired_rx) = mpsc::channel();

        let publisher_state = Arc::clone(&state);
        let publisher = std::thread::spawn(move || {
            let mut locked = publisher_state.lock();
            run_elf_start_transition(&mut locked, 1usize, || {
                reset_entered_tx.send(()).expect("signal reset entry");
                release_reset_rx.recv().expect("release reset")
            })
        });
        reset_entered_rx.recv().expect("publisher entered reset");

        let observer_state = Arc::clone(&state);
        let observer = std::thread::spawn(move || {
            observer_attempt_tx.send(()).expect("signal lock attempt");
            let locked = observer_state.lock();
            observer_acquired_tx.send(()).expect("signal lock acquired");
            locked.request().copied()
        });
        observer_attempt_rx.recv().expect("observer attempted lock");
        assert!(matches!(
            observer_acquired_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_reset_tx.send(()).expect("release publisher reset");
        assert!(matches!(
            publisher.join().expect("publisher"),
            RunElfStart::Started(_)
        ));
        assert_eq!(observer.join().expect("observer"), Some(1));
        observer_acquired_rx
            .recv()
            .expect("observer acquired after reset");
    }

    #[test]
    fn run_elf_old_completion_reset_precedes_reentrant_start() {
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::Duration;

        let mut lifecycle = RunElfLifecycleState::new();
        let first_id = match run_elf_start_transition(&mut lifecycle, 1usize, || {}) {
            RunElfStart::Started(id) => id,
            _ => panic!("first request must start"),
        };
        let state = Arc::new(RunElfStateCell::new(lifecycle));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (reset_entered_tx, reset_entered_rx) = mpsc::channel();
        let (release_reset_tx, release_reset_rx) = mpsc::channel();
        let (reentrant_attempt_tx, reentrant_attempt_rx) = mpsc::channel();
        let (reentrant_acquired_tx, reentrant_acquired_rx) = mpsc::channel();

        let completion_state = Arc::clone(&state);
        let completion_events = Arc::clone(&events);
        let completion = std::thread::spawn(move || {
            let mut locked = completion_state.lock();
            run_elf_take_completion_transition(&mut locked, first_id, || {
                completion_events.lock().unwrap().push("old-reset");
                reset_entered_tx.send(()).expect("signal old reset");
                release_reset_rx.recv().expect("release old reset")
            })
        });
        reset_entered_rx.recv().expect("completion entered reset");

        let reentrant_state = Arc::clone(&state);
        let reentrant_events = Arc::clone(&events);
        let reentrant = std::thread::spawn(move || {
            reentrant_attempt_tx.send(()).expect("signal lock attempt");
            let mut locked = reentrant_state.lock();
            reentrant_acquired_tx
                .send(())
                .expect("signal lock acquired");
            run_elf_start_transition(&mut locked, 2usize, || {
                reentrant_events.lock().unwrap().push("new-reset");
            })
        });
        reentrant_attempt_rx
            .recv()
            .expect("reentrant start attempted lock");
        assert!(matches!(
            reentrant_acquired_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_reset_tx.send(()).expect("release old reset");
        let taken = completion.join().expect("completion");
        assert_eq!(taken.completion, RunElfCompletion::Requested(1));
        let second_id = match reentrant.join().expect("reentrant") {
            RunElfStart::Started(id) => id,
            _ => panic!("reentrant request must start"),
        };
        assert_ne!(first_id, second_id);
        reentrant_acquired_rx
            .recv()
            .expect("reentrant start acquired after old reset");
        assert_eq!(*events.lock().unwrap(), ["old-reset", "new-reset"]);
        assert_eq!(state.lock().request().copied(), Some(2));
        assert_eq!(state.lock().active_id(), Some(second_id));
    }

    #[test]
    fn run_elf_state_cell_serializes_concurrent_request_publication_and_take() {
        use std::sync::Arc;

        struct State {
            lifecycle: RunElfLifecycleState<usize>,
            completed: usize,
        }

        let state = Arc::new(RunElfStateCell::new(State {
            lifecycle: RunElfLifecycleState::new(),
            completed: 0,
        }));
        let mut workers = Vec::new();
        for worker in 0..8usize {
            let state = Arc::clone(&state);
            workers.push(std::thread::spawn(move || {
                for launch in 0..1_000usize {
                    let token = worker * 1_000 + launch;
                    let launch_id = loop {
                        let mut locked = state.lock();
                        match locked.lifecycle.try_start(token) {
                            RunElfStart::Started(id) => break id,
                            RunElfStart::Busy(_) => {}
                            RunElfStart::Exhausted(_) => panic!("launch IDs exhausted"),
                        }
                        drop(locked);
                        std::thread::yield_now();
                    };

                    loop {
                        let mut locked = state.lock();
                        if locked.lifecycle.request_for(launch_id).copied() == Some(token) {
                            assert_eq!(
                                locked.lifecycle.take_completion(launch_id).completion,
                                RunElfCompletion::Requested(token)
                            );
                            locked.completed += 1;
                            break;
                        }
                        drop(locked);
                        std::thread::yield_now();
                    }
                }
            }));
        }
        for worker in workers {
            worker.join().expect("state worker");
        }

        let locked = state.lock();
        assert!(locked.lifecycle.request().is_none());
        assert_eq!(locked.completed, 8_000);
    }

    #[test]
    fn run_elf_shared_flow_covers_synchronous_and_terminal_failures() {
        type Request = RunElfActiveRequest<usize, String>;

        let mut state = RunElfLifecycleState::<Request>::new();
        let mut resets = 0usize;
        let mut callbacks = 0usize;

        assert!(!run_elf_environment_valid(
            &["INVALID"],
            "LD_LIBRARY_PATH",
            60,
            64,
            4 * 1024,
            32 * 1024,
        ));
        assert!(state.request().is_none());
        assert_eq!((resets, callbacks), (0, 0));

        let thread_failure_id =
            match run_elf_start_transition(&mut state, Request::new(1), || resets += 1) {
                RunElfStart::Started(id) => id,
                _ => panic!("thread-failure request must start"),
            };
        let thread_failure =
            match run_elf_clear_transition(&mut state, thread_failure_id, || resets += 1) {
                RunElfCompletion::Requested(request) => request,
                _ => panic!("thread failure must clear its accepted request"),
            };
        drop(thread_failure);
        assert!(state.request().is_none());
        assert_eq!((resets, callbacks), (2, 0));

        let loader_failure_id =
            match run_elf_start_transition(&mut state, Request::new(2), || resets += 1) {
                RunElfStart::Started(id) => id,
                _ => panic!("loader-failure request must start"),
            };
        assert!(matches!(
            run_elf_start_transition(&mut state, Request::new(3), || resets += 1),
            RunElfStart::Busy(_)
        ));
        assert_eq!((resets, callbacks), (3, 0));

        let loader_failure =
            run_elf_take_completion_transition(&mut state, loader_failure_id, || resets += 1);
        let completed = match loader_failure.completion {
            RunElfCompletion::Requested(request) => request,
            _ => panic!("loader failure must retain its accepted request"),
        };
        assert!(state.request().is_none());
        let (_, stack) = completed.into_parts();
        assert!(stack.is_none());
        drop(stack);
        callbacks += 1;
        assert_eq!((resets, callbacks), (4, 1));

        assert!(matches!(
            run_elf_take_completion_transition(&mut state, loader_failure_id, || resets += 1)
                .completion,
            RunElfCompletion::Repeated
        ));
        assert_eq!((resets, callbacks), (4, 1));

        assert!(matches!(
            run_elf_start_transition(&mut state, Request::new(4), || resets += 1),
            RunElfStart::Started(_)
        ));
        assert_eq!(state.request().map(|request| *request.launch()), Some(4));
        assert_eq!((resets, callbacks), (5, 1));
    }

    struct TestAllocation {
        releases: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    fn release_test_allocation(allocation: TestAllocation) {
        allocation
            .releases
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn test_allocation(
        allocations: &std::sync::atomic::AtomicUsize,
        releases: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> RunElfOwnedResource<TestAllocation> {
        allocations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        RunElfOwnedResource::new(
            TestAllocation {
                releases: std::sync::Arc::clone(releases),
            },
            release_test_allocation,
        )
    }

    #[test]
    fn run_elf_owned_resource_releases_on_post_allocation_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        fn fail_after_allocation(
            allocations: &AtomicUsize,
            releases: &Arc<AtomicUsize>,
        ) -> Result<(), ()> {
            let _stack = test_allocation(allocations, releases);
            Err(())
        }

        let allocations = AtomicUsize::new(0);
        let releases = Arc::new(AtomicUsize::new(0));
        assert_eq!(fail_after_allocation(&allocations, &releases), Err(()));
        assert_eq!(allocations.load(Ordering::SeqCst), 1);
        assert_eq!(releases.load(Ordering::SeqCst), 1);

        let mut no_request = RunElfLifecycleState::<
            RunElfActiveRequest<usize, RunElfOwnedResource<TestAllocation>>,
        >::new();
        let missing_id = RunElfLaunchId::from_raw(1).expect("nonzero launch ID");
        let unattached = run_elf_attach_resource_transition(
            &mut no_request,
            missing_id,
            test_allocation(&allocations, &releases),
        )
        .expect_err("a resource cannot attach without an active request")
        .into_resource();
        drop(unattached);
        assert_eq!(allocations.load(Ordering::SeqCst), 2);
        assert_eq!(releases.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn run_elf_stale_launch_work_cannot_mutate_reentrant_successor() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        type Request = RunElfActiveRequest<usize, RunElfOwnedResource<TestAllocation>>;

        let allocations = AtomicUsize::new(0);
        let releases = Arc::new(AtomicUsize::new(0));
        let mut state = RunElfLifecycleState::<Request>::new();
        let mut resets = 0usize;
        let mut callbacks = 0usize;

        let first_id = match run_elf_start_transition(&mut state, Request::new(1), || resets += 1) {
            RunElfStart::Started(id) => id,
            _ => panic!("first request must start"),
        };
        assert!(run_elf_attach_resource_transition(
            &mut state,
            first_id,
            test_allocation(&allocations, &releases),
        )
        .is_ok());

        let first = run_elf_take_completion_transition(&mut state, first_id, || resets += 1);
        let first = match first.completion {
            RunElfCompletion::Requested(request) => request,
            _ => panic!("first request must complete"),
        };
        assert!(state.request().is_none());
        let (first_token, first_resource) = first.into_parts();
        assert_eq!(first_token, 1);
        assert_eq!(releases.load(Ordering::SeqCst), 0);
        drop(first_resource);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
        callbacks += 1;

        let second_id = match run_elf_start_transition(&mut state, Request::new(2), || resets += 1)
        {
            RunElfStart::Started(id) => id,
            _ => panic!("reentrant request must start"),
        };
        assert_ne!(first_id, second_id);
        assert!(run_elf_attach_resource_transition(
            &mut state,
            second_id,
            test_allocation(&allocations, &releases),
        )
        .is_ok());
        assert_eq!((resets, callbacks), (3, 1));
        assert_eq!(releases.load(Ordering::SeqCst), 1);

        let stale_completion =
            run_elf_take_completion_transition(&mut state, first_id, || resets += 1);
        assert!(matches!(
            stale_completion.completion,
            RunElfCompletion::Stale
        ));

        let stale_attachment = run_elf_attach_resource_transition(
            &mut state,
            first_id,
            test_allocation(&allocations, &releases),
        )
        .expect_err("the old launch cannot attach to its successor");
        assert_eq!(releases.load(Ordering::SeqCst), 1);
        drop(stale_attachment.into_resource());
        assert_eq!(releases.load(Ordering::SeqCst), 2);

        let stale_loader_failure =
            run_elf_take_completion_transition(&mut state, first_id, || resets += 1);
        assert!(matches!(
            stale_loader_failure.completion,
            RunElfCompletion::Stale
        ));
        assert!(matches!(
            run_elf_clear_transition(&mut state, first_id, || resets += 1),
            RunElfCompletion::Stale
        ));
        assert_eq!(
            run_elf_prepare_return_transition(&mut state, first_id, 99, || resets += 1),
            RunElfTransition::Stale
        );

        assert_eq!(state.active_id(), Some(second_id));
        assert_eq!(state.request().map(|request| *request.launch()), Some(2));
        assert_eq!((resets, callbacks), (3, 1));
        assert_eq!(allocations.load(Ordering::SeqCst), 3);
        assert_eq!(releases.load(Ordering::SeqCst), 2);

        assert_eq!(
            run_elf_prepare_return_transition(&mut state, second_id, 7, || resets += 1),
            RunElfTransition::Matched
        );
        let second = run_elf_take_completion_transition(&mut state, second_id, || resets += 1);
        assert_eq!(second.exit_code, 7);
        let second = match second.completion {
            RunElfCompletion::Requested(request) => request,
            _ => panic!("second request must complete"),
        };
        assert!(state.request().is_none());
        let (second_token, second_resource) = second.into_parts();
        assert_eq!(second_token, 2);
        assert_eq!(releases.load(Ordering::SeqCst), 2);
        drop(second_resource);
        assert_eq!(releases.load(Ordering::SeqCst), 3);
        callbacks += 1;

        assert_eq!((resets, callbacks), (5, 2));
        assert_eq!(allocations.load(Ordering::SeqCst), 3);
        assert_eq!(releases.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn run_elf_stack_ownership_balances_long_reentrant_campaign_before_callbacks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        const LAUNCHES: usize = 10_000;
        let allocations = AtomicUsize::new(0);
        let releases = Arc::new(AtomicUsize::new(0));
        let mut callbacks = 0usize;
        let mut state = RunElfLifecycleState::new();

        for launch in 0..LAUNCHES {
            let request = RunElfActiveRequest::new(launch);
            let launch_id = match run_elf_start_transition(&mut state, request, || {}) {
                RunElfStart::Started(id) => id,
                _ => panic!("campaign request must start"),
            };
            assert!(run_elf_attach_resource_transition(
                &mut state,
                launch_id,
                test_allocation(&allocations, &releases),
            )
            .is_ok());
            assert_eq!(
                run_elf_prepare_return_transition(&mut state, launch_id, launch as i32, || {}),
                RunElfTransition::Matched
            );

            let taken = run_elf_take_completion_transition(&mut state, launch_id, || {});
            assert_eq!(taken.exit_code, launch as i32);
            let request = match taken.completion {
                RunElfCompletion::Requested(request) => request,
                _ => panic!("accepted request must complete"),
            };
            let (token, stack) = request.into_parts();
            assert_eq!(token, launch);
            drop(stack);
            assert_eq!(
                releases.load(Ordering::SeqCst),
                launch + 1,
                "the old EL0 stack must be released before its observer callback"
            );
            callbacks += 1;
        }

        assert_eq!(callbacks, LAUNCHES);
        assert_eq!(allocations.load(Ordering::SeqCst), LAUNCHES);
        assert_eq!(releases.load(Ordering::SeqCst), LAUNCHES);
        assert!(state.request().is_none());
    }

    #[test]
    fn run_elf_library_names_reject_traversal_and_malformed_paths() {
        for valid in [
            "ld-linux-aarch64.so.1",
            "libc.so.6",
            "/lib/ld-linux-aarch64.so.1",
            "/shared/posixtest/lib/libpthread.so.0",
        ] {
            assert!(run_elf_library_name_valid(valid), "rejected {valid:?}");
        }

        for invalid in [
            "",
            ".",
            "..",
            "../libc.so.6",
            "lib/../../libc.so.6",
            "/lib/../evil.so",
            "/lib//evil.so",
            "/lib/./evil.so",
            "libc\0.so.6",
        ] {
            assert!(!run_elf_library_name_valid(invalid), "accepted {invalid:?}");
        }
    }

    #[test]
    fn run_elf_library_resolver_stages_are_stable() {
        assert_eq!(
            run_elf_library_search_stage(0),
            Some(RunElfLibrarySearchStage::Posix)
        );
        assert_eq!(
            run_elf_library_search_stage(1),
            Some(RunElfLibrarySearchStage::Shared)
        );
        assert_eq!(
            run_elf_library_search_stage(2),
            Some(RunElfLibrarySearchStage::System)
        );
        assert_eq!(
            run_elf_library_search_stage(3),
            Some(RunElfLibrarySearchStage::Direct)
        );
        assert_eq!(run_elf_library_search_stage(4), None);
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

mod posix_test_logic_shared {
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

    #[test]
    fn harness_launcher_normalization_is_exact_and_saturating() {
        assert_eq!(normalize_scheduler_threads(7, false), 7);
        assert_eq!(normalize_scheduler_threads(7, true), 6);
        assert_eq!(normalize_scheduler_threads(0, true), 0);
    }

    #[test]
    fn coverage_tracks_noncontiguous_apis_and_groups() {
        let mut tracker = PosixCoverageTracker::default();
        tracker.select("read", "base").unwrap();
        tracker.select("write", "base").unwrap();
        tracker.select("read", "io").unwrap();

        assert_eq!(
            tracker.snapshot(),
            PosixCoverageSnapshot {
                tests_completed: 0,
                tests_selected: 3,
                apis_complete: 0,
                apis_pass: 0,
                apis_selected: 2,
                groups_complete: 0,
                groups_pass: 0,
                groups_selected: 2,
                status_counts: PosixCoverageStatusCounts::default(),
            }
        );

        let first = tracker
            .record("read", "base", PosixCoverageResult::Pass)
            .unwrap();
        assert!(!first.api_completed);
        tracker
            .record("write", "base", PosixCoverageResult::Fail)
            .unwrap();
        let last = tracker
            .record("read", "io", PosixCoverageResult::Pass)
            .unwrap();
        assert!(last.api_completed);
        assert_eq!(last.snapshot.tests_completed, 3);
        assert_eq!(last.snapshot.apis_complete, 2);
        assert_eq!(last.snapshot.apis_pass, 1);
        assert_eq!(last.snapshot.groups_complete, 2);
        assert_eq!(last.snapshot.groups_pass, 1);
        assert_eq!(last.snapshot.status_counts.passed, 2);
        assert_eq!(last.snapshot.status_counts.failed, 1);
    }

    #[test]
    fn every_nonpass_result_completes_but_does_not_pass_a_unit() {
        let cases = [
            PosixCoverageResult::Fail,
            PosixCoverageResult::Unresolved,
            PosixCoverageResult::Unsupported,
            PosixCoverageResult::Untested,
            PosixCoverageResult::LaunchError,
        ];
        for result in cases {
            let mut tracker = PosixCoverageTracker::default();
            tracker.select("api", "group").unwrap();
            let update = tracker.record("api", "group", result).unwrap();
            assert_eq!(update.snapshot.tests_completed, 1);
            assert_eq!(update.snapshot.apis_complete, 1);
            assert_eq!(update.snapshot.apis_pass, 0);
            assert_eq!(update.snapshot.groups_complete, 1);
            assert_eq!(update.snapshot.groups_pass, 0);
        }
    }

    #[test]
    fn coverage_percentages_and_progress_triggers_are_exact() {
        assert_eq!(coverage_percent_hundredths(0, 0), 0);
        assert_eq!(coverage_percent_hundredths(25, 1598), 156);
        assert_eq!(coverage_percent_hundredths(3, 195), 153);
        assert_eq!(coverage_percent_hundredths(2, 195), 102);
        assert_eq!(coverage_percent_hundredths(1598, 1598), 10_000);

        assert!(!should_emit_progress(24, 1598, false));
        assert!(should_emit_progress(25, 1598, false));
        assert!(should_emit_progress(26, 1598, true));
        assert!(should_emit_progress(1598, 1598, true));
        assert!(should_emit_progress(1, 1, true));
    }

    #[test]
    fn coverage_rejects_unknown_and_excess_completion_at_the_manifest_bound() {
        let mut tracker = PosixCoverageTracker::default();
        for _ in 0..4096 {
            tracker.select("api", "group").unwrap();
        }
        assert_eq!(tracker.snapshot().tests_selected, 4096);
        assert_eq!(
            tracker.record("missing", "group", PosixCoverageResult::Pass),
            Err(PosixCoverageError::UnknownUnit)
        );
        for _ in 0..4096 {
            tracker
                .record("api", "group", PosixCoverageResult::Pass)
                .unwrap();
        }
        assert_eq!(
            tracker.record("api", "group", PosixCoverageResult::Pass),
            Err(PosixCoverageError::TestOverComplete)
        );
    }
}

#[cfg(test)]
mod linux_child_exit_lifecycle_logic {
    use super::linux_futex_logic::{
        FutexQueue, FutexWaitOutcome, FutexWaiter, FUTEX_BITSET_MATCH_ANY,
    };
    use super::linux_syscall_context_logic::LinuxSyscallFrameOwners;
    use super::linux_task_logic::{LinuxChildExitDisposition, LinuxTaskTable, LINUX_ROOT_TID};
    use super::scheduler_logic::{
        scheduler_retired_slot_reuse_action, DeferredThreadRetirements, SchedulerSlotReuse,
    };

    fn waiter(tid: usize, scheduler_thread: usize, address: usize) -> FutexWaiter {
        FutexWaiter {
            address,
            bitset: FUTEX_BITSET_MATCH_ANY,
            tid,
            scheduler_thread,
            deadline: None,
            sequence: 0,
            outcome: FutexWaitOutcome::Waiting,
        }
    }

    #[test]
    fn child_exit_join_lifecycle_is_one_shot_and_defers_current_stack_reuse() {
        let mut clear_word = 0xfeed_beefu32;
        assert_ne!(clear_word, 0);
        let clear_address = (&mut clear_word as *mut u32) as usize;
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let exiting = tasks.reserve_child(8).unwrap();
        let peer = tasks.reserve_child(9).unwrap();
        assert!(tasks.publish(exiting));
        assert!(tasks.publish(peer));
        assert!(tasks.set_clear_child_tid(exiting.tid, 8, clear_address));

        let mut futexes = FutexQueue::<4>::new();
        futexes.push(waiter(exiting.tid, 8, 0x2000)).unwrap();
        futexes
            .push(waiter(LINUX_ROOT_TID, 7, clear_address))
            .unwrap();
        futexes.push(waiter(peer.tid, 9, clear_address)).unwrap();

        let owners = LinuxSyscallFrameOwners::<10>::new();
        assert!(owners.install(8, 0x1000, 0xaaaa, 0x1111));

        let transition = tasks
            .begin_child_exit_by_scheduler(8)
            .expect("live child begins one exit transition");
        assert_eq!(transition.task.tid, exiting.tid);
        assert_eq!(transition.slot, exiting.slot);
        assert_eq!(transition.clear_child_tid, clear_address);
        assert_eq!(
            transition.disposition,
            LinuxChildExitDisposition::ScheduleWithoutEl0Return
        );

        assert_eq!(futexes.remove_task(exiting.tid, 8), 1);
        assert!(!futexes.remove(exiting.tid, 8));
        assert!(owners.clear_owner(8));
        assert_eq!(owners.current(8), None);
        assert!(tasks.retire(exiting.tid, 8));

        let mut clear_writes = 0usize;
        let mut wake_calls = 0usize;
        clear_word = 0;
        clear_writes += 1;
        let woken = futexes.wake(clear_address, 1, FUTEX_BITSET_MATCH_ANY);
        wake_calls += 1;
        assert_eq!(clear_word, 0);
        assert_eq!(woken[0], Some((LINUX_ROOT_TID, 7)));
        assert!(woken[1..].iter().all(Option::is_none));
        assert_eq!(
            futexes.take_outcome(LINUX_ROOT_TID, 7),
            Some(FutexWaitOutcome::Woken)
        );
        assert_eq!(futexes.take_outcome(peer.tid, 9), None);

        let retirements = DeferredThreadRetirements::<1>::new();
        assert!(retirements.record_before_switch(0, 8));
        assert_eq!(retirements.take_reclaimable(0), None);
        assert_eq!(
            scheduler_retired_slot_reuse_action(true, 8, 8, Some(8), true, true),
            SchedulerSlotReuse::Unavailable
        );
        assert!(retirements.confirm_after_switch(0, 9));
        let retired = retirements.take_reclaimable(0);
        assert_eq!(retired, Some(8));
        assert_eq!(
            scheduler_retired_slot_reuse_action(true, 8, 9, retired, true, true),
            SchedulerSlotReuse::DeallocateAndReuse
        );
        assert!(owners.install(8, 0x2000, 0xbbbb, 0x2222));

        if let Some(stale) = tasks.begin_child_exit_by_scheduler(8) {
            assert_eq!(stale.clear_child_tid, clear_address);
            clear_word = 0;
            clear_writes += 1;
            let _ = futexes.wake(stale.clear_child_tid, 1, FUTEX_BITSET_MATCH_ANY);
            wake_calls += 1;
        }
        assert_eq!(clear_word, 0);
        assert_eq!(clear_writes, 1);
        assert_eq!(wake_calls, 1);
        assert_eq!(futexes.take_outcome(peer.tid, 9), None);
    }
}

#[cfg(test)]
#[path = "../../../src/user_level/services/posix_test.rs"]
mod posix_test_guest;
