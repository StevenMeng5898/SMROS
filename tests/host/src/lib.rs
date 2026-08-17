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

mod linux_record_lock_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_record_lock_logic_shared.rs"
    ));

    #[test]
    fn record_lock_ranges_normalize_all_whence_and_length_forms() {
        assert_eq!(
            normalize_linux_record_lock_range(0, 10, 20, 40, 80),
            Ok(LinuxRecordLockRange::finite(10, 30).unwrap())
        );
        assert_eq!(
            normalize_linux_record_lock_range(1, -5, 10, 40, 80),
            Ok(LinuxRecordLockRange::finite(35, 45).unwrap())
        );
        assert_eq!(
            normalize_linux_record_lock_range(2, -20, 0, 40, 80),
            Ok(LinuxRecordLockRange::to_eof(60))
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, 30, -10, 40, 80),
            Ok(LinuxRecordLockRange::finite(20, 30).unwrap())
        );
    }

    #[test]
    fn record_lock_range_errors_distinguish_invalid_from_overflow() {
        assert_eq!(
            normalize_linux_record_lock_range(3, 0, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Invalid)
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, -1, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Invalid)
        );
        assert_eq!(
            normalize_linux_record_lock_range(0, i64::MAX, 1, 0, 0),
            Err(LinuxRecordLockRangeError::Overflow)
        );
        assert_eq!(
            normalize_linux_record_lock_range(1, 0, 1, u64::MAX, 0),
            Err(LinuxRecordLockRangeError::Overflow)
        );
    }

    fn range(start: u64, end: u64) -> LinuxRecordLockRange {
        LinuxRecordLockRange::finite(start, end).unwrap()
    }

    #[test]
    fn record_lock_table_conflicts_follow_process_ownership_and_lock_kind() {
        let mut locks = LinuxRecordLockTable::<4>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();

        assert_eq!(
            locks.first_conflict(7, 100, LinuxRecordLockKind::Write, range(20, 40)),
            None
        );
        assert_eq!(
            locks
                .first_conflict(7, 101, LinuxRecordLockKind::Read, range(20, 40))
                .unwrap()
                .owner,
            100
        );

        let mut reads = LinuxRecordLockTable::<4>::new();
        reads
            .set(7, 100, LinuxRecordLockKind::Read, range(0, 100))
            .unwrap();
        assert_eq!(
            reads.first_conflict(7, 101, LinuxRecordLockKind::Read, range(0, 100)),
            None
        );
        assert!(reads
            .first_conflict(7, 101, LinuxRecordLockKind::Write, range(0, 100))
            .is_some());
    }

    #[test]
    fn record_lock_table_replacement_splits_and_coalesces_owner_ranges() {
        let mut locks = LinuxRecordLockTable::<4>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(7, 100, LinuxRecordLockKind::Read, range(20, 80))
            .unwrap();

        assert_eq!(
            locks.snapshot(),
            [
                Some(LinuxRecordLock {
                    file_id: 7,
                    owner: 100,
                    kind: LinuxRecordLockKind::Write,
                    range: range(0, 20),
                }),
                Some(LinuxRecordLock {
                    file_id: 7,
                    owner: 100,
                    kind: LinuxRecordLockKind::Read,
                    range: range(20, 80),
                }),
                Some(LinuxRecordLock {
                    file_id: 7,
                    owner: 100,
                    kind: LinuxRecordLockKind::Write,
                    range: range(80, 100),
                }),
                None,
            ]
        );

        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(20, 80))
            .unwrap();
        assert_eq!(
            locks.snapshot(),
            [
                Some(LinuxRecordLock {
                    file_id: 7,
                    owner: 100,
                    kind: LinuxRecordLockKind::Write,
                    range: range(0, 100),
                }),
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn record_lock_table_unlock_splits_only_matching_file_and_owner() {
        let mut locks = LinuxRecordLockTable::<6>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(8, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(7, 101, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks.unlock(7, 100, range(20, 80)).unwrap();

        let snapshot = locks.snapshot();
        assert!(snapshot.contains(&Some(LinuxRecordLock {
            file_id: 7,
            owner: 100,
            kind: LinuxRecordLockKind::Write,
            range: range(0, 20),
        })));
        assert!(snapshot.contains(&Some(LinuxRecordLock {
            file_id: 7,
            owner: 100,
            kind: LinuxRecordLockKind::Write,
            range: range(80, 100),
        })));
        assert!(snapshot.contains(&Some(LinuxRecordLock {
            file_id: 8,
            owner: 100,
            kind: LinuxRecordLockKind::Write,
            range: range(0, 100),
        })));
        assert!(snapshot.contains(&Some(LinuxRecordLock {
            file_id: 7,
            owner: 101,
            kind: LinuxRecordLockKind::Write,
            range: range(0, 100),
        })));
    }

    #[test]
    fn record_lock_table_zero_length_range_remains_open_through_growth() {
        let mut locks = LinuxRecordLockTable::<2>::new();
        locks
            .set(
                7,
                100,
                LinuxRecordLockKind::Write,
                LinuxRecordLockRange::to_eof(50),
            )
            .unwrap();

        assert!(locks
            .first_conflict(7, 101, LinuxRecordLockKind::Read, range(1_000, 1_100))
            .is_some());
    }

    #[test]
    fn record_lock_table_capacity_failure_is_atomic() {
        let mut locks = LinuxRecordLockTable::<2>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        let before = locks.snapshot();

        assert_eq!(
            locks.set(7, 100, LinuxRecordLockKind::Read, range(20, 80)),
            Err(LinuxRecordLockTableError::Capacity)
        );
        assert_eq!(locks.snapshot(), before);
    }

    #[test]
    fn record_lock_table_release_is_scoped_and_fork_does_not_copy_ownership() {
        let mut locks = LinuxRecordLockTable::<6>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(8, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(7, 101, LinuxRecordLockKind::Read, range(100, 200))
            .unwrap();

        assert!(locks
            .first_conflict(7, 200, LinuxRecordLockKind::Write, range(1, 100))
            .is_some());
        assert!(!locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| lock.owner == 200));

        locks.release_owner_file(100, 7);
        assert!(locks
            .first_conflict(7, 200, LinuxRecordLockKind::Write, range(1, 100))
            .is_none());
        assert!(locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| { lock.file_id == 8 && lock.owner == 100 }));
        assert!(locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| { lock.file_id == 7 && lock.owner == 101 }));

        locks.release_owner(100);
        assert!(!locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| lock.owner == 100));
        assert!(locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| lock.owner == 101));
    }

    #[test]
    fn record_lock_duplicate_close_is_idempotent_and_file_scoped() {
        let mut locks = LinuxRecordLockTable::<4>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        locks
            .set(8, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();

        locks.release_owner_file(100, 7);
        locks.release_owner_file(100, 7);

        assert!(locks
            .first_conflict(7, 200, LinuxRecordLockKind::Write, range(1, 100))
            .is_none());
        assert!(locks
            .first_conflict(8, 200, LinuxRecordLockKind::Write, range(1, 100))
            .is_some());
    }

    #[test]
    fn record_lock_child_exit_and_fork_rollback_preserve_parent_ownership() {
        let mut locks = LinuxRecordLockTable::<4>::new();
        locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();

        locks.release_owner(200);
        locks.release_owner(200);

        assert_eq!(
            locks
                .first_conflict(7, 200, LinuxRecordLockKind::Write, range(1, 100))
                .map(|lock| lock.owner),
            Some(100)
        );
        assert!(!locks
            .snapshot()
            .iter()
            .flatten()
            .any(|lock| lock.owner == 200));
    }

    #[test]
    fn record_lock_parent_exit_wakes_waiting_child() {
        let mut state = LinuxRecordLockState::<4, 2>::new();
        state
            .locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        state
            .push(LinuxRecordLockWaiter::new(
                7,
                200,
                LinuxRecordLockKind::Write,
                range(1, 100),
                20,
                30,
            ))
            .unwrap();

        assert_eq!(state.wake_ready(), [None, None]);
        state.locks.release_owner(100);
        assert_eq!(state.wake_ready(), [Some((20, 30)), None]);
    }

    #[test]
    fn record_lock_waiter_interrupt_records_terminal_outcome() {
        let mut state = LinuxRecordLockState::<2, 2>::new();
        let waiter =
            LinuxRecordLockWaiter::new(7, 101, LinuxRecordLockKind::Write, range(20, 40), 11, 12);

        assert_eq!(state.push(waiter), Ok(()));
        assert!(state.interrupt(11, 12));
        assert_eq!(
            state.take_outcome(11, 12),
            Some(LinuxRecordLockWaitOutcome::Interrupted)
        );
        assert_eq!(state.take_outcome(11, 12), None);
    }

    #[test]
    fn record_lock_waiter_publication_is_fifo_and_task_unique() {
        let mut state = LinuxRecordLockState::<2, 3>::new();
        let first =
            LinuxRecordLockWaiter::new(7, 101, LinuxRecordLockKind::Write, range(20, 40), 11, 12);
        let second =
            LinuxRecordLockWaiter::new(7, 102, LinuxRecordLockKind::Write, range(20, 40), 13, 14);

        assert_eq!(state.push(first), Ok(()));
        assert_eq!(state.push(second), Ok(()));
        assert_eq!(
            state.push(first),
            Err(LinuxRecordLockWaiterError::Duplicate)
        );
        assert_eq!(state.waiter_snapshot()[0].unwrap().sequence, 0);
        assert_eq!(state.waiter_snapshot()[1].unwrap().sequence, 1);
    }

    #[test]
    fn record_lock_waiter_capacity_failure_preserves_existing_waiters() {
        let mut state = LinuxRecordLockState::<2, 1>::new();
        let first =
            LinuxRecordLockWaiter::new(7, 101, LinuxRecordLockKind::Write, range(20, 40), 11, 12);
        let second =
            LinuxRecordLockWaiter::new(7, 102, LinuxRecordLockKind::Write, range(20, 40), 13, 14);
        state.push(first).unwrap();
        let before = state.waiter_snapshot();

        assert_eq!(
            state.push(second),
            Err(LinuxRecordLockWaiterError::Capacity)
        );
        assert_eq!(state.waiter_snapshot(), before);
    }

    #[test]
    fn record_lock_waiters_wake_in_fifo_order_only_after_conflict_clears() {
        let mut state = LinuxRecordLockState::<4, 3>::new();
        state
            .locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        state
            .push(LinuxRecordLockWaiter::new(
                7,
                101,
                LinuxRecordLockKind::Write,
                range(20, 40),
                11,
                12,
            ))
            .unwrap();
        state
            .push(LinuxRecordLockWaiter::new(
                7,
                102,
                LinuxRecordLockKind::Read,
                range(30, 50),
                13,
                14,
            ))
            .unwrap();

        assert_eq!(state.wake_ready(), [None, None, None]);
        state.locks.release_owner_file(100, 7);
        assert_eq!(state.wake_ready(), [Some((11, 12)), Some((13, 14)), None]);
        assert_eq!(
            state.take_outcome(11, 12),
            Some(LinuxRecordLockWaitOutcome::Woken)
        );
        assert_eq!(
            state.take_outcome(13, 14),
            Some(LinuxRecordLockWaitOutcome::Woken)
        );
    }

    #[test]
    fn record_lock_waiter_task_cleanup_and_reset_are_scoped() {
        let mut state = LinuxRecordLockState::<4, 3>::new();
        state
            .locks
            .set(7, 100, LinuxRecordLockKind::Write, range(0, 100))
            .unwrap();
        state
            .push(LinuxRecordLockWaiter::new(
                7,
                101,
                LinuxRecordLockKind::Write,
                range(20, 40),
                11,
                12,
            ))
            .unwrap();
        state
            .push(LinuxRecordLockWaiter::new(
                7,
                102,
                LinuxRecordLockKind::Write,
                range(20, 40),
                13,
                14,
            ))
            .unwrap();

        assert_eq!(state.remove_task(11, 12), 1);
        assert_eq!(state.remove_task(11, 12), 0);
        assert!(state
            .waiter_snapshot()
            .iter()
            .flatten()
            .any(|waiter| { waiter.tid == 13 && waiter.scheduler_thread == 14 }));

        state.reset();
        assert_eq!(state.locks.snapshot(), [None; 4]);
        assert_eq!(state.waiter_snapshot(), [None; 3]);
    }
}

mod syscall_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall_logic_shared.rs"
    ));

    #[test]
    fn linux_fcntl_recognizes_record_lock_commands() {
        for command in 0usize..=7 {
            assert!(smros_linux_fcntl_cmd_supported_body!(
                command, 0, 1, 2, 3, 4, 5, 6, 7, 1030
            ));
        }
        assert!(smros_linux_fcntl_cmd_supported_body!(
            1030usize, 0, 1, 2, 3, 4, 5, 6, 7, 1030
        ));
        assert!(!smros_linux_fcntl_cmd_supported_body!(
            8usize, 0, 1, 2, 3, 4, 5, 6, 7, 1030
        ));
    }

    #[test]
    fn clock_nanosleep_accepts_only_relative_or_timer_abstime_flags() {
        const TIMER_ABSTIME: usize = 1;
        assert!(smros_linux_clock_nanosleep_flags_valid_body!(
            0,
            TIMER_ABSTIME
        ));
        assert!(smros_linux_clock_nanosleep_flags_valid_body!(
            TIMER_ABSTIME,
            TIMER_ABSTIME
        ));
        assert!(!smros_linux_clock_nanosleep_flags_valid_body!(
            2,
            TIMER_ABSTIME
        ));
        assert!(!smros_linux_clock_nanosleep_flags_valid_body!(
            usize::MAX,
            TIMER_ABSTIME
        ));
    }

    #[test]
    fn clock_gettime_accepts_glibc_time_coarse_clock_ids() {
        const CLOCK_REALTIME: usize = 0;
        const CLOCK_MONOTONIC: usize = 1;
        const CLOCK_PROCESS_CPUTIME_ID: usize = 2;
        const CLOCK_THREAD_CPUTIME_ID: usize = 3;
        const CLOCK_MONOTONIC_RAW: usize = 4;
        const CLOCK_REALTIME_COARSE: usize = 5;
        const CLOCK_MONOTONIC_COARSE: usize = 6;
        const CLOCK_BOOTTIME: usize = 7;

        for clock_id in [
            CLOCK_REALTIME,
            CLOCK_MONOTONIC,
            CLOCK_PROCESS_CPUTIME_ID,
            CLOCK_THREAD_CPUTIME_ID,
            CLOCK_MONOTONIC_RAW,
            CLOCK_REALTIME_COARSE,
            CLOCK_MONOTONIC_COARSE,
            CLOCK_BOOTTIME,
        ] {
            assert!(smros_linux_clock_id_supported_body!(clock_id));
        }
        assert!(!smros_linux_clock_id_supported_body!(8));
        assert!(!smros_linux_clock_id_supported_body!(usize::MAX));
    }

    #[test]
    fn posix_realtime_offset_is_checked_and_monotonic_is_not_settable() {
        assert!(linux_posix_clock_settable(0));
        assert!(!linux_posix_clock_settable(1));
        assert!(!linux_posix_clock_settable(usize::MAX));

        assert_eq!(linux_posix_timespec_nanoseconds(2, 3), Some(2_000_000_003));
        assert_eq!(linux_posix_timespec_nanoseconds(-1, 0), None);
        assert_eq!(linux_posix_timespec_nanoseconds(0, -1), None);
        assert_eq!(linux_posix_timespec_nanoseconds(0, 1_000_000_000), None);

        assert_eq!(
            linux_realtime_offset_for_set(3_000_000_000, 2, 0),
            Some(-1_000_000_000)
        );
        assert_eq!(
            linux_realtime_from_offset(3_000_000_000, -1_000_000_000),
            Some(2_000_000_000)
        );
        assert_eq!(linux_realtime_from_offset(1, -2), None);
        assert_eq!(linux_realtime_from_offset(u64::MAX, 1), None);
    }

    #[test]
    fn posix_relative_and_absolute_timers_use_the_correct_clock_domain() {
        let mut relative = LinuxPosixTimerCore::new(7, LinuxPosixClock::Realtime, 14);
        relative
            .arm(
                false,
                100,
                LinuxPosixTimerSpec {
                    interval: 0,
                    value: 50,
                },
            )
            .unwrap();
        assert_eq!(relative.snapshot(120, 20_000).value, 30);
        assert!(!relative.expire(149, 30_000));
        assert!(relative.expire(150, 30_000));
        assert_eq!(relative.snapshot(150, 30_000).value, 0);

        let mut absolute = LinuxPosixTimerCore::new(8, LinuxPosixClock::Realtime, 14);
        absolute
            .arm(
                true,
                100,
                LinuxPosixTimerSpec {
                    interval: 0,
                    value: 1_050,
                },
            )
            .unwrap();
        assert!(!absolute.expire(200, 1_049));
        assert!(absolute.expire(200, 1_050));
    }

    #[test]
    fn posix_timer_disarm_query_and_periodic_reschedule_are_one_shot_per_scan() {
        let mut timer = LinuxPosixTimerCore::new(9, LinuxPosixClock::Monotonic, 10);
        timer
            .arm(
                false,
                100,
                LinuxPosixTimerSpec {
                    interval: 20,
                    value: 10,
                },
            )
            .unwrap();
        assert!(timer.expire(150, 900));
        let snapshot = timer.snapshot(150, 900);
        assert_eq!(snapshot.interval, 20);
        assert_eq!(snapshot.value, 20);
        assert!(!timer.expire(150, 900));

        timer
            .arm(
                false,
                150,
                LinuxPosixTimerSpec {
                    interval: 20,
                    value: 0,
                },
            )
            .unwrap();
        assert_eq!(timer.snapshot(150, 900), LinuxPosixTimerSpec::DISARMED);
    }

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

        assert!(smros_linux_signal_valid_body!(0usize, max_signal));
        assert!(smros_linux_signal_valid_body!(1usize, max_signal));
        assert!(smros_linux_signal_valid_body!(64usize, max_signal));
        assert!(!smros_linux_signal_valid_body!(65usize, max_signal));

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

    #[test]
    fn fxfs_stat_identity_uses_distinct_nonzero_object_ids_as_inodes() {
        assert_eq!(linux_fxfs_stat_identity(2), Some((1, 2)));
        assert_eq!(linux_fxfs_stat_identity(3), Some((1, 3)));
        assert_ne!(linux_fxfs_stat_identity(2), linux_fxfs_stat_identity(3));
        assert_eq!(linux_fxfs_stat_identity(0), None);
    }
}

mod linux_mqueue_logic {
    extern crate alloc;

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_mqueue_logic_shared.rs"
    ));

    fn attr(maxmsg: usize, msgsize: usize) -> LinuxMqueueAttr {
        LinuxMqueueAttr {
            flags: 0,
            maxmsg,
            msgsize,
            curmsgs: 0,
        }
    }

    #[test]
    fn open_normalizes_linux_syscall_names_without_leading_slash() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();

        let opened = state
            .open("smros-mq", 101, true, false, Some(attr(3, 16)))
            .unwrap();
        assert!(opened.created);

        let reopened = state.open("/smros-mq", 102, false, false, None).unwrap();
        assert!(!reopened.created);

        state.send(101, b"hello", 4).unwrap();
        let received = state.receive(102, 16).unwrap().message;
        assert_eq!(received.bytes, b"hello");
        assert_eq!(received.priority, 4);
    }

    #[test]
    fn open_preserves_attributes_and_rejects_exclusive_existing_queue() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();

        let opened = state
            .open("/smros-mq", 101, true, false, Some(attr(3, 16)))
            .unwrap();
        assert!(opened.created);
        assert_eq!(opened.attr.maxmsg, 3);
        assert_eq!(opened.attr.msgsize, 16);

        assert_eq!(
            state.open("/smros-mq", 102, true, true, None),
            Err(LinuxMqueueError::Exists)
        );

        let reopened = state
            .open("/smros-mq", 103, true, false, Some(attr(1, 1)))
            .unwrap();
        assert!(!reopened.created);
        assert_eq!(reopened.attr.maxmsg, 3);
        assert_eq!(reopened.attr.msgsize, 16);
    }

    #[test]
    fn send_receive_uses_priority_order_and_fifo_within_equal_priority() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(4, 16)))
            .unwrap();

        state.send(101, b"low-first", 1).unwrap();
        state.send(101, b"high", 3).unwrap();
        state.send(101, b"low-second", 1).unwrap();

        let first = state.receive(101, 16).unwrap().message;
        let second = state.receive(101, 16).unwrap().message;
        let third = state.receive(101, 16).unwrap().message;

        assert_eq!(first.bytes, b"high");
        assert_eq!(first.priority, 3);
        assert_eq!(second.bytes, b"low-first");
        assert_eq!(second.priority, 1);
        assert_eq!(third.bytes, b"low-second");
        assert_eq!(third.priority, 1);
    }

    #[test]
    fn message_size_priority_empty_and_full_errors_match_posix() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(1, 4)))
            .unwrap();

        assert_eq!(
            state.send(101, b"12345", 0),
            Err(LinuxMqueueError::MessageTooLarge)
        );
        assert_eq!(
            state.send(101, b"1234", LINUX_MQ_PRIO_MAX),
            Err(LinuxMqueueError::Invalid)
        );
        assert_eq!(
            state.receive(101, 4),
            Err(LinuxMqueueError::WouldBlock)
        );

        state.send(101, b"1234", 0).unwrap();
        assert_eq!(
            state.send(101, b"5678", 0),
            Err(LinuxMqueueError::WouldBlock)
        );
        assert_eq!(
            state.receive(101, 3),
            Err(LinuxMqueueError::MessageTooLarge)
        );
        assert_eq!(state.getattr(101, 0).unwrap().curmsgs, 1);
    }

    #[test]
    fn unlink_hides_name_but_keeps_open_handle_usable_until_close() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(2, 8)))
            .unwrap();
        state.unlink("/smros-mq").unwrap();

        assert_eq!(
            state.open("/smros-mq", 102, false, false, None),
            Err(LinuxMqueueError::NotFound)
        );
        state.send(101, b"old", 0).unwrap();
        assert_eq!(state.receive(101, 8).unwrap().message.bytes, b"old");

        assert!(state.close_handle(101));
        assert_eq!(state.getattr(101, 0), Err(LinuxMqueueError::BadDescriptor));
    }

    #[test]
    fn waiters_wake_by_operation_and_report_timeout_or_interrupt() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(1, 8)))
            .unwrap();

        state
            .push_waiter(
                101,
                LinuxMqueueWaitKind::Receive,
                11,
                12,
                Some(LinuxMqueueDeadline { ticks: 50 }),
            )
            .unwrap();
        assert_eq!(state.send(101, b"wake", 0).unwrap().receiver, Some((11, 12)));
        assert_eq!(
            state.take_outcome(11, 12),
            Some(LinuxMqueueWaitOutcome::Woken)
        );

        state
            .push_waiter(
                101,
                LinuxMqueueWaitKind::Send,
                21,
                22,
                Some(LinuxMqueueDeadline { ticks: 70 }),
            )
            .unwrap();
        assert_eq!(state.expire(69), [None, None, None, None]);
        assert_eq!(state.expire(70)[0], Some((21, 22)));
        assert_eq!(
            state.take_outcome(21, 22),
            Some(LinuxMqueueWaitOutcome::TimedOut)
        );

        state
            .push_waiter(101, LinuxMqueueWaitKind::Receive, 31, 32, None)
            .unwrap();
        assert!(state.interrupt(31, 32));
        assert_eq!(
            state.take_outcome(31, 32),
            Some(LinuxMqueueWaitOutcome::Interrupted)
        );
    }

    #[test]
    fn notify_is_single_registrant_and_fires_once_on_empty_to_nonempty() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(2, 8)))
            .unwrap();

        let notification = LinuxMqueueNotification {
            handle: 101,
            pid: 42,
            signum: 10,
        };
        state.notify(101, Some(notification)).unwrap();
        assert_eq!(
            state.notify(
                101,
                Some(LinuxMqueueNotification {
                    handle: 101,
                    pid: 43,
                    signum: 12,
                })
            ),
            Err(LinuxMqueueError::Busy)
        );

        assert_eq!(
            state.send(101, b"first", 0).unwrap().notification,
            Some(notification)
        );
        assert_eq!(state.send(101, b"second", 0).unwrap().notification, None);

        state.receive(101, 8).unwrap();
        state.receive(101, 8).unwrap();
        state.notify(101, Some(notification)).unwrap();
        state.notify(101, None).unwrap();
        assert_eq!(state.send(101, b"silent", 0).unwrap().notification, None);
    }

    #[test]
    fn close_of_registering_handle_releases_notification_registration() {
        let mut state = LinuxMqueueState::<4, 8, 4>::new();
        state
            .open("/smros-mq", 101, true, false, Some(attr(2, 8)))
            .unwrap();
        state.open("/smros-mq", 102, false, false, None).unwrap();

        state
            .notify(
                101,
                Some(LinuxMqueueNotification {
                    handle: 101,
                    pid: 42,
                    signum: 10,
                }),
            )
            .unwrap();
        assert!(state.close_handle(101));

        state
            .notify(
                102,
                Some(LinuxMqueueNotification {
                    handle: 102,
                    pid: 43,
                    signum: 0,
                }),
            )
            .unwrap();
    }
}

mod linux_task_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_task_logic_shared.rs"
    ));

    pub(crate) fn scoped_process_signal_target<const N: usize>(
        tasks: &LinuxTaskTable<N>,
        tgid: usize,
        signum: usize,
    ) -> Option<LinuxTaskCore> {
        crate::linux_process_logic::select_linux_process_signal_target(
            &tasks.tasks,
            tgid,
            linux_signal_bit(signum),
            |task| LinuxTaskTable::<N>::is_live(task),
            |task| task.tgid,
            |slot| tasks.signal_states[slot].mask,
        )
    }

    pub(crate) fn retire_process_tasks_for_behavior<const N: usize>(
        tasks: &mut LinuxTaskTable<N>,
        tgid: usize,
    ) -> usize {
        let snapshot = tasks.tasks;
        crate::linux_process_logic::for_each_linux_process_task(
            &snapshot,
            tgid,
            None,
            |task| LinuxTaskTable::<N>::is_live(task),
            |task| task.tgid,
            |task| task.scheduler_thread,
            |_, task| {
                tasks
                    .exit_with_clear_child_tid(task.tid, task.scheduler_thread)
                    .is_some()
                    && tasks.retire(task.tid, task.scheduler_thread)
            },
        )
    }

    pub(crate) fn live_process_task_count<const N: usize>(
        tasks: &LinuxTaskTable<N>,
        tgid: usize,
    ) -> usize {
        tasks
            .tasks
            .iter()
            .copied()
            .filter(|task| task.tgid == tgid && LinuxTaskTable::<N>::is_live(*task))
            .count()
    }

    pub(crate) fn prepare_fork_task_signal_state_for_behavior<const N: usize>(
        tasks: &mut LinuxTaskTable<N>,
        reservation: LinuxTaskReservation,
        parent_scheduler_thread: usize,
    ) {
        let parent_slot = tasks
            .tasks
            .iter()
            .position(|task| {
                LinuxTaskTable::<N>::is_live(*task)
                    && task.scheduler_thread == parent_scheduler_thread
            })
            .unwrap();
        let parent_mask = tasks.signal_states[parent_slot].mask;
        crate::linux_process_logic::prepare_linux_fork_task_signal_state(
            &mut tasks.signal_states[reservation.slot],
            parent_mask,
            |signal_state| signal_state.reset_in_place(),
            |signal_state, mask| signal_state.mask = mask,
        );
    }

    const PTHREAD_BASE_FLAGS: usize =
        CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM;

    #[test]
    fn synchronous_fault_record_uses_aarch64_linux_siginfo_layout() {
        let record = LinuxPendingSignal::synchronous_fault(11, 2, 0x1234_5678_9abc_def0);
        assert_eq!(record.signum, 11);
        assert!(record.has_info);
        assert_eq!(
            i32::from_ne_bytes(record.info[0..4].try_into().unwrap()),
            11
        );
        assert_eq!(i32::from_ne_bytes(record.info[4..8].try_into().unwrap()), 0);
        assert_eq!(
            i32::from_ne_bytes(record.info[8..12].try_into().unwrap()),
            2
        );
        assert_eq!(
            u64::from_ne_bytes(record.info[16..24].try_into().unwrap()),
            0x1234_5678_9abc_def0
        );
    }

    #[test]
    fn synchronous_fault_ucontext_core_preserves_faulting_aarch64_state() {
        let regs = core::array::from_fn::<u64, 32, _>(|index| 0x1000 + index as u64);
        let core = linux_aarch64_ucontext_core(
            0xdead_beef,
            regs,
            0x1fff_f000,
            0x1234_5000,
            0x6000_0000,
            0x55aa,
            LinuxSignalStack::DISABLED,
        );
        assert_eq!(LINUX_AARCH64_UCONTEXT_BYTES, 4560);
        assert_eq!(core.len(), LINUX_AARCH64_UCONTEXT_CORE_BYTES);
        assert_eq!(u64::from_ne_bytes(core[40..48].try_into().unwrap()), 0x55aa);
        assert_eq!(
            u64::from_ne_bytes(core[176..184].try_into().unwrap()),
            0xdead_beef
        );
        assert_eq!(
            u64::from_ne_bytes(core[184..192].try_into().unwrap()),
            regs[0]
        );
        assert_eq!(
            u64::from_ne_bytes(core[424..432].try_into().unwrap()),
            regs[30]
        );
        assert_eq!(
            u64::from_ne_bytes(core[432..440].try_into().unwrap()),
            0x1fff_f000
        );
        assert_eq!(
            u64::from_ne_bytes(core[440..448].try_into().unwrap()),
            0x1234_5000
        );
        assert_eq!(
            u64::from_ne_bytes(core[448..456].try_into().unwrap()),
            0x6000_0000
        );
    }

    #[test]
    fn synchronous_fault_user_frame_is_aligned_bounded_and_non_overlapping() {
        let (sp, info, context) = linux_aarch64_signal_user_frame(0x20_000).unwrap();
        assert_eq!(sp & 0xf, 0);
        assert_eq!(info, sp);
        assert_eq!(context, info + LINUX_SIGNAL_INFO_BYTES as u64);
        assert!(context + LINUX_AARCH64_UCONTEXT_BYTES as u64 <= 0x20_000);
        assert_eq!(linux_aarch64_signal_user_frame(1), None);
    }

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

        let first = tasks
            .reserve_child(LINUX_ROOT_TID, 8)
            .expect("first child reservation");
        assert_eq!(first.tid, 2);
        assert_eq!(tasks.by_tid(first.tid), None);
        assert_eq!(tasks.by_scheduler(8), None);
        assert!(tasks.publish(first));
        assert_eq!(tasks.by_scheduler(8).map(|task| task.tid), Some(2));

        assert!(tasks.exit(first.tid, 8));
        assert!(tasks.retire(first.tid, 8));
        let second = tasks
            .reserve_child(LINUX_ROOT_TID, 9)
            .expect("reused table slot");
        assert_eq!(second.tid, 3);
        assert_ne!(first.tid, second.tid);
        assert!(!tasks.publish(first), "stale reservation must not publish");
        assert!(tasks.publish(second));
    }

    #[test]
    fn unpublished_fork_task_can_roll_back_after_task_publication() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();

        assert!(tasks.publish(child));
        assert!(tasks.rollback(child));
        assert_eq!(tasks.by_tid(child.tid), None);
        assert_eq!(tasks.by_scheduler(child.scheduler_thread), None);
    }

    #[test]
    fn task_state_and_scheduler_identity_move_together() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
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
    fn child_wait_blocking_is_tgid_scoped_and_wakes_once() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let other_process = tasks.reserve_child(2, 8).unwrap();
        assert!(tasks.publish(other_process));

        assert!(tasks.block(LINUX_ROOT_TID, 7, LinuxBlockReason::ChildWait,));
        assert_eq!(
            tasks.by_tid(LINUX_ROOT_TID).map(|task| task.block_reason),
            Some(LinuxBlockReason::ChildWait)
        );
        assert_eq!(
            tasks.by_tid(other_process.tid).map(|task| task.tgid),
            Some(2)
        );
        assert!(tasks.wake(LINUX_ROOT_TID, 7));
        assert!(!tasks.wake(LINUX_ROOT_TID, 7));
    }

    #[test]
    fn child_waiter_snapshot_includes_every_waiter_in_the_parent_tgid() {
        let mut tasks = LinuxTaskTable::<4>::new();
        tasks.register_root(7).unwrap();
        let peer = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        let other_process = tasks.reserve_child(2, 9).unwrap();
        assert!(tasks.publish(peer));
        assert!(tasks.publish(other_process));

        assert!(tasks.block(LINUX_ROOT_TID, 7, LinuxBlockReason::ChildWait));
        assert!(tasks.block(peer.tid, 8, LinuxBlockReason::ChildWait));
        assert!(tasks.block(other_process.tid, 9, LinuxBlockReason::ChildWait,));

        let waiters: Vec<_> = tasks
            .child_waiters(LINUX_ROOT_TID)
            .into_iter()
            .flatten()
            .map(|task| task.tid)
            .collect();
        assert_eq!(waiters, vec![LINUX_ROOT_TID, peer.tid]);
    }

    #[test]
    fn rollback_releases_only_the_matching_starting_reservation() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let first = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();

        assert_eq!(tasks.scheduler_thread_for_reset(first.slot), Some(8));
        assert_eq!(tasks.scheduler_thread_for_reset(usize::MAX), None);

        assert!(!tasks.rollback(LinuxTaskReservation {
            scheduler_thread: 99,
            ..first
        }));
        assert!(tasks.rollback(first));
        assert!(!tasks.rollback(first));

        let second = tasks
            .reserve_child(LINUX_ROOT_TID, 9)
            .expect("rolled-back slot");
        assert_eq!(second.slot, first.slot);
        assert_eq!(second.tid, first.tid + 1);
        assert!(!tasks.publish(first));
        assert!(tasks.publish(second));
    }

    #[test]
    fn invalid_and_stale_transitions_leave_the_live_task_unchanged() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();

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
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
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
    fn launch_reset_preserves_boot_tid_high_water() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        assert_eq!(child.tid, 2);
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 9), None);
        assert!(tasks.rollback(child));
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap().tid, 3);

        tasks.reset();
        assert_eq!(tasks.by_scheduler(7), None);
        assert_eq!(tasks.register_root(10), Ok(LINUX_ROOT_TID));
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 11).unwrap().tid, 4);
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

        let last = tasks
            .reserve_child(LINUX_ROOT_TID, 8)
            .expect("last valid Linux TID");
        assert_eq!(last.tid, LINUX_MAX_TID);
        assert!(tasks.rollback(last));
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 9), None);

        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 9), None);

        tasks.reset();
        tasks.register_root(10).unwrap();
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 11), None);
    }

    #[test]
    fn out_of_range_next_tid_does_not_mutate_a_slot_and_exhausts_permanently() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        tasks.next_tid = LINUX_MAX_TID + 1;
        let slots_before = tasks.tasks;

        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 8), None);
        assert_eq!(tasks.tasks, slots_before);

        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 8), None);
        assert_eq!(tasks.tasks, slots_before);
    }

    #[test]
    fn allocator_exhaustion_remains_permanent_across_launch_reset() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();
        tasks.next_tid = usize::MAX;

        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 8), None);
        tasks.next_tid = 2;
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 8), None);

        tasks.reset();
        tasks.register_root(9).unwrap();
        assert_eq!(tasks.reserve_child(LINUX_ROOT_TID, 10), None);
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
    fn pending_and_task_signal_state_reset_in_place_matches_new_state() {
        let mut pending = LinuxPendingSignals::new();
        pending.queue(signal_record(10, 0x10)).unwrap();
        pending.queue(signal_record(35, 0x35)).unwrap();
        assert!(matches!(
            pending.reserve_direct(signal_record(11, 0x11)).unwrap(),
            Some(LinuxPendingSignalReservation::Standard(11))
        ));
        assert!(matches!(
            pending.reserve_direct(signal_record(36, 0x36)).unwrap(),
            Some(LinuxPendingSignalReservation::Realtime(_))
        ));
        assert_ne!(pending.standard_pending, 0);
        assert_ne!(pending.standard_reserved, 0);
        assert_ne!(pending.realtime_len, 0);
        assert_ne!(pending.realtime_reserved, 0);

        pending.reset_in_place();
        let expected_pending = LinuxPendingSignals::new();
        assert_eq!(pending.standard_pending, expected_pending.standard_pending);
        assert_eq!(
            pending.standard_reserved,
            expected_pending.standard_reserved
        );
        assert_eq!(pending.standard_records, expected_pending.standard_records);
        assert_eq!(pending.realtime_pending, expected_pending.realtime_pending);
        assert_eq!(
            pending.realtime_sequences,
            expected_pending.realtime_sequences
        );
        assert_eq!(pending.realtime_len, expected_pending.realtime_len);
        assert_eq!(
            pending.realtime_reservations,
            expected_pending.realtime_reservations
        );
        assert_eq!(
            pending.realtime_reserved,
            expected_pending.realtime_reserved
        );
        assert_eq!(
            pending.next_realtime_sequence,
            expected_pending.next_realtime_sequence
        );

        let mut state = LinuxTaskSignalState::new();
        state.mask = linux_signal_bit(12);
        state.alt_stack = LinuxSignalStack {
            sp: 0x4000,
            flags: LINUX_SS_ONSTACK as u32,
            _padding: 0xfeed_beef,
            size: 0x2000,
        };
        state.queue(signal_record(12, 0x12)).unwrap();
        state.queue(signal_record(37, 0x37)).unwrap();
        assert!(state
            .pending
            .reserve_direct(signal_record(13, 0x13))
            .unwrap()
            .is_some());
        assert!(state
            .pending
            .reserve_direct(signal_record(38, 0x38))
            .unwrap()
            .is_some());
        let restart = LinuxRestartBlock {
            syscall_number: 98,
            arguments: [1, 2, 3, 4, 5, 6],
            svc_address: 0x8000,
            timeout: LinuxRestartTimeout::Deadline {
                ticks: 99,
                realtime: true,
            },
        };
        assert_eq!(
            state.push_frame(LinuxSignalFrame {
                regs: [0x55; 32],
                return_pc: 0x9000,
                previous_mask: 0xaa,
                user_sp: 0xa000,
                previous_stack_flags: LINUX_SS_ONSTACK,
                restart: Some(restart),
            }),
            Some(0)
        );
        assert!(state.request_sigreturn());
        state.signal_wait = Some(LinuxSignalWait::suspend(linux_signal_bit(14), 0xbb));
        state.restart_block = Some(restart);
        state.suspend_restore_mask = Some(0xcc);

        state.reset_in_place();
        let expected_state = LinuxTaskSignalState::new();
        assert_eq!(state.mask, expected_state.mask);
        assert_eq!(state.pending.standard_pending, 0);
        assert_eq!(state.pending.standard_reserved, 0);
        assert_eq!(
            state.pending.standard_records,
            expected_state.pending.standard_records
        );
        assert_eq!(
            state.pending.realtime_pending,
            expected_state.pending.realtime_pending
        );
        assert_eq!(
            state.pending.realtime_sequences,
            expected_state.pending.realtime_sequences
        );
        assert_eq!(state.pending.realtime_len, 0);
        assert_eq!(
            state.pending.realtime_reservations,
            expected_state.pending.realtime_reservations
        );
        assert_eq!(state.pending.realtime_reserved, 0);
        assert_eq!(
            state.pending.next_realtime_sequence,
            expected_state.pending.next_realtime_sequence
        );
        assert_eq!(state.alt_stack, expected_state.alt_stack);
        assert_eq!(state.frames, expected_state.frames);
        assert_eq!(state.frame_depth, expected_state.frame_depth);
        assert_eq!(
            state.sigreturn_requested,
            expected_state.sigreturn_requested
        );
        assert_eq!(state.signal_wait, expected_state.signal_wait);
        assert_eq!(state.restart_block, expected_state.restart_block);
        assert_eq!(
            state.suspend_restore_mask,
            expected_state.suspend_restore_mask
        );
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
        let first = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        let second = tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap();
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
    fn process_and_directed_signal_routing_select_only_the_addressed_live_tgid() {
        let mut tasks = LinuxTaskTable::<4>::new();
        tasks.register_root(7).unwrap();
        let first = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        let competing = tasks.reserve_child(42, 9).unwrap();
        let second = tasks.reserve_child(LINUX_ROOT_TID, 10).unwrap();
        assert!(tasks.publish(first));
        assert!(tasks.publish(competing));
        assert!(tasks.publish(second));
        let signal = 12usize;
        let bit = linux_signal_bit(signal);
        tasks.signal_state_mut(LINUX_ROOT_TID, 7).unwrap().mask = bit;
        tasks.signal_state_mut(first.tid, 8).unwrap().mask = bit;

        assert_eq!(
            scoped_process_signal_target(&tasks, LINUX_ROOT_TID, signal),
            Some(LinuxTaskCore {
                tid: second.tid,
                tgid: LINUX_ROOT_TID,
                scheduler_thread: 10,
                state: LinuxTaskState::Runnable,
                block_reason: LinuxBlockReason::None,
            })
        );
        tasks.signal_state_mut(second.tid, 10).unwrap().mask = bit;
        assert_eq!(
            scoped_process_signal_target(&tasks, LINUX_ROOT_TID, signal),
            None,
            "an unmasked task in another TGID must never receive the signal"
        );
        tasks.signal_state_mut(first.tid, 8).unwrap().mask = 0;
        assert_eq!(
            scoped_process_signal_target(&tasks, LINUX_ROOT_TID, signal),
            Some(target_for(first))
        );
        tasks.signal_state_mut(first.tid, 8).unwrap().mask = bit;
        tasks.signal_state_mut(second.tid, 10).unwrap().mask = 0;

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
            tasks.signal_state(second.tid, 10).unwrap().realtime_pending[0],
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
    fn task_pending_peek_preserves_masked_and_unmasked_records() {
        let mut state = LinuxTaskSignalState::new();
        let record = signal_record(6, 0x5a);
        let bit = linux_signal_bit(record.signum);
        state.queue(record).unwrap();

        state.mask = bit;
        assert_eq!(state.peek_unblocked(), None);
        assert_eq!(state.pending_mask(), bit);

        state.mask = 0;
        assert_eq!(state.peek_unblocked(), Some(record));
        assert_eq!(state.peek_unblocked(), Some(record));
        assert_eq!(state.pending_mask(), bit);
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
    fn linux_sleep_deadlines_round_up_and_remaining_time_never_goes_negative() {
        const TICK_NANOS: u64 = 10_000_000;

        assert_eq!(
            linux_sleep_relative_deadline_ticks(40, 0, 0, TICK_NANOS),
            Some(40)
        );
        assert_eq!(
            linux_sleep_relative_deadline_ticks(40, 0, 1, TICK_NANOS),
            Some(42)
        );
        assert_eq!(
            linux_sleep_relative_deadline_ticks(40, 1, 0, TICK_NANOS),
            Some(141)
        );
        assert_eq!(
            linux_sleep_absolute_deadline_ticks(0, 1, TICK_NANOS),
            Some(1)
        );
        assert_eq!(
            linux_sleep_absolute_deadline_ticks(1, 0, TICK_NANOS),
            Some(100)
        );
        assert_eq!(
            linux_sleep_relative_deadline_ticks(0, 18_446_744_073, 709_551_615, TICK_NANOS,),
            Some(1_844_674_407_372)
        );
        assert_eq!(
            linux_sleep_absolute_deadline_ticks(18_446_744_073, 709_551_615, TICK_NANOS,),
            Some(1_844_674_407_371)
        );
        assert_eq!(
            linux_sleep_timespec_nanoseconds(18_446_744_073, 709_551_615),
            Some(u64::MAX)
        );
        assert_eq!(
            linux_sleep_remaining_timespec(40, 1, 40, TICK_NANOS),
            Some((0, 1))
        );
        assert_eq!(
            linux_sleep_remaining_timespec(40, 1, 41, TICK_NANOS),
            Some((0, 0))
        );
        assert_eq!(
            linux_sleep_remaining_timespec(40, TICK_NANOS, 40, TICK_NANOS),
            Some((0, 10_000_000))
        );
        assert_eq!(
            linux_sleep_remaining_timespec(40, TICK_NANOS, 41, TICK_NANOS),
            Some((0, 0))
        );
        assert_eq!(
            linux_sleep_remaining_timespec(0, 1_000_000_000, 40, TICK_NANOS),
            Some((0, 600_000_000))
        );
        assert_eq!(
            linux_sleep_remaining_timespec(0, u64::MAX, 0, TICK_NANOS),
            Some((18_446_744_073, 709_551_615))
        );
        assert_eq!(
            linux_sleep_relative_deadline_ticks(0, -1, 0, TICK_NANOS),
            None
        );
        assert_eq!(
            linux_sleep_absolute_deadline_ticks(0, 1_000_000_000, TICK_NANOS),
            None
        );
        assert_eq!(linux_sleep_timespec_nanoseconds(0, 1_000_000_000), None);
        assert_eq!(linux_sleep_remaining_timespec(0, 1, 0, 0), None);
    }

    #[test]
    fn linux_sleep_waits_expire_or_interrupt_once_and_reset_with_their_task() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        assert!(tasks.publish(child));

        assert!(!tasks.install_sleep(
            child.tid,
            8,
            LinuxSleepWait {
                deadline: 40,
                outcome: LinuxSleepOutcome::Completed,
                relative: None,
            },
        ));
        assert!(!tasks.install_sleep(
            child.tid,
            8,
            LinuxSleepWait {
                deadline: 40,
                outcome: LinuxSleepOutcome::Interrupted,
                relative: None,
            },
        ));
        assert_eq!(LinuxSleepWait::waiting(45).relative, None);
        let completed_wait = LinuxSleepWait::relative_waiting(50, 40, 1);
        assert_eq!(
            completed_wait.relative,
            Some(LinuxSleepRelative {
                started_at: 40,
                requested_nanoseconds: 1,
            })
        );
        assert!(tasks.install_sleep(child.tid, 8, completed_wait));
        assert!(!tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(60)));
        assert!(tasks.block(child.tid, 8, LinuxBlockReason::Sleep));
        assert_eq!(tasks.expire_sleeps(49), [None, None, None]);
        let expired = tasks.expire_sleeps(50);
        assert_eq!(expired[0], Some((child.tid, 8, LinuxBlockReason::Sleep)));
        assert!(expired[1..].iter().all(Option::is_none));
        assert_eq!(tasks.expire_sleeps(51), [None, None, None]);
        assert!(tasks.wake(child.tid, 8));
        assert_eq!(
            tasks.take_sleep_outcome(child.tid, 8),
            Some(LinuxSleepWait {
                deadline: 50,
                outcome: LinuxSleepOutcome::Completed,
                relative: completed_wait.relative,
            })
        );
        assert_eq!(tasks.take_sleep_outcome(child.tid, 8), None);

        assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(70)));
        assert!(!tasks.cancel_sleep(child.tid + 1, 8));
        assert!(!tasks.cancel_sleep(child.tid, 9));
        assert!(tasks.cancel_sleep(child.tid, 8));
        assert!(!tasks.cancel_sleep(child.tid, 8));

        assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(75)));
        assert!(tasks.block(child.tid, 8, LinuxBlockReason::Sleep));
        assert_eq!(
            tasks.expire_sleeps(75)[0],
            Some((child.tid, 8, LinuxBlockReason::Sleep))
        );
        assert!(tasks.wake(child.tid, 8));
        assert!(tasks.cancel_sleep(child.tid, 8));
        assert!(!tasks.cancel_sleep(child.tid, 8));
        assert_eq!(tasks.take_sleep_outcome(child.tid, 8), None);

        let interrupted_wait = LinuxSleepWait::relative_waiting(80, 70, 10_000_000);
        assert!(tasks.install_sleep(child.tid, 8, interrupted_wait));
        assert!(tasks.block(child.tid, 8, LinuxBlockReason::Sleep));
        tasks.signal_state_mut(child.tid, 8).unwrap().mask = linux_signal_bit(6);
        assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
        tasks.signal_state_mut(child.tid, 8).unwrap().mask = 0;
        assert!(tasks.interrupt_sleep(child.tid, 8, 6));
        assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
        assert!(tasks.wake(child.tid, 8));
        assert_eq!(
            tasks.take_sleep_outcome(child.tid, 8),
            Some(LinuxSleepWait {
                deadline: 80,
                outcome: LinuxSleepOutcome::Interrupted,
                relative: interrupted_wait.relative,
            })
        );

        assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(90)));
        assert!(tasks.exit(child.tid, 8));
        assert!(tasks.retire(child.tid, 8));
        assert_eq!(tasks.take_sleep_outcome(child.tid, 8), None);
        let replacement = tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap();
        assert!(tasks.publish(replacement));
        assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
        assert_eq!(tasks.expire_sleeps(90), [None, None, None]);

        let rollback = tasks.reserve_child(LINUX_ROOT_TID, 10).unwrap();
        tasks.sleep_waits[rollback.slot] = Some(LinuxSleepWait::waiting(95));
        assert!(tasks.rollback(rollback));
        assert_eq!(tasks.sleep_waits[rollback.slot], None);

        tasks.reset();
        assert_eq!(tasks.expire_sleeps(u64::MAX), [None, None, None]);
    }

    #[test]
    fn signal_wait_zero_deadlines_expire_and_report_eagain_outcomes() {
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 1, 10_000_000),
            Some(42)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 10_000_001, 10_000_000),
            Some(43)
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
    fn positive_signal_wait_deadlines_include_the_current_tick_phase() {
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 0, 10_000_000),
            Some(40)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 1, 10_000_000),
            Some(42)
        );
        assert_eq!(
            linux_signal_timespec_to_ticks_ceil(40, 0, 10_000_000, 10_000_000),
            Some(42)
        );
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

    #[cfg(test)]
    fn assert_realtime_wait_efault_rollback_is_reserved_and_exact() {
        let mut pending = LinuxPendingSignals::new();
        for marker in 0..LINUX_RT_QUEUE_LIMIT - 1 {
            pending.queue(signal_record(34, marker as u8)).unwrap();
        }
        let matching = signal_record(35, 0xa5);
        let reservation = pending
            .reserve_direct(matching)
            .unwrap()
            .expect("direct wait completion reservation");

        assert_eq!(pending.realtime_reserved, 1);
        assert_eq!(
            pending.queue(signal_record(36, 0xff)),
            Err(LinuxSignalRouteError::QueueFull)
        );

        pending
            .rollback_reservation(reservation, matching)
            .expect("guaranteed EFAULT rollback");
        assert_eq!(pending.realtime_reserved, 0);
        assert_eq!(pending.realtime_len, LINUX_RT_QUEUE_LIMIT);
        assert_eq!(pending.take_matching(linux_signal_bit(35)), Some(matching));
    }

    #[test]
    fn task_realtime_wait_efault_rollback_restores_the_exact_reserved_record() {
        assert_realtime_wait_efault_rollback_is_reserved_and_exact();
    }

    #[test]
    fn process_realtime_wait_efault_rollback_restores_the_exact_reserved_record() {
        assert_realtime_wait_efault_rollback_is_reserved_and_exact();
    }

    #[test]
    fn pending_realtime_wait_rollback_restores_the_reserved_record_at_capacity() {
        let mut pending = LinuxPendingSignals::new();
        let original = signal_record(35, 0xa5);
        pending.queue(original).unwrap();
        let (taken, reservation) = pending
            .take_matching_reserved(linux_signal_bit(35))
            .expect("reserved destructive wait take");
        assert_eq!(taken, original);
        assert_eq!(pending.realtime_reserved, 1);

        for marker in 0..LINUX_RT_QUEUE_LIMIT - 1 {
            pending.queue(signal_record(36, marker as u8)).unwrap();
        }
        assert_eq!(pending.realtime_len, LINUX_RT_QUEUE_LIMIT - 1);
        assert_eq!(
            pending.queue(signal_record(37, 0xff)),
            Err(LinuxSignalRouteError::QueueFull)
        );

        pending
            .rollback_reservation(reservation, taken)
            .expect("guaranteed destructive-take rollback");
        assert_eq!(pending.realtime_reserved, 0);
        assert_eq!(pending.realtime_len, LINUX_RT_QUEUE_LIMIT);
        assert_eq!(pending.take_matching(linux_signal_bit(35)), Some(original));
    }

    #[cfg(test)]
    fn assert_realtime_reservation_rollback_preserves_fifo(rollback_first_first: bool) {
        let mut pending = LinuxPendingSignals::new();
        let first = signal_record(35, 0x11);
        let second = signal_record(35, 0x22);
        let later = signal_record(35, 0x33);
        pending.queue(first).unwrap();
        pending.queue(second).unwrap();
        let (first_record, first_reservation) = pending
            .take_matching_reserved(linux_signal_bit(35))
            .expect("first realtime reservation");
        let (second_record, second_reservation) = pending
            .take_matching_reserved(linux_signal_bit(35))
            .expect("second realtime reservation");
        pending.queue(later).unwrap();

        if rollback_first_first {
            pending
                .rollback_reservation(first_reservation, first_record)
                .unwrap();
            pending
                .rollback_reservation(second_reservation, second_record)
                .unwrap();
        } else {
            pending
                .rollback_reservation(second_reservation, second_record)
                .unwrap();
            pending
                .rollback_reservation(first_reservation, first_record)
                .unwrap();
        }

        assert_eq!(pending.realtime_reserved, 0);
        assert_eq!(pending.take_matching(linux_signal_bit(35)), Some(first));
        assert_eq!(pending.take_matching(linux_signal_bit(35)), Some(second));
        assert_eq!(pending.take_matching(linux_signal_bit(35)), Some(later));
    }

    #[test]
    fn realtime_reservations_rollback_in_fifo_order_when_completed_in_assignment_order() {
        assert_realtime_reservation_rollback_preserves_fifo(true);
    }

    #[test]
    fn realtime_reservations_rollback_in_fifo_order_when_completed_in_reverse_order() {
        assert_realtime_reservation_rollback_preserves_fifo(false);
    }

    #[test]
    fn standard_wait_rollback_restores_the_original_record_over_later_coalescing() {
        let mut pending = LinuxPendingSignals::new();
        let original = signal_record(10, 0x31);
        let later = signal_record(10, 0x92);
        let duplicate = signal_record(10, 0xb4);
        let reservation = pending
            .reserve_direct(original)
            .unwrap()
            .expect("standard wait reservation");

        pending.queue(later).unwrap();
        pending.queue(duplicate).unwrap();
        assert_eq!(pending.standard_reserved, linux_signal_bit(10));
        assert_eq!(pending.standard_records[10], later);

        pending
            .rollback_reservation(reservation, original)
            .expect("standard EFAULT rollback");
        assert_eq!(pending.standard_reserved, 0);
        assert_eq!(pending.take_matching(linux_signal_bit(10)), Some(original));
    }

    #[test]
    fn process_standard_commit_hands_the_later_record_to_the_next_waiter() {
        let (mut tasks, first, second) = three_live_tasks();
        let signum = 10;
        let bit = linux_signal_bit(signum);
        for (reservation, output_address) in [(first, 0x6100), (second, 0x6200)] {
            assert!(tasks
                .signal_state_mut(reservation.tid, reservation.scheduler_thread)
                .unwrap()
                .install_signal_wait(LinuxSignalWait::timed(bit, None, output_address,)));
            assert!(tasks.block(
                reservation.tid,
                reservation.scheduler_thread,
                LinuxBlockReason::SignalWait,
            ));
        }

        let mut pending = LinuxPendingSignals::new();
        let original = signal_record(signum, 0x31);
        let later = signal_record(signum, 0x92);
        let reservation = pending
            .reserve_direct(original)
            .unwrap()
            .expect("first process waiter reservation");
        assert_eq!(
            tasks.complete_process_signal_wait(
                first.tid,
                first.scheduler_thread,
                original,
                reservation,
            ),
            Some(LinuxBlockReason::SignalWait)
        );
        assert!(tasks.wake(first.tid, first.scheduler_thread));
        assert_eq!(pending.reserve_direct(later).unwrap(), None);
        assert_eq!(pending.standard_records[signum], later);

        let first_wait = tasks
            .signal_state_mut(first.tid, first.scheduler_thread)
            .unwrap()
            .take_signal_wait_outcome()
            .expect("first process waiter outcome");
        assert_eq!(first_wait.signal, original);
        assert_eq!(
            first_wait.signal_source,
            Some(LinuxPendingSignalSource::Process)
        );
        pending
            .commit_reservation(first_wait.signal_reservation.unwrap())
            .unwrap();

        let (target, reason) = tasks
            .handoff_process_pending_signal(&mut pending)
            .unwrap()
            .expect("newly visible process signal handoff");
        assert_eq!(target.tid, second.tid);
        assert_eq!(reason, LinuxBlockReason::SignalWait);
        assert!(tasks.wake(target.tid, target.scheduler_thread));

        let second_wait = tasks
            .signal_state_mut(second.tid, second.scheduler_thread)
            .unwrap()
            .take_signal_wait_outcome()
            .expect("second process waiter outcome");
        assert_eq!(second_wait.signal, later);
        assert_eq!(
            second_wait.signal_source,
            Some(LinuxPendingSignalSource::Process)
        );
        assert_eq!(pending.standard_pending, 0);
        assert_eq!(pending.standard_reserved, bit);
        assert_eq!(
            tasks.handoff_process_pending_signal(&mut pending).unwrap(),
            None,
            "the later standard occurrence is handed off exactly once"
        );
        pending
            .commit_reservation(second_wait.signal_reservation.unwrap())
            .unwrap();
        assert_eq!(pending.standard_reserved, 0);
        assert_eq!(pending.take_matching(bit), None);
    }

    #[test]
    fn process_standard_rollback_hands_the_exact_original_to_the_next_waiter() {
        let (mut tasks, first, second) = three_live_tasks();
        let signum = 10;
        let bit = linux_signal_bit(signum);
        for (reservation, output_address) in [(first, 0x6300), (second, 0x6400)] {
            assert!(tasks
                .signal_state_mut(reservation.tid, reservation.scheduler_thread)
                .unwrap()
                .install_signal_wait(LinuxSignalWait::timed(bit, None, output_address,)));
            assert!(tasks.block(
                reservation.tid,
                reservation.scheduler_thread,
                LinuxBlockReason::SignalWait,
            ));
        }

        let mut pending = LinuxPendingSignals::new();
        let original = signal_record(signum, 0x31);
        let later = signal_record(signum, 0x92);
        let reservation = pending
            .reserve_direct(original)
            .unwrap()
            .expect("first process waiter reservation");
        assert_eq!(
            tasks.complete_process_signal_wait(
                first.tid,
                first.scheduler_thread,
                original,
                reservation,
            ),
            Some(LinuxBlockReason::SignalWait)
        );
        assert!(tasks.wake(first.tid, first.scheduler_thread));
        assert_eq!(pending.reserve_direct(later).unwrap(), None);

        let first_wait = tasks
            .signal_state_mut(first.tid, first.scheduler_thread)
            .unwrap()
            .take_signal_wait_outcome()
            .expect("first process waiter outcome");
        pending
            .rollback_reservation(first_wait.signal_reservation.unwrap(), first_wait.signal)
            .unwrap();
        assert_eq!(pending.standard_records[signum], original);

        let (target, reason) = tasks
            .handoff_process_pending_signal(&mut pending)
            .unwrap()
            .expect("rolled-back process signal handoff");
        assert_eq!(target.tid, second.tid);
        assert_eq!(reason, LinuxBlockReason::SignalWait);
        assert!(tasks.wake(target.tid, target.scheduler_thread));

        let second_wait = tasks
            .signal_state_mut(second.tid, second.scheduler_thread)
            .unwrap()
            .take_signal_wait_outcome()
            .expect("second process waiter outcome");
        assert_eq!(second_wait.signal, original);
        assert_eq!(second_wait.signal.info, [0x31; LINUX_SIGNAL_INFO_BYTES]);
        assert_eq!(pending.standard_pending, 0);
        assert_eq!(pending.standard_reserved, bit);
        assert_eq!(
            tasks.handoff_process_pending_signal(&mut pending).unwrap(),
            None,
            "rollback must not duplicate the original occurrence"
        );
        pending
            .commit_reservation(second_wait.signal_reservation.unwrap())
            .unwrap();
        assert_eq!(pending.standard_reserved, 0);
        assert_eq!(pending.take_matching(bit), None);
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
    fn sigreturn_stages_restart_until_the_next_handler_disposition_is_known() {
        let restart = LinuxRestartBlock {
            syscall_number: 98,
            arguments: [0x10, 0x20, 0x30, 0x40, 0x50, 0x60],
            svc_address: 0x7ffc,
            timeout: LinuxRestartTimeout::Infinite,
        };
        let frame = LinuxSignalFrame {
            regs: [0xaa; 32],
            return_pc: 0x8000,
            previous_mask: 0x55,
            user_sp: 0x9000,
            previous_stack_flags: 0,
            restart: Some(restart),
        };
        let mut state = LinuxTaskSignalState::new();
        state.push_frame(frame).unwrap();
        assert!(state.request_sigreturn());

        let restored = state
            .take_requested_frame()
            .expect("requested signal frame");
        assert_eq!(restored.regs, frame.regs);
        assert_eq!(restored.return_pc, frame.return_pc);
        assert_eq!(state.restart_block, Some(restart));
        assert_eq!(state.take_restart_for_signal(false), None);
        assert!(state.restart_block.is_none());
    }

    #[test]
    fn signal_state_is_cleared_on_rollback_retire_and_reset() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();

        let rolled_back = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        tasks.signal_states[rolled_back.slot].mask = 0x55;
        tasks.signal_states[rolled_back.slot]
            .queue(signal_record(34, 0x34))
            .unwrap();
        tasks.signal_states[rolled_back.slot]
            .pending
            .reserve_direct(signal_record(35, 0x35))
            .unwrap()
            .unwrap();
        tasks.signal_states[rolled_back.slot]
            .pending
            .reserve_direct(signal_record(12, 0x12))
            .unwrap()
            .unwrap();
        assert!(tasks.rollback(rolled_back));
        assert_eq!(tasks.signal_states[rolled_back.slot].mask, 0);
        assert_eq!(tasks.signal_states[rolled_back.slot].realtime_len, 0);
        assert_eq!(tasks.signal_states[rolled_back.slot].standard_reserved, 0);
        assert_eq!(tasks.signal_states[rolled_back.slot].realtime_reserved, 0);

        let retired = tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap();
        assert_eq!(retired.slot, rolled_back.slot);
        assert!(tasks.publish(retired));
        let retired_state = tasks.signal_state_mut(retired.tid, 9).unwrap();
        retired_state.mask = 0xaa;
        retired_state
            .pending
            .reserve_direct(signal_record(35, 0x35))
            .unwrap()
            .unwrap();
        retired_state
            .pending
            .reserve_direct(signal_record(12, 0x12))
            .unwrap()
            .unwrap();
        assert!(tasks.exit(retired.tid, 9));
        assert!(tasks.retire(retired.tid, 9));
        assert_eq!(tasks.signal_states[retired.slot].mask, 0);
        assert_eq!(tasks.signal_states[retired.slot].standard_reserved, 0);
        assert_eq!(tasks.signal_states[retired.slot].realtime_reserved, 0);

        let root_state = tasks.signal_state_mut(LINUX_ROOT_TID, 7).unwrap();
        root_state.queue(signal_record(12, 0x12)).unwrap();
        root_state
            .pending
            .reserve_direct(signal_record(35, 0x35))
            .unwrap()
            .unwrap();
        root_state
            .pending
            .reserve_direct(signal_record(13, 0x13))
            .unwrap()
            .unwrap();
        tasks.reset();
        assert!(tasks.signal_states.iter().all(|state| {
            state.mask == 0
                && state.pending_mask() == 0
                && state.standard_reserved == 0
                && state.realtime_reserved == 0
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

        let stale = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        assert!(tasks.rollback(stale));
        let child = tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap();
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

    #[test]
    fn thread_reservation_retains_the_current_process_tgid() {
        let mut tasks = LinuxTaskTable::<2>::new();
        tasks.register_root(7).unwrap();

        let child = tasks.reserve_child(42, 8).unwrap();
        assert!(tasks.publish(child));
        assert_eq!(tasks.by_tid(child.tid).map(|task| task.tgid), Some(42));
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

mod linux_process_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_process_logic_shared.rs"
    ));

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum ObjectType {
        LinuxFile,
        LinuxPipe,
        MessageQueue,
        SharedMemory,
    }

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_process_memory_logic_shared.rs"
    ));

    #[test]
    fn executable_linux_signal_lifecycle_behavior_contract() {
        crate::linux_signal_lifecycle_behavior_contract();
    }

    #[test]
    fn root_registration_is_unique_and_published_by_pid_and_scheduler() {
        let mut processes = LinuxProcessTable::<2>::new();

        assert_eq!(processes.register_root(7), Ok(LINUX_ROOT_PID));
        assert_eq!(
            processes.register_root(8),
            Err(LinuxProcessError::DuplicateRoot)
        );
        assert_eq!(
            processes.by_pid(LINUX_ROOT_PID),
            Some(LinuxProcessCore {
                pid: LINUX_ROOT_PID,
                parent_pid: 0,
                process_group: LINUX_ROOT_PID,
                root_scheduler_thread: 7,
                state: LinuxProcessState::Running,
                wait_status: 0,
                exit_signal: 0,
            })
        );
        assert_eq!(processes.by_scheduler(7), processes.by_pid(LINUX_ROOT_PID));

        let mut empty = LinuxProcessTable::<0>::new();
        assert_eq!(empty.register_root(7), Err(LinuxProcessError::Capacity));
    }

    #[test]
    fn reservations_are_hidden_and_publish_or_rollback_atomically() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();

        let first = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert_eq!(first.pid, 2);
        assert_eq!(first.parent_pid, LINUX_ROOT_PID);
        assert_eq!(processes.by_pid(first.pid), None);
        assert_eq!(processes.by_scheduler(8), None);
        assert!(!processes.has_matching_child(LINUX_ROOT_PID, LinuxWaitSelector::Pid(first.pid)));
        assert!(processes.publish(first));
        assert_eq!(
            processes.by_scheduler(8).map(|process| process.pid),
            Some(2)
        );

        let rolled_back = processes.reserve_child(LINUX_ROOT_PID, 9).unwrap();
        assert_eq!(rolled_back.pid, 3);
        assert!(processes.rollback(rolled_back));
        assert!(!processes.publish(rolled_back));
        let replacement = processes.reserve_child(LINUX_ROOT_PID, 10).unwrap();
        assert_eq!(replacement.slot, rolled_back.slot);
        assert_eq!(replacement.pid, 4, "rolled-back PIDs must not be reused");
    }

    #[test]
    fn children_inherit_parent_and_process_group_relationships() {
        let mut processes = LinuxProcessTable::<4>::new();
        processes.register_root(7).unwrap();
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(child));
        let grandchild = processes.reserve_child(child.pid, 9).unwrap();
        assert!(processes.publish(grandchild));

        assert_eq!(
            processes
                .by_pid(child.pid)
                .map(|process| (process.parent_pid, process.process_group,)),
            Some((LINUX_ROOT_PID, LINUX_ROOT_PID))
        );
        assert_eq!(
            processes.by_scheduler(9).map(|process| (
                process.pid,
                process.parent_pid,
                process.process_group,
            )),
            Some((grandchild.pid, child.pid, LINUX_ROOT_PID))
        );
        assert_eq!(
            processes.reserve_child(999, 10),
            Err(LinuxProcessError::NoSuchParent)
        );
    }

    #[test]
    fn wait_status_encoding_matches_posix_layout() {
        for (code, expected) in [(0, 0), (1, 0x100), (255, 0xff00), (256, 0), (-1, 0xff00)] {
            assert_eq!(linux_wait_status_exit(code), expected);
        }
        assert_eq!(linux_wait_status_signal(1, false), Some(1));
        assert_eq!(linux_wait_status_signal(127, true), Some(0xff));
        assert_eq!(linux_wait_status_signal(0, false), None);
        assert_eq!(linux_wait_status_signal(128, false), None);
    }

    #[test]
    fn signal_termination_status_uses_low_seven_bits_without_a_spurious_core_bit() {
        for signum in [1usize, 9, 15, 31, 64, 127] {
            let status = linux_wait_status_signal(signum, false).expect("valid signal status");
            assert_eq!(status & 0x7f, signum as i32);
            assert_eq!(status & 0x80, 0, "no core file was created");
        }
    }

    #[test]
    fn sigchld_policy_matches_ignored_no_child_wait_and_zombie_rules() {
        assert_eq!(
            linux_sigchld_exit_policy(false, false),
            LinuxSigchldExitPolicy::RetainZombieAndNotify
        );
        assert_eq!(
            linux_sigchld_exit_policy(true, false),
            LinuxSigchldExitPolicy::ReapWithoutNotify
        );
        assert_eq!(
            linux_sigchld_exit_policy(false, true),
            LinuxSigchldExitPolicy::ReapAndNotify
        );
        assert_eq!(
            linux_sigchld_exit_policy(true, true),
            LinuxSigchldExitPolicy::ReapWithoutNotify,
            "ignored SIGCHLD takes precedence over SA_NOCLDWAIT"
        );
    }

    #[test]
    fn process_exit_signal_routes_exact_notification() {
        assert_eq!(linux_child_exit_notification(0), None);
        assert_eq!(linux_child_exit_notification(17), Some(17));

        for (exit_signal, policy, expected_notification) in [
            (0, LinuxSigchldExitPolicy::ReapWithoutNotify, None),
            (17, LinuxSigchldExitPolicy::RetainZombieAndNotify, Some(17)),
            (12, LinuxSigchldExitPolicy::ReapWithoutNotify, Some(12)),
        ] {
            let mut processes = LinuxProcessTable::<2>::new();
            processes.register_root(7).unwrap();
            let child = processes
                .reserve_child_with_pid(LINUX_ROOT_PID, 8, 2, exit_signal)
                .unwrap();
            assert!(processes.publish(child));

            let transition = processes
                .terminate_child(child.pid, 15, policy)
                .expect("running child terminal transition");

            assert_eq!(transition.notification_signal, expected_notification);
            let mut delivered_signal = None;
            let mut waiter_wakes = 0usize;
            apply_linux_terminal_child_transition(
                transition,
                |parent_pid, signum| {
                    assert_eq!(parent_pid, LINUX_ROOT_PID);
                    delivered_signal = Some(signum);
                    Ok::<(), ()>(())
                },
                |parent_pid| {
                    assert_eq!(parent_pid, LINUX_ROOT_PID);
                    waiter_wakes += 1;
                },
            )
            .unwrap();
            assert_eq!(delivered_signal, expected_notification);
            assert_eq!(waiter_wakes, 1);
            assert_eq!(
                processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Pid(child.pid)),
                LinuxWaitOutcome::Ready {
                    pid: child.pid,
                    status: 15,
                },
                "zero and custom exit signals do not inherit SIGCHLD auto-reap policy",
            );
        }
    }

    #[test]
    fn orphan_parent_identity_is_user_visible() {
        assert_eq!(linux_visible_parent_pid(LINUX_ROOT_PID, 0), 0);
        assert_eq!(
            linux_visible_parent_pid(42, LINUX_LAUNCH_REAPER_PID),
            LINUX_ROOT_PID,
        );
    }

    #[test]
    fn terminal_child_transition_reaps_immediately_or_retains_one_zombie() {
        for (policy, expected_wait, expected_notification) in [
            (
                LinuxSigchldExitPolicy::RetainZombieAndNotify,
                LinuxWaitOutcome::Ready { pid: 2, status: 15 },
                Some(17),
            ),
            (
                LinuxSigchldExitPolicy::ReapAndNotify,
                LinuxWaitOutcome::NoChildren,
                Some(17),
            ),
            (
                LinuxSigchldExitPolicy::ReapWithoutNotify,
                LinuxWaitOutcome::NoChildren,
                None,
            ),
        ] {
            let mut processes = LinuxProcessTable::<2>::new();
            processes.register_root(7).unwrap();
            let child = processes
                .reserve_child_with_pid(LINUX_ROOT_PID, 8, 2, LINUX_SIGCHLD)
                .unwrap();
            assert!(processes.publish(child));

            let terminal = processes
                .terminate_child(child.pid, 15, policy)
                .expect("running child terminal transition");

            assert_eq!(terminal.parent_pid, LINUX_ROOT_PID);
            assert_eq!(terminal.notification_signal, expected_notification);
            assert_eq!(
                processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Pid(child.pid)),
                expected_wait,
                "an immediate reap leaves a woken matching waiter with ECHILD"
            );
        }
    }

    #[test]
    fn terminal_child_transition_wakes_waiters_after_notification_failure() {
        let transition = LinuxTerminalChildTransition {
            parent_pid: LINUX_ROOT_PID,
            notification_signal: Some(17),
        };
        let mut notification_attempts = 0usize;
        let mut waiter_wakes = 0usize;

        let result = apply_linux_terminal_child_transition(
            transition,
            |parent_pid, signum| {
                assert_eq!(parent_pid, LINUX_ROOT_PID);
                assert_eq!(signum, 17);
                notification_attempts += 1;
                Err(17usize)
            },
            |parent_pid| {
                assert_eq!(parent_pid, LINUX_ROOT_PID);
                waiter_wakes += 1;
            },
        );

        assert_eq!(result, Err(17));
        assert_eq!(notification_attempts, 1);
        assert_eq!(waiter_wakes, 1);
    }

    #[test]
    fn wait_selection_covers_exact_any_group_and_wnohang_states() {
        let mut processes = LinuxProcessTable::<4>::new();
        processes.register_root(7).unwrap();
        let first = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        let second = processes.reserve_child(LINUX_ROOT_PID, 9).unwrap();
        assert!(processes.publish(first));
        assert!(processes.publish(second));

        assert_eq!(
            processes.select_waitable(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            None
        );
        assert!(
            processes.has_matching_child(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            "WNOHANG returns zero only while a matching live child exists"
        );
        assert!(!processes.has_matching_child(LINUX_ROOT_PID, LinuxWaitSelector::Pid(999)));

        let second_status = linux_wait_status_exit(23);
        assert!(processes.exit(second.pid, second_status));
        assert_eq!(
            processes
                .select_waitable(LINUX_ROOT_PID, LinuxWaitSelector::Pid(second.pid))
                .map(|process| (process.pid, process.wait_status)),
            Some((second.pid, second_status))
        );
        assert_eq!(
            processes
                .select_waitable(LINUX_ROOT_PID, LinuxWaitSelector::Any)
                .map(|process| process.pid),
            Some(second.pid)
        );
        assert_eq!(
            processes
                .select_waitable(
                    LINUX_ROOT_PID,
                    LinuxWaitSelector::ProcessGroup(LINUX_ROOT_PID),
                )
                .map(|process| process.pid),
            Some(second.pid)
        );
        assert_eq!(
            processes.select_waitable(LINUX_ROOT_PID, LinuxWaitSelector::ProcessGroup(99)),
            None
        );
    }

    #[test]
    fn wait_outcomes_distinguish_ready_live_and_absent_children_without_reaping() {
        let mut processes = LinuxProcessTable::<4>::new();
        processes.register_root(7).unwrap();
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(child));

        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::WouldBlock
        );
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Pid(999)),
            LinuxWaitOutcome::NoChildren
        );

        let status = linux_wait_status_exit(37);
        assert!(processes.exit(child.pid, status));
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::Ready {
                pid: child.pid,
                status,
            }
        );
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::Ready {
                pid: child.pid,
                status,
            },
            "selecting a ready child must not reap before status copyout"
        );
        assert!(processes.reap(LINUX_ROOT_PID, child.pid).is_some());
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::NoChildren
        );
    }

    #[test]
    fn wait_completion_copies_before_reaping_and_preserves_zombie_on_fault() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(child));
        let status = linux_wait_status_exit(37);
        assert!(processes.exit(child.pid, status));

        let selected = processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any);
        let LinuxWaitOutcome::Ready { pid, status } = selected else {
            panic!("child must be waitable");
        };
        let failed = complete_linux_wait(
            &mut processes,
            LINUX_ROOT_PID,
            LinuxWaitSelector::Any,
            pid,
            status,
            |_status| Err(14u8),
        );
        assert_eq!(failed, Err(LinuxWaitCompletionError::Copy(14)));
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            selected
        );

        let mut copied_status = None;
        let completed = complete_linux_wait(
            &mut processes,
            LINUX_ROOT_PID,
            LinuxWaitSelector::Any,
            pid,
            status,
            |status| {
                copied_status = Some(status);
                Ok::<(), u8>(())
            },
        );
        assert_eq!(completed, Ok(Some(pid)));
        assert_eq!(copied_status, Some(status));
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::NoChildren
        );

        let repeated = complete_linux_wait(
            &mut processes,
            LINUX_ROOT_PID,
            LinuxWaitSelector::Any,
            pid,
            status,
            |_status| Ok::<(), u8>(()),
        );
        assert_eq!(repeated, Ok(None));
    }

    #[test]
    fn concurrent_waiters_serialize_stale_selection_copyout_and_reap() {
        use std::sync::{Arc, Barrier, Mutex};

        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let first = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        let second = processes.reserve_child(LINUX_ROOT_PID, 9).unwrap();
        assert!(processes.publish(first));
        assert!(processes.publish(second));
        assert!(processes.exit(first.pid, linux_wait_status_exit(31)));
        assert!(processes.exit(second.pid, linux_wait_status_exit(32)));

        let stale = processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any);
        let runtime = Arc::new(Mutex::new(processes));
        let start = Arc::new(Barrier::new(2));
        let mut waiters = Vec::new();
        for _ in 0..2 {
            let runtime = Arc::clone(&runtime);
            let start = Arc::clone(&start);
            waiters.push(std::thread::spawn(move || {
                let mut observed = stale;
                start.wait();
                loop {
                    let LinuxWaitOutcome::Ready { pid, status } = observed else {
                        panic!("a distinct zombie must remain for each waiter");
                    };
                    let mut processes = runtime.lock().expect("process runtime lock");
                    match complete_linux_wait(
                        &mut processes,
                        LINUX_ROOT_PID,
                        LinuxWaitSelector::Any,
                        pid,
                        status,
                        |_status| Ok::<(), u8>(()),
                    ) {
                        Ok(Some(reaped_pid)) => return reaped_pid,
                        Ok(None) => {
                            observed =
                                processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any);
                        }
                        Err(error) => panic!("wait completion failed: {error:?}"),
                    }
                }
            }));
        }

        let mut reaped: Vec<_> = waiters
            .into_iter()
            .map(|waiter| waiter.join().expect("waiter thread"))
            .collect();
        reaped.sort_unstable();
        assert_eq!(reaped, vec![first.pid, second.pid]);
        assert_eq!(
            runtime
                .lock()
                .expect("process runtime lock")
                .wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Any),
            LinuxWaitOutcome::NoChildren
        );
    }

    #[test]
    fn wait_pid_parser_covers_posix_selectors_without_signed_overflow() {
        assert_eq!(linux_wait_selector(23, 7), Some(LinuxWaitSelector::Pid(23)));
        assert_eq!(linux_wait_selector(-1, 7), Some(LinuxWaitSelector::Any));
        assert_eq!(
            linux_wait_selector(0, 7),
            Some(LinuxWaitSelector::ProcessGroup(7))
        );
        assert_eq!(
            linux_wait_selector(-23, 7),
            Some(LinuxWaitSelector::ProcessGroup(23))
        );
        assert_eq!(
            linux_wait_selector(i32::MIN, 7),
            Some(LinuxWaitSelector::ProcessGroup(1usize << 31))
        );
    }

    #[test]
    fn wait_options_accept_posix_untraced_without_accepting_unknown_bits() {
        assert!(linux_wait_options_valid(0));
        assert!(linux_wait_options_valid(LINUX_WAIT_WNOHANG));
        assert!(linux_wait_options_valid(LINUX_WAIT_WUNTRACED));
        assert!(linux_wait_options_valid(
            LINUX_WAIT_WNOHANG | LINUX_WAIT_WUNTRACED
        ));
        assert!(!linux_wait_options_valid(1 << 2));
    }

    #[test]
    fn process_counts_track_live_and_zombie_lifecycle_exactly() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        assert_eq!(processes.resource_counts(), (1, 0));
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert_eq!(processes.resource_counts(), (1, 0));
        assert!(processes.publish(child));
        assert_eq!(processes.resource_counts(), (2, 0));
        assert!(processes.exit(child.pid, linux_wait_status_exit(9)));
        assert_eq!(processes.resource_counts(), (1, 1));
        assert!(processes.reap(LINUX_ROOT_PID, child.pid).is_some());
        assert_eq!(processes.resource_counts(), (1, 0));
    }

    #[test]
    fn process_memory_identity_match_rejects_partial_fork_and_exit_snapshots() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        assert!(processes.running_pids_match(&[LINUX_ROOT_PID]));

        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.running_pids_match(&[LINUX_ROOT_PID]));
        assert!(!processes.running_pids_match(&[LINUX_ROOT_PID, child.pid]));

        assert!(processes.publish(child));
        assert!(processes.running_pids_match(&[LINUX_ROOT_PID, child.pid]));
        assert!(!processes.running_pids_match(&[LINUX_ROOT_PID]));

        assert!(processes.exit(child.pid, linux_wait_status_exit(9)));
        assert!(processes.running_pids_match(&[LINUX_ROOT_PID]));
        assert!(!processes.running_pids_match(&[LINUX_ROOT_PID, child.pid]));
    }

    #[test]
    fn zombies_are_reaped_once_without_reusing_their_pid() {
        let mut processes = LinuxProcessTable::<2>::new();
        processes.register_root(7).unwrap();
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(child));
        let status = linux_wait_status_exit(9);
        assert!(processes.exit(child.pid, status));
        assert!(!processes.exit(child.pid, status));

        assert_eq!(
            processes
                .reap(LINUX_ROOT_PID, child.pid)
                .map(|process| process.wait_status),
            Some(status)
        );
        assert_eq!(processes.reap(LINUX_ROOT_PID, child.pid), None);
        assert_eq!(processes.by_pid(child.pid), None);
        let next = processes.reserve_child(LINUX_ROOT_PID, 9).unwrap();
        assert_eq!(next.pid, child.pid + 1);
    }

    #[test]
    fn published_descendants_reparent_to_the_launch_reaper() {
        let mut processes = LinuxProcessTable::<5>::new();
        processes.register_root(7).unwrap();
        let parent = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(parent));
        let running = processes.reserve_child(parent.pid, 9).unwrap();
        assert!(processes.publish(running));
        let zombie = processes.reserve_child(parent.pid, 10).unwrap();
        assert!(processes.publish(zombie));
        assert!(processes.exit(zombie.pid, linux_wait_status_exit(4)));
        let reserved = processes.reserve_child(parent.pid, 11).unwrap();

        assert_eq!(processes.reparent_children_to_launch_reaper(parent.pid), 2);
        assert_eq!(
            processes
                .by_pid(running.pid)
                .map(|process| process.parent_pid),
            Some(LINUX_LAUNCH_REAPER_PID)
        );
        assert_eq!(
            processes
                .by_pid(zombie.pid)
                .map(|process| process.parent_pid),
            Some(LINUX_LAUNCH_REAPER_PID)
        );
        assert_eq!(processes.by_pid(reserved.pid), None);
        assert!(processes.rollback(reserved));
    }

    #[test]
    fn terminal_child_parent_is_resolved_after_live_reparenting() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let parent = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(parent));
        let child = processes.reserve_child(parent.pid, 9).unwrap();
        assert!(processes.publish(child));
        let stale_child = processes.by_pid(child.pid).unwrap();

        assert_eq!(processes.reparent_children_to_launch_reaper(parent.pid), 1);
        let live_child = processes.by_pid(child.pid).unwrap();
        assert_eq!(stale_child.parent_pid, parent.pid);
        assert_eq!(live_child.parent_pid, LINUX_LAUNCH_REAPER_PID);
        assert_ne!(stale_child.parent_pid, live_child.parent_pid);

        let transition = processes
            .terminate_child(
                child.pid,
                linux_wait_status_signal(9, false).unwrap(),
                LinuxSigchldExitPolicy::RetainZombieAndNotify,
            )
            .expect("the live child record remains terminal-transition eligible");
        assert_eq!(transition.parent_pid, LINUX_LAUNCH_REAPER_PID);
    }

    #[test]
    fn launch_reaper_adopts_and_reaps_every_descendant_without_reaping_root() {
        let mut processes = LinuxProcessTable::<5>::new();
        assert!(!processes.launch_reaper_active());
        processes.register_root(7).unwrap();
        assert!(processes.launch_reaper_active());
        let parent = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(parent));
        let grandchild = processes.reserve_child(parent.pid, 9).unwrap();
        assert!(processes.publish(grandchild));
        let zombie = processes.reserve_child(parent.pid, 10).unwrap();
        assert!(processes.publish(zombie));
        assert!(processes.exit(zombie.pid, linux_wait_status_exit(4)));

        assert_eq!(processes.adopt_launch_descendants(LINUX_ROOT_PID), 3);
        for pid in [parent.pid, grandchild.pid, zombie.pid] {
            assert_eq!(
                processes.by_pid(pid).map(|process| process.parent_pid),
                Some(LINUX_LAUNCH_REAPER_PID)
            );
        }
        assert_eq!(processes.reap_launch_descendants(), 3);
        assert_eq!(processes.resource_counts(), (1, 0));
        assert_eq!(
            processes
                .by_pid(LINUX_ROOT_PID)
                .map(|process| process.state),
            Some(LinuxProcessState::Running)
        );
        assert_eq!(processes.reap_launch_descendants(), 0);
        processes.reset();
        assert!(!processes.launch_reaper_active());
    }

    #[test]
    fn pid_allocator_exhaustion_remains_permanent_across_launch_reset() {
        let max_pid = i32::MAX as usize;
        let mut processes = LinuxProcessTable::<3>::with_next_pid(max_pid);
        processes.register_root(7).unwrap();
        let final_pid = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert_eq!(final_pid.pid, max_pid);
        assert!(processes.rollback(final_pid));
        assert_eq!(
            processes.reserve_child(LINUX_ROOT_PID, 9),
            Err(LinuxProcessError::Exhausted)
        );
        processes.reset();
        processes.register_root(10).unwrap();
        assert_eq!(
            processes.reserve_child(LINUX_ROOT_PID, 11),
            Err(LinuxProcessError::Exhausted)
        );
    }

    #[test]
    fn launch_reset_preserves_boot_pid_high_water() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish(child));
        assert!(processes.exit(child.pid, linux_wait_status_exit(2)));

        processes.reset();
        assert_eq!(processes.by_pid(LINUX_ROOT_PID), None);
        assert_eq!(processes.by_pid(child.pid), None);
        assert_eq!(processes.register_root(17), Ok(LINUX_ROOT_PID));
        assert_eq!(processes.reserve_child(LINUX_ROOT_PID, 18).unwrap().pid, 3);
    }

    #[test]
    fn resource_clone_copies_descriptor_flags_and_shares_open_descriptions() {
        let mut descriptions = LinuxOpenDescriptionTableCore::<8>::new();
        let file = descriptions
            .insert(41, ObjectType::LinuxFile, 0o2002, 7)
            .unwrap();
        let pipe_read = descriptions
            .insert(51, ObjectType::LinuxPipe, 0, 0)
            .unwrap();
        let pipe_write = descriptions
            .insert(52, ObjectType::LinuxPipe, 1, 0)
            .unwrap();
        let queue = descriptions
            .insert_object(61, ObjectType::MessageQueue)
            .unwrap();
        let shared_memory = descriptions
            .insert_object(62, ObjectType::SharedMemory)
            .unwrap();

        let mut parent = LinuxProcessResourceCore::<8, 4>::new();
        assert!(parent.insert_descriptor(3, file, false, &mut descriptions));
        assert!(parent.insert_descriptor(7, file, true, &mut descriptions));
        assert!(parent.insert_descriptor(8, pipe_read, false, &mut descriptions));
        assert!(parent.insert_descriptor(9, pipe_write, true, &mut descriptions));
        assert!(parent.insert_object(queue, &mut descriptions));
        assert!(parent.insert_object(shared_memory, &mut descriptions));

        let clone = LinuxResourceCloneCore::<8, 4>::reserve(&parent, &mut descriptions).unwrap();
        assert_eq!(
            clone.descriptors(),
            &[
                LinuxDescriptorEntry {
                    fd: 3,
                    description_id: file,
                    close_on_exec: false,
                },
                LinuxDescriptorEntry {
                    fd: 7,
                    description_id: file,
                    close_on_exec: true,
                },
                LinuxDescriptorEntry {
                    fd: 8,
                    description_id: pipe_read,
                    close_on_exec: false,
                },
                LinuxDescriptorEntry {
                    fd: 9,
                    description_id: pipe_write,
                    close_on_exec: true,
                },
            ]
        );
        assert_eq!(clone.objects(), &[queue, shared_memory]);
        assert_eq!(descriptions.get(file).unwrap().references, 4);
        assert_eq!(descriptions.get(queue).unwrap().references, 2);
        assert_eq!(descriptions.get(shared_memory).unwrap().references, 2);

        let mut child = LinuxProcessResourceCore::<8, 4>::new();
        assert!(clone.commit(&mut child));
        assert_eq!(child.descriptor(7).unwrap().close_on_exec, true);

        assert!(descriptions.set_offset(file, 4096));
        assert_eq!(
            descriptions
                .get(child.descriptor(3).unwrap().description_id)
                .unwrap()
                .offset,
            4096
        );

        assert_eq!(parent.close_descriptor(3, &mut descriptions), None);
        assert!(parent.descriptor(3).is_none());
        assert!(child.descriptor(3).is_some());
        assert_eq!(descriptions.get(file).unwrap().references, 3);

        assert_eq!(parent.close_descriptor(8, &mut descriptions), None);
        assert!(
            child.descriptor(8).is_some(),
            "the inherited pipe endpoint survives"
        );
        assert_eq!(descriptions.get(pipe_read).unwrap().references, 1);
    }

    #[test]
    fn resource_clone_final_release_destroys_once_and_rollback_preserves_parent() {
        let mut descriptions = LinuxOpenDescriptionTableCore::<6>::new();
        let file = descriptions
            .insert(71, ObjectType::LinuxFile, 0, 0)
            .unwrap();
        let queue = descriptions
            .insert_object(72, ObjectType::MessageQueue)
            .unwrap();
        let mut parent = LinuxProcessResourceCore::<4, 2>::new();
        assert!(parent.insert_descriptor(3, file, false, &mut descriptions));
        assert!(parent.insert_object(queue, &mut descriptions));

        let rollback = LinuxResourceCloneCore::<4, 2>::reserve(&parent, &mut descriptions).unwrap();
        assert_eq!(descriptions.get(file).unwrap().references, 2);
        assert_eq!(descriptions.get(queue).unwrap().references, 2);
        assert_eq!(
            rollback.rollback(&mut descriptions),
            [None, None, None, None, None, None]
        );
        assert_eq!(descriptions.get(file).unwrap().references, 1);
        assert_eq!(descriptions.get(queue).unwrap().references, 1);
        assert!(parent.descriptor(3).is_some());

        let clone = LinuxResourceCloneCore::<4, 2>::reserve(&parent, &mut descriptions).unwrap();
        let mut child = LinuxProcessResourceCore::<4, 2>::new();
        assert!(clone.commit(&mut child));
        assert_eq!(parent.close_descriptor(3, &mut descriptions), None);
        assert_eq!(child.close_descriptor(3, &mut descriptions), Some(71));
        assert!(descriptions.get(file).is_none());
        assert_eq!(parent.release_object(queue, &mut descriptions), None);
        assert_eq!(child.release_object(queue, &mut descriptions), Some(72));
        assert!(descriptions.get(queue).is_none());
    }

    #[test]
    fn resource_clone_accepts_a_parent_without_descriptors_or_objects() {
        let mut descriptions = LinuxOpenDescriptionTableCore::<2>::new();
        let parent = LinuxProcessResourceCore::<2, 2>::new();

        let clone = LinuxResourceCloneCore::reserve(&parent, &mut descriptions).unwrap();
        assert!(clone.descriptors().is_empty());
        assert!(clone.objects().is_empty());

        let mut child = LinuxProcessResourceCore::<2, 2>::new();
        assert!(clone.commit(&mut child));
        assert!(child.descriptors().is_empty());
        assert!(child.objects().is_empty());
    }

    #[test]
    fn fork_reservation_failures_never_publish_or_mutate_the_parent() {
        let mut descriptions = LinuxOpenDescriptionTableCore::<4>::new();
        let file = descriptions
            .insert(91, ObjectType::LinuxFile, 0o2, 37)
            .unwrap();
        let mut parent_resources = LinuxProcessResourceCore::<4, 1>::new();
        assert!(parent_resources.insert_descriptor(5, file, true, &mut descriptions));

        let parent_descriptors = parent_resources.descriptors().to_vec();
        let parent_objects = parent_resources.objects().to_vec();
        let parent_description = *descriptions.get(file).unwrap();

        for failure_after_resource_clone in [false, true] {
            let mut processes = LinuxProcessTable::<3>::new();
            processes.register_root(7).unwrap();
            let parent_snapshot = processes.processes;
            let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();

            assert_eq!(processes.by_pid(child.pid), None);
            assert_eq!(processes.by_scheduler(8), None);
            assert!(
                !processes.has_matching_child(LINUX_ROOT_PID, LinuxWaitSelector::Pid(child.pid),)
            );

            if failure_after_resource_clone {
                let resources =
                    LinuxResourceCloneCore::<4, 1>::reserve(&parent_resources, &mut descriptions)
                        .unwrap();
                assert_eq!(descriptions.get(file).unwrap().references, 2);
                let released = resources.rollback(&mut descriptions);
                assert_eq!(released, [None, None, None, None]);
            }

            assert!(processes.rollback(child));
            assert_eq!(processes.processes, parent_snapshot);
            assert_eq!(processes.by_pid(child.pid), None);
            assert_eq!(parent_resources.descriptors(), parent_descriptors);
            assert_eq!(parent_resources.objects(), parent_objects);
            assert_eq!(*descriptions.get(file).unwrap(), parent_description);
        }

        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let parent_snapshot = processes.processes;
        let child = processes.reserve_child(LINUX_ROOT_PID, 8).unwrap();
        assert!(processes.publish_fork(child));
        assert_eq!(processes.by_pid(child.pid), None);
        assert_eq!(processes.by_scheduler(8), None);
        assert!(processes.rollback_fork(child));
        assert_eq!(processes.processes, parent_snapshot);
    }

    #[test]
    fn fork_process_leader_uses_the_next_task_id_and_keeps_its_exit_signal() {
        use super::linux_task_logic::{LinuxTaskTable, LINUX_ROOT_TID};

        let mut tasks = LinuxTaskTable::<4>::new();
        tasks.register_root(7).unwrap();
        let thread = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        assert_eq!(thread.tid, 2);

        let leader = tasks.reserve_child(0, 9).unwrap();
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();
        let child = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 9, leader.tid, 17)
            .unwrap();

        assert_eq!(child.pid, 3);
        assert_eq!(leader.tid, child.pid);
        assert!(processes.publish(child));
        assert_eq!(processes.by_pid(child.pid).unwrap().exit_signal, 17);
    }

    #[test]
    fn concurrent_fork_process_slots_accept_out_of_order_reserved_task_ids() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();

        let later = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 9, 4, 17)
            .unwrap();
        let earlier = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 8, 3, 17)
            .unwrap();

        assert_eq!(later.pid, 4);
        assert_eq!(earlier.pid, 3);
    }

    #[test]
    fn exact_pid_reservations_accept_lower_ids_after_the_pid_ceiling() {
        let mut processes = LinuxProcessTable::<3>::new();
        processes.register_root(7).unwrap();

        let ceiling = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 9, LINUX_MAX_PID, 17)
            .unwrap();
        let lower = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 8, 3, 17)
            .unwrap();

        assert_eq!(ceiling.pid, LINUX_MAX_PID);
        assert_eq!(lower.pid, 3);
    }

    #[test]
    fn fork_descriptor_failure_rolls_back_every_prior_reference() {
        let mut descriptions = LinuxOpenDescriptionTableCore::<3>::new();
        let first = descriptions
            .insert(101, ObjectType::LinuxFile, 0, 0)
            .unwrap();
        let exhausted = descriptions
            .insert(102, ObjectType::LinuxFile, 0, 0)
            .unwrap();
        let mut parent = LinuxProcessResourceCore::<3, 1>::new();
        assert!(parent.insert_descriptor(3, first, false, &mut descriptions));
        assert!(parent.insert_descriptor(4, exhausted, false, &mut descriptions));
        descriptions
            .descriptions
            .iter_mut()
            .flatten()
            .find(|description| description.id == exhausted)
            .unwrap()
            .references = usize::MAX;

        assert!(LinuxResourceCloneCore::<3, 1>::reserve(&parent, &mut descriptions).is_none());
        assert_eq!(descriptions.get(first).unwrap().references, 1);
        assert_eq!(descriptions.get(exhausted).unwrap().references, usize::MAX);
        assert_eq!(parent.descriptor(3).unwrap().description_id, first);
        assert_eq!(parent.descriptor(4).unwrap().description_id, exhausted);
    }
}

pub fn linux_signal_lifecycle_behavior_contract() {
    use linux_process_logic::{
        apply_linux_terminal_child_transition, linux_signal_delivery_route,
        linux_wait_status_signal, LinuxProcessSignalStateCore, LinuxProcessTable,
        LinuxSigchldExitPolicy, LinuxSignalDeliveryRoute, LinuxWaitOutcome, LinuxWaitSelector,
        LINUX_ROOT_PID, LINUX_SIGCHLD,
    };
    use linux_task_logic::{
        linux_signal_bit, linux_signal_disposition, live_process_task_count,
        prepare_fork_task_signal_state_for_behavior, retire_process_tasks_for_behavior,
        scoped_process_signal_target, LinuxBlockReason, LinuxPendingSignal, LinuxPendingSignals,
        LinuxSignalDisposition, LinuxSignalFrame, LinuxSignalStack, LinuxTaskState, LinuxTaskTable,
        LINUX_ROOT_TID, LINUX_SS_DISABLE,
    };

    let mut parent_process_signals =
        LinuxProcessSignalStateCore::<u64, LinuxPendingSignals, 65>::new(
            0,
            LinuxPendingSignals::new(),
        );
    parent_process_signals.signal_actions[12] = 0x1200;
    parent_process_signals
        .process_pending
        .queue(LinuxPendingSignal::standard(10))
        .unwrap();
    let child_process_signals = parent_process_signals.fork_child(LinuxPendingSignals::new());
    assert_eq!(
        child_process_signals.signal_actions, parent_process_signals.signal_actions,
        "fork copies process dispositions"
    );
    assert_eq!(
        child_process_signals.process_pending.standard_pending, 0,
        "fork clears process-pending signals"
    );

    let mut fork_tasks = LinuxTaskTable::<2>::new();
    fork_tasks.register_root(7).unwrap();
    let parent_task_signals = fork_tasks.signal_state_mut(LINUX_ROOT_TID, 7).unwrap();
    parent_task_signals.mask = linux_signal_bit(12) | linux_signal_bit(15);
    parent_task_signals
        .queue(LinuxPendingSignal::standard(12))
        .unwrap();
    parent_task_signals.alt_stack = LinuxSignalStack {
        sp: 0x4000,
        flags: 0,
        _padding: 0,
        size: 0x2000,
    };
    parent_task_signals
        .push_frame(LinuxSignalFrame {
            regs: [0x55; 32],
            return_pc: 0x8000,
            previous_mask: 0x22,
            user_sp: 0x5000,
            previous_stack_flags: 0,
            restart: None,
        })
        .unwrap();
    assert!(parent_task_signals.request_sigreturn());
    let child_task = fork_tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
    prepare_fork_task_signal_state_for_behavior(&mut fork_tasks, child_task, 7);
    assert!(fork_tasks.publish(child_task));
    let child_task_signals = fork_tasks
        .signal_state(child_task.tid, child_task.scheduler_thread)
        .unwrap();
    assert_eq!(
        child_task_signals.mask,
        linux_signal_bit(12) | linux_signal_bit(15),
        "fork copies only the calling thread mask"
    );
    assert_eq!(child_task_signals.pending_mask(), 0);
    assert_eq!(child_task_signals.frame_depth, 0);
    assert!(!child_task_signals.sigreturn_requested);
    assert_eq!(child_task_signals.alt_stack.flags, LINUX_SS_DISABLE as u32);

    let mut routed_tasks = LinuxTaskTable::<4>::new();
    routed_tasks.register_root(17).unwrap();
    let masked = routed_tasks.reserve_child(LINUX_ROOT_TID, 18).unwrap();
    let competing = routed_tasks.reserve_child(42, 19).unwrap();
    let selected = routed_tasks.reserve_child(LINUX_ROOT_TID, 20).unwrap();
    assert!(routed_tasks.publish(masked));
    assert!(routed_tasks.publish(competing));
    assert!(routed_tasks.publish(selected));
    let signal = 15;
    let signal_bit = linux_signal_bit(signal);
    routed_tasks
        .signal_state_mut(LINUX_ROOT_TID, 17)
        .unwrap()
        .mask = signal_bit;
    routed_tasks
        .signal_state_mut(masked.tid, masked.scheduler_thread)
        .unwrap()
        .mask = signal_bit;
    assert_eq!(
        scoped_process_signal_target(&routed_tasks, LINUX_ROOT_TID, signal)
            .map(|task| (task.tid, task.tgid)),
        Some((selected.tid, LINUX_ROOT_TID)),
        "process-directed selection is scoped to the target TGID"
    );

    for (policy, expected_wait, expect_sigchld) in [
        (
            LinuxSigchldExitPolicy::RetainZombieAndNotify,
            LinuxWaitOutcome::Ready { pid: 2, status: 9 },
            true,
        ),
        (
            LinuxSigchldExitPolicy::ReapAndNotify,
            LinuxWaitOutcome::NoChildren,
            true,
        ),
        (
            LinuxSigchldExitPolicy::ReapWithoutNotify,
            LinuxWaitOutcome::NoChildren,
            false,
        ),
    ] {
        let disposition = linux_signal_disposition(0, 9);
        assert_eq!(disposition, LinuxSignalDisposition::Terminate);
        assert_eq!(
            linux_signal_delivery_route(
                disposition,
                LinuxSignalDisposition::Ignore,
                LinuxSignalDisposition::Terminate,
            ),
            LinuxSignalDeliveryRoute::TerminateProcess,
            "default SIGKILL routes through process termination"
        );

        let mut processes = LinuxProcessTable::<2>::new();
        processes.register_root(7).unwrap();
        let child = processes
            .reserve_child_with_pid(LINUX_ROOT_PID, 8, 2, LINUX_SIGCHLD)
            .unwrap();
        assert!(processes.publish(child));

        let mut tasks = LinuxTaskTable::<4>::new();
        tasks.register_root(7).unwrap();
        let first = tasks.reserve_child(child.pid, 8).unwrap();
        let second = tasks.reserve_child(child.pid, 9).unwrap();
        let competing = tasks.reserve_child(77, 10).unwrap();
        assert!(tasks.publish(first));
        assert!(tasks.publish(second));
        assert!(tasks.publish(competing));
        assert!(tasks.block(LINUX_ROOT_TID, 7, LinuxBlockReason::ChildWait));

        assert_eq!(retire_process_tasks_for_behavior(&mut tasks, child.pid), 2);
        assert_eq!(live_process_task_count(&tasks, child.pid), 0);
        assert_eq!(live_process_task_count(&tasks, 77), 1);

        let wait_status = linux_wait_status_signal(9, false).unwrap();
        let transition = processes
            .terminate_child(child.pid, wait_status, policy)
            .expect("SIGKILL terminal transition");
        let mut process_pending = LinuxPendingSignals::new();
        apply_linux_terminal_child_transition(
            transition,
            |_, notification_signal| {
                process_pending.queue(LinuxPendingSignal::standard(notification_signal))
            },
            |parent_pid| {
                for waiter in tasks.child_waiters(parent_pid).into_iter().flatten() {
                    assert!(tasks.wake(waiter.tid, waiter.scheduler_thread));
                }
            },
        )
        .unwrap();

        assert_eq!(
            process_pending.standard_pending & linux_signal_bit(17) != 0,
            expect_sigchld,
            "SIGCHLD delivery follows the terminal policy"
        );
        assert_eq!(
            tasks.by_tid(LINUX_ROOT_TID).map(|task| task.state),
            Some(LinuxTaskState::Runnable),
            "a blocked child waiter wakes for every terminal policy"
        );
        assert_eq!(
            processes.wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Pid(child.pid)),
            expected_wait
        );
    }
}

mod linux_process_memory_logic {
    extern crate alloc;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum ObjectType {
        LinuxFile,
        SharedMemory,
    }

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_process_logic_shared.rs"
    ));
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_task_logic_shared.rs"
    ));
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_process_memory_logic_shared.rs"
    ));
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_fork_logic_shared.rs"
    ));

    #[test]
    fn copy_address_errors_are_efault() {
        for error in [
            LinuxAddressSpaceErrorKind::InvalidAddress,
            LinuxAddressSpaceErrorKind::InvalidPermissions,
            LinuxAddressSpaceErrorKind::AlreadyMapped,
            LinuxAddressSpaceErrorKind::NotMapped,
            LinuxAddressSpaceErrorKind::PermissionDenied,
        ] {
            assert_eq!(
                linux_copy_address_error_class(error),
                LinuxCopyAddressErrorClass::Fault,
            );
        }
        assert_eq!(
            linux_copy_address_error_class(LinuxAddressSpaceErrorKind::OutOfMemory),
            LinuxCopyAddressErrorClass::OutOfMemory,
        );
    }

    #[test]
    fn shared_attachment_replacement_reconciles_final_mappings() {
        let attachment = LinuxSharedAttachmentRecord {
            object_id: 7,
            addr: 0x1200_0000,
            len: 0x3000,
        };
        let partial_replacement = [LinuxSharedMappingRange {
            object_id: 7,
            addr: 0x1200_2000,
            len: 0x1000,
        }];

        assert_eq!(
            linux_shared_attachment_detached_reference(attachment, &partial_replacement),
            None,
            "one surviving page retains the SysV attachment reference",
        );
        assert_eq!(
            linux_shared_attachment_detached_reference(attachment, &[]),
            Some((7, 0x1200_0000)),
            "full replacement emits exactly one detached attachment reference",
        );
    }

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/ARM64/context_shared.rs"
    ));

    struct PerPageProtectionOps {
        mapped: Vec<(usize, u64, usize)>,
        unmapped: Vec<usize>,
        map_attempts: usize,
        fail_map_attempt: Option<usize>,
    }

    impl PerPageProtectionOps {
        fn new(fail_map_attempt: Option<usize>) -> Self {
            Self {
                mapped: Vec::new(),
                unmapped: Vec::new(),
                map_attempts: 0,
                fail_map_attempt,
            }
        }
    }

    impl LinuxForkPageOps for PerPageProtectionOps {
        type Page = u64;
        type Error = ();

        fn failure_error(&self) -> Self::Error {}

        fn is_private(&self, _page: Self::Page) -> bool {
            true
        }

        fn allocate_private(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error> {
            Ok(parent)
        }

        fn copy_private(
            &mut self,
            _parent: Self::Page,
            _child: Self::Page,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn acquire_shared(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error> {
            Ok(parent)
        }

        fn release_page(&mut self, _page: Self::Page) {}

        fn map_page(
            &mut self,
            address: usize,
            page: Self::Page,
            prot: usize,
        ) -> Result<(), Self::Error> {
            self.map_attempts += 1;
            if self.fail_map_attempt == Some(self.map_attempts) {
                return Err(());
            }
            self.mapped.push((address, page, prot));
            Ok(())
        }

        fn unmap_page(&mut self, address: usize) {
            self.unmapped.push(address);
            let index = self
                .mapped
                .iter()
                .position(|(mapped, _, _)| *mapped == address)
                .expect("rollback address was mapped");
            self.mapped.remove(index);
        }
    }

    #[test]
    fn map_linux_fork_pages_with_protection_applies_each_page_and_rolls_back_in_reverse() {
        let pages = [41, 42, 43];
        let mut success = PerPageProtectionOps::new(None);
        map_linux_fork_pages_with_protection(
            &mut success,
            0x1000,
            0x1000,
            &pages,
            |index| {
                if index == 1 {
                    0
                } else {
                    LINUX_PROT_READ
                }
            },
            |_| false,
        )
        .expect("per-page fork map");
        assert_eq!(
            success
                .mapped
                .iter()
                .map(|(_, _, prot)| *prot)
                .collect::<Vec<_>>(),
            vec![LINUX_PROT_READ, 0, LINUX_PROT_READ]
        );

        let mut failure = PerPageProtectionOps::new(Some(3));
        assert_eq!(
            map_linux_fork_pages_with_protection(
                &mut failure,
                0x1000,
                0x1000,
                &pages,
                |_| LINUX_PROT_READ,
                |_| false,
            ),
            Err(())
        );
        assert!(failure.mapped.is_empty());
        assert_eq!(failure.unmapped, vec![0x2000, 0x1000]);
    }

    #[test]
    fn memory_fault_policy_distinguishes_maperr_accerr_and_file_tail_bus() {
        let anonymous = LinuxMemoryFaultRegion {
            addr: 0x1200_0000,
            len: 0x2000,
            prot: LINUX_PROT_READ,
            file_offset: None,
            backing_len: None,
        };
        let file = LinuxMemoryFaultRegion {
            addr: 0x1300_0000,
            len: 0x3000,
            prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
            file_offset: Some(0),
            backing_len: Some(0x800),
        };

        assert_eq!(
            linux_memory_fault_signal(
                &[anonymous, file],
                0x1100_0000,
                LinuxMemoryFaultAccess::Read,
                0x1000,
            ),
            LinuxMemoryFaultSignal::SegvMaperr
        );
        assert_eq!(
            linux_memory_fault_signal(
                &[anonymous, file],
                0x1200_0008,
                LinuxMemoryFaultAccess::Write,
                0x1000,
            ),
            LinuxMemoryFaultSignal::SegvAccerr
        );
        assert_eq!(
            linux_memory_fault_signal(
                &[anonymous, file],
                0x1300_1001,
                LinuxMemoryFaultAccess::Write,
                0x1000,
            ),
            LinuxMemoryFaultSignal::BusAdrerr
        );
        assert_eq!(
            linux_memory_fault_signal(&[file], 0x1300_07ff, LinuxMemoryFaultAccess::Read, 0x1000,),
            LinuxMemoryFaultSignal::SegvAccerr,
            "the partial final page is not a beyond-object page"
        );
    }

    #[test]
    fn effective_file_page_protection_blocks_only_pages_wholly_beyond_object() {
        assert_eq!(
            linux_effective_mapping_page_prot(
                LINUX_PROT_READ | LINUX_PROT_WRITE,
                Some(0),
                Some(0x800),
                0,
                0x1000,
            ),
            LINUX_PROT_READ | LINUX_PROT_WRITE
        );
        assert_eq!(
            linux_effective_mapping_page_prot(
                LINUX_PROT_READ | LINUX_PROT_WRITE,
                Some(0),
                Some(0x800),
                1,
                0x1000,
            ),
            0
        );
        assert_eq!(
            linux_effective_mapping_page_prot(LINUX_PROT_EXEC, None, None, 99, 0x1000),
            LINUX_PROT_EXEC
        );
    }

    #[test]
    fn process_memory_metadata_is_independent_per_pid() {
        let mut first = LinuxProcessMemoryCore::<4>::new(1, 0x4000).unwrap();
        let mut second = LinuxProcessMemoryCore::<4>::new(2, 0x8000).unwrap();

        assert!(first.set_initial_stack(0x1ffd_f000, 0x20_000));
        assert!(first.set_brk(0x1200_0000, 0x1200_3000, 0x1210_0000));
        assert!(first.push_mapping(LinuxProcessMappingCore {
            owner_pid: 1,
            addr: 0x1300_0000,
            len: 0x2000,
            prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
            flags: LINUX_MAP_PRIVATE,
        }));

        assert_eq!(first.root_paddr, 0x4000);
        assert_eq!(first.mapping_count(), 1);
        assert_eq!(first.initial_stack, Some((0x1ffd_f000, 0x20_000)));
        assert_eq!(first.brk.current, 0x1200_3000);
        assert_eq!(second.root_paddr, 0x8000);
        assert_eq!(second.mapping_count(), 0);
        assert_eq!(second.initial_stack, None);
        assert_eq!(first.next_addr, second.next_addr);
        assert!(first.set_next_addr(0x1400_0000));
        assert_ne!(first.next_addr, second.next_addr);

        assert!(!second.push_mapping(LinuxProcessMappingCore {
            owner_pid: 1,
            addr: 0x1400_0000,
            len: 0x1000,
            prot: LINUX_PROT_READ,
            flags: LINUX_MAP_PRIVATE,
        }));
    }

    #[test]
    fn private_and_shared_backing_have_exact_reference_rules() {
        let private = LinuxPageBacking::Private { pfn: 17 };
        let shared = LinuxPageBacking::Shared {
            object_id: 9,
            page_index: 2,
            pfn: 33,
        };

        assert_eq!(private.pfn(), 17);
        assert_eq!(shared.pfn(), 33);
        assert!(!private.is_shared());
        assert!(shared.is_shared());
        assert_eq!(linux_shared_reference_acquire(1), Some(2));
        assert_eq!(linux_shared_reference_release(2), Some(1));
        assert_eq!(linux_shared_reference_release(1), Some(0));
        assert_eq!(linux_shared_reference_acquire(usize::MAX), None);
        assert_eq!(linux_shared_reference_release(0), None);
    }

    #[test]
    fn shared_file_mapping_identity_follows_the_inode_not_its_path_alias() {
        assert!(linux_shared_file_identity_matches(41, 41));
        assert!(!linux_shared_file_identity_matches(41, 42));
    }

    #[test]
    fn checked_copy_chunks_cross_pages_without_skipping_bytes() {
        let mut address = 0x1000_0ff0usize;
        let mut remaining = 0x1020usize;
        let mut chunks = [0usize; 3];
        let mut count = 0usize;

        while remaining != 0 {
            let chunk = linux_user_copy_chunk(address, remaining, LINUX_PAGE_SIZE).unwrap();
            chunks[count] = chunk;
            count += 1;
            address += chunk;
            remaining -= chunk;
        }

        assert_eq!(&chunks[..count], &[0x10, 0x1000, 0x10]);
        assert!(linux_mapping_allows(LINUX_PROT_READ, false));
        assert!(!linux_mapping_allows(LINUX_PROT_READ, true));
        assert!(linux_mapping_allows(LINUX_PROT_WRITE, true));
        assert_eq!(linux_user_copy_chunk(0x1000, 1, 0), None);
    }

    #[test]
    fn mapping_metadata_denies_execute_only_data_access_and_covers_brk() {
        let ranges = [
            LinuxMappingAccessRange {
                addr: 0x1200_0000,
                len: 0x1000,
                prot: LINUX_PROT_EXEC,
            },
            LinuxMappingAccessRange {
                addr: 0x1200_1000,
                len: 0x1000,
                prot: LINUX_PROT_READ,
            },
            LinuxMappingAccessRange {
                addr: 0x1200_2000,
                len: 0x1000,
                prot: LINUX_PROT_WRITE,
            },
            LinuxMappingAccessRange {
                addr: LINUX_BRK_BASE,
                len: 0x2000,
                prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
            },
        ];

        assert!(!linux_mapping_access_range_covered(
            ranges,
            0x1200_0800,
            1,
            false,
        ));
        assert!(!linux_mapping_access_range_covered(
            ranges,
            0x1200_0800,
            1,
            true,
        ));
        assert!(linux_mapping_access_range_covered(
            ranges,
            0x1200_1800,
            0x1000,
            false,
        ));
        assert!(!linux_mapping_access_range_covered(
            ranges,
            0x1200_1800,
            0x1000,
            true,
        ));
        assert!(linux_mapping_access_range_covered(
            ranges,
            LINUX_BRK_BASE + 0x1000,
            0x1000,
            true,
        ));
        assert!(!linux_mapping_access_range_covered(
            ranges,
            0x1200_3000,
            1,
            false,
        ));
        assert!(!linux_mapping_access_range_covered(
            ranges,
            usize::MAX,
            2,
            false,
        ));
    }

    #[test]
    fn mapping_coverage_accepts_adjacent_ranges_and_rejects_gaps() {
        let adjacent = [
            LinuxMappingRange {
                addr: 0x1200_0000,
                len: 0x1000,
            },
            LinuxMappingRange {
                addr: 0x1200_1000,
                len: 0x2000,
            },
        ];
        let gapped = [
            LinuxMappingRange {
                addr: 0x1200_0000,
                len: 0x1000,
            },
            LinuxMappingRange {
                addr: 0x1200_2000,
                len: 0x1000,
            },
        ];

        assert!(linux_mapping_range_covered(&adjacent, 0x1200_0800, 0x1800));
        assert!(!linux_mapping_range_covered(&gapped, 0x1200_0800, 0x1800));
        assert!(!linux_mapping_range_covered(&adjacent, usize::MAX, 2));
    }

    #[test]
    fn process_removal_selects_only_the_requested_pid() {
        let pids = [1usize, 7, 11];

        assert_eq!(linux_process_memory_remove_index(&pids, 7), Some(1));
        assert_eq!(linux_process_memory_remove_index(&pids, 1), Some(0));
        assert_eq!(linux_process_memory_remove_index(&pids, 9), None);
    }

    #[test]
    fn fixed_remap_moves_even_when_lengths_match() {
        assert!(!linux_mremap_requires_move(
            0x1200_0000,
            0x2000,
            0x2000,
            None,
            false,
        ));
        assert!(!linux_mremap_requires_move(
            0x1200_0000,
            0x2000,
            0x2000,
            Some(0x1200_0000),
            false,
        ));
        assert!(linux_mremap_requires_move(
            0x1200_0000,
            0x2000,
            0x2000,
            Some(0x1201_0000),
            false,
        ));
        assert!(linux_mremap_requires_move(
            0x1200_0000,
            0x2000,
            0x3000,
            None,
            false,
        ));
        assert!(linux_mremap_requires_move(
            0x1200_0000,
            0x2000,
            0x2000,
            None,
            true,
        ));
    }

    #[test]
    fn clone_backing_copies_private_pages_and_reuses_shared_pages() {
        let private = LinuxPageBacking::Private { pfn: 17 };
        let shared = LinuxPageBacking::Shared {
            object_id: 9,
            page_index: 2,
            pfn: 33,
        };

        assert_eq!(
            linux_clone_page_backing(private, 81),
            LinuxPageBacking::Private { pfn: 81 }
        );
        assert_ne!(linux_clone_page_backing(private, 81).pfn(), private.pfn());
        assert_eq!(linux_clone_page_backing(shared, 99), shared);
    }

    #[test]
    fn ordinary_map_shared_pages_use_shared_fork_backing() {
        assert!(linux_mmap_backing_is_shared(LINUX_MAP_SHARED));
        assert!(!linux_mmap_backing_is_shared(LINUX_MAP_PRIVATE));

        let parent = LinuxPageBacking::Shared {
            object_id: u32::MAX,
            page_index: 0,
            pfn: 33,
        };
        assert_eq!(linux_clone_page_backing(parent, 81), parent);
    }

    #[test]
    fn shared_file_page_indices_reject_offset_overflow() {
        assert_eq!(linux_shared_page_index(7, 3), Some(10));
        assert_eq!(linux_shared_page_index(usize::MAX, 1), None);
    }

    #[test]
    fn fork_page_rollback_releases_each_private_copy_and_shared_reference() {
        let parent_pages = [
            LinuxPageBacking::Private { pfn: 17 },
            LinuxPageBacking::Private { pfn: 18 },
            LinuxPageBacking::Shared {
                object_id: 9,
                page_index: 0,
                pfn: 33,
            },
        ];
        let parent_snapshot = parent_pages;

        for fail_after in 0..=parent_pages.len() {
            let mut shared = LinuxSharedPageTableCore::<2>::new();
            assert!(shared.insert(9, 0, 33));
            let mut private_copies = Vec::new();
            let mut acquired_shared = Vec::new();

            for (index, page) in parent_pages.iter().copied().enumerate() {
                if index == fail_after {
                    break;
                }
                match page {
                    LinuxPageBacking::Private { .. } => {
                        private_copies.push(linux_clone_page_backing(page, 80 + index as u64));
                    }
                    LinuxPageBacking::Shared {
                        object_id,
                        page_index,
                        ..
                    } => {
                        assert!(shared.acquire(object_id, page_index));
                        acquired_shared.push((object_id, page_index));
                    }
                }
            }

            private_copies.clear();
            for (object_id, page_index) in acquired_shared.into_iter().rev() {
                assert_eq!(shared.release(object_id, page_index), None);
            }

            assert_eq!(parent_pages, parent_snapshot);
            assert_eq!(shared.get(9, 0).unwrap().references, 1);
        }
    }

    #[test]
    fn shared_page_name_removal_defers_destruction_until_final_reference() {
        let mut pages = LinuxSharedPageTableCore::<2>::new();
        assert!(pages.insert(7, 0, 41));
        assert!(pages.acquire(7, 0));
        assert_eq!(pages.get(7, 0).unwrap().references, 2);
        assert!(pages.remove_name(7));
        assert!(pages.get(7, 0).is_none());
        assert_eq!(pages.release(7, 0), None);
        assert_eq!(pages.get_any(7, 0).unwrap().references, 1);
        assert_eq!(pages.release(7, 0), Some(41));
        assert!(pages.get_any(7, 0).is_none());
    }

    #[test]
    fn shared_page_acquire_or_insert_selects_one_canonical_backing() {
        let mut pages = LinuxSharedPageTableCore::<2>::new();
        assert_eq!(pages.acquire_or_insert(7, 0, 41), Some(41));
        assert_eq!(pages.acquire_or_insert(7, 0, 99), Some(41));
        assert_eq!(pages.get(7, 0).unwrap().references, 2);
        assert_eq!(pages.release(7, 0), None);
        assert_eq!(pages.release(7, 0), Some(41));
    }

    #[test]
    fn shared_attachment_survives_mapping_splits_until_its_final_fragment_is_removed() {
        let attachment = LinuxSharedAttachmentRecord {
            object_id: 7,
            addr: 0x2000,
            len: 0x3000,
        };
        let fragments = [
            LinuxSharedMappingRange {
                object_id: 7,
                addr: 0x2000,
                len: 0x1000,
            },
            LinuxSharedMappingRange {
                object_id: 7,
                addr: 0x4000,
                len: 0x1000,
            },
        ];

        assert!(linux_shared_attachment_has_mapping(attachment, &fragments));
        assert!(linux_shared_attachment_has_mapping(
            attachment,
            &fragments[1..]
        ));
        assert!(!linux_shared_attachment_has_mapping(attachment, &[]));
        assert!(!linux_shared_attachment_has_mapping(
            attachment,
            &[LinuxSharedMappingRange {
                object_id: 8,
                addr: 0x2000,
                len: 0x3000,
            }]
        ));
    }

    #[test]
    fn shared_mremap_is_allowed_only_when_it_preserves_the_existing_mapping() {
        assert!(linux_shared_mremap_supported(false));
        assert!(!linux_shared_mremap_supported(true));
    }

    #[test]
    fn forked_process_attributes_are_independent_and_inherit_security_views() {
        let parent = LinuxProcessAttributesCore {
            namespace_flags: 0x0200_0000,
            setns_count: 3,
            mount_count: 2,
            mount_flags: 0x4000,
            pivot_rooted: true,
            chrooted: true,
            no_new_privs: true,
            seccomp_mode: 2,
            seccomp_filters: 4,
            cap_effective: 0x55,
            cap_permitted: 0xaa,
            cap_inheritable: 0x11,
            hostname_set: true,
            domainname_set: false,
        };

        let mut child = parent.fork_child(0x2000_0000);
        child.cap_effective = 0;
        child.mount_count = 0;
        child.chrooted = false;

        assert_eq!(parent.cap_effective, 0x55);
        assert_eq!(parent.mount_count, 2);
        assert!(parent.chrooted);
        assert_eq!(child.namespace_flags, 0x2200_0000);
        assert!(child.no_new_privs);
        assert_eq!(child.seccomp_filters, 4);
    }

    #[test]
    fn fork_transaction_ledger_rolls_back_every_stage_in_reverse_order() {
        let stages = [
            LinuxForkAcquisition::SchedulerThread,
            LinuxForkAcquisition::Task,
            LinuxForkAcquisition::Process,
            LinuxForkAcquisition::Resources,
            LinuxForkAcquisition::Memory,
            LinuxForkAcquisition::Configured,
        ];

        for fail_after in 0..=stages.len() {
            let mut ledger = LinuxForkAcquisitionLedger::new();
            for stage in stages.iter().copied().take(fail_after) {
                assert!(ledger.acquire(stage));
            }
            let mut rollback = [None; 6];
            let rollback_len = ledger.rollback_into(&mut rollback);
            assert_eq!(rollback_len, fail_after);
            for index in 0..fail_after {
                assert_eq!(rollback[index], Some(stages[fail_after - index - 1]));
            }
            assert!(ledger.is_empty());
        }
    }

    #[test]
    fn fork_failure_schedule_targets_every_resource_occurrence() {
        let points = [
            LinuxForkFailurePoint::SchedulerThread,
            LinuxForkFailurePoint::Task,
            LinuxForkFailurePoint::Process,
            LinuxForkFailurePoint::ChildRoot,
            LinuxForkFailurePoint::TablePage,
            LinuxForkFailurePoint::DescriptorReference,
            LinuxForkFailurePoint::SharedReference,
            LinuxForkFailurePoint::PrivatePage,
            LinuxForkFailurePoint::PrivatePageAllocation,
            LinuxForkFailurePoint::PrivatePageCopy,
            LinuxForkFailurePoint::PrivatePageMap,
            LinuxForkFailurePoint::SharedPageMap,
            LinuxForkFailurePoint::Memory,
            LinuxForkFailurePoint::Configured,
            LinuxForkFailurePoint::ProcessPublication,
            LinuxForkFailurePoint::TaskPublication,
            LinuxForkFailurePoint::SchedulerPublication,
        ];

        for point in points {
            let mut schedule = LinuxForkFailureSchedule::new(point, 2);
            assert!(!schedule.should_fail(point));
            assert!(!schedule.should_fail(point));
            assert!(schedule.should_fail(point));
            assert!(!schedule.should_fail(point));
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum HostSchedulerState {
        Empty,
        Suspended,
        Ready,
        Running,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct HostPrivatePage {
        pfn: u64,
        bytes: [u8; LINUX_PAGE_SIZE],
    }

    struct HostForkKernel {
        processes: LinuxProcessTable<4>,
        tasks: LinuxTaskTable<4>,
        scheduler: [HostSchedulerState; 4],
        address_space_roots: Vec<u64>,
        table_pages: Vec<u64>,
        private_pages: Vec<HostPrivatePage>,
        shared_pages: LinuxSharedPageTableCore<4>,
        mapped_pages: Vec<(usize, LinuxPageBacking)>,
        shared_attachment_references: usize,
        descriptions: LinuxOpenDescriptionTableCore<8>,
        parent_resources: LinuxProcessResourceCore<4, 2>,
        child_resources: Option<LinuxProcessResourceCore<4, 2>>,
        memory_pids: Vec<usize>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct HostLinuxProcessResourceCounts {
        linux_processes: usize,
        linux_zombies: usize,
        private_pages: usize,
        shared_pages: usize,
        page_table_pages: usize,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ForkResourceSnapshot {
        process_records: Vec<LinuxProcessCore>,
        task_records: Vec<LinuxTaskCore>,
        task_signal_states: Vec<LinuxTaskSignalState>,
        task_sleep_waits: Vec<Option<LinuxSleepWait>>,
        task_clear_child_tids: Vec<usize>,
        scheduler_states: Vec<HostSchedulerState>,
        root_pages: Vec<u64>,
        table_page_ids: Vec<u64>,
        private_page_images: Vec<HostPrivatePage>,
        shared_page_records: Vec<Option<LinuxSharedPageRecord>>,
        mapped_page_records: Vec<(usize, LinuxPageBacking)>,
        open_descriptions: Vec<Option<LinuxOpenDescription>>,
        next_description_id: u32,
        parent_descriptors: Vec<LinuxDescriptorEntry>,
        parent_objects: Vec<u32>,
        child_descriptors: Option<Vec<LinuxDescriptorEntry>>,
        child_objects: Option<Vec<u32>>,
        memory_pids: Vec<usize>,
        shared_attachment_references: usize,
        process_slots: usize,
        scheduler_threads: usize,
        child_tasks: usize,
        address_space_roots: usize,
        table_pages: usize,
        private_pages: usize,
        shared_references: usize,
        descriptor_references: usize,
        process_resources: usize,
        publishing_processes: usize,
        published_child_tasks: usize,
        ready_scheduler_threads: usize,
        visible_pids: Vec<usize>,
    }

    impl HostForkKernel {
        fn parent(parent_bytes: &[u8; LINUX_PAGE_SIZE]) -> Self {
            let mut processes = LinuxProcessTable::<4>::new();
            processes.register_root(1).expect("register parent process");
            let mut tasks = LinuxTaskTable::<4>::new();
            tasks.register_root(1).expect("register parent task");
            tasks.signal_states[0].mask = linux_signal_bit(2) | linux_signal_bit(10);
            let mut descriptions = LinuxOpenDescriptionTableCore::<8>::new();
            let file = descriptions
                .insert(41, ObjectType::LinuxFile, 0, 0)
                .expect("parent file description");
            let shared = descriptions
                .insert_object(51, ObjectType::SharedMemory)
                .expect("parent shared object");
            let mut parent_resources = LinuxProcessResourceCore::<4, 2>::new();
            assert!(parent_resources.insert_descriptor(3, file, false, &mut descriptions));
            assert!(parent_resources.insert_descriptor(7, file, true, &mut descriptions));
            assert!(parent_resources.insert_object(shared, &mut descriptions));
            let mut shared_pages = LinuxSharedPageTableCore::<4>::new();
            assert!(shared_pages.insert(9, 0, 31));
            assert!(shared_pages.insert(9, 1, 32));
            Self {
                processes,
                tasks,
                scheduler: [
                    HostSchedulerState::Empty,
                    HostSchedulerState::Running,
                    HostSchedulerState::Empty,
                    HostSchedulerState::Empty,
                ],
                address_space_roots: vec![0x1000],
                table_pages: vec![0x2000, 0x3000, 0x4000],
                private_pages: vec![
                    HostPrivatePage {
                        pfn: 17,
                        bytes: *parent_bytes,
                    },
                    HostPrivatePage {
                        pfn: 18,
                        bytes: core::array::from_fn(|index| {
                            parent_bytes[LINUX_PAGE_SIZE - 1 - index]
                        }),
                    },
                ],
                shared_pages,
                mapped_pages: vec![
                    (0x1000_0000, LinuxPageBacking::Private { pfn: 17 }),
                    (0x1000_1000, LinuxPageBacking::Private { pfn: 18 }),
                    (
                        0x1200_0000,
                        LinuxPageBacking::Shared {
                            object_id: 9,
                            page_index: 0,
                            pfn: 31,
                        },
                    ),
                    (
                        0x1200_1000,
                        LinuxPageBacking::Shared {
                            object_id: 9,
                            page_index: 1,
                            pfn: 32,
                        },
                    ),
                ],
                shared_attachment_references: 1,
                descriptions,
                parent_resources,
                child_resources: None,
                memory_pids: vec![LINUX_ROOT_PID],
            }
        }

        fn snapshot(&self) -> ForkResourceSnapshot {
            let visible_pids = self
                .processes
                .processes
                .iter()
                .filter(|process| {
                    matches!(
                        process.state,
                        LinuxProcessState::Running | LinuxProcessState::Zombie
                    )
                })
                .map(|process| process.pid)
                .collect();
            ForkResourceSnapshot {
                process_records: self.processes.processes.to_vec(),
                task_records: self.tasks.tasks.to_vec(),
                task_signal_states: self.tasks.signal_states.to_vec(),
                task_sleep_waits: self.tasks.sleep_waits.to_vec(),
                task_clear_child_tids: self.tasks.clear_child_tids.to_vec(),
                scheduler_states: self.scheduler.to_vec(),
                root_pages: self.address_space_roots.clone(),
                table_page_ids: self.table_pages.clone(),
                private_page_images: self.private_pages.clone(),
                shared_page_records: self.shared_pages.pages.to_vec(),
                mapped_page_records: self.mapped_pages.clone(),
                open_descriptions: self.descriptions.descriptions.to_vec(),
                next_description_id: self.descriptions.next_id,
                parent_descriptors: self.parent_resources.descriptors().to_vec(),
                parent_objects: self.parent_resources.objects().to_vec(),
                child_descriptors: self
                    .child_resources
                    .as_ref()
                    .map(|resources| resources.descriptors().to_vec()),
                child_objects: self
                    .child_resources
                    .as_ref()
                    .map(|resources| resources.objects().to_vec()),
                memory_pids: self.memory_pids.clone(),
                shared_attachment_references: self.shared_attachment_references,
                process_slots: self
                    .processes
                    .processes
                    .iter()
                    .filter(|process| process.state != LinuxProcessState::Empty)
                    .count(),
                scheduler_threads: self
                    .scheduler
                    .iter()
                    .filter(|state| **state != HostSchedulerState::Empty)
                    .count(),
                child_tasks: self
                    .tasks
                    .tasks
                    .iter()
                    .filter(|task| task.state != LinuxTaskState::Empty)
                    .count(),
                address_space_roots: self.address_space_roots.len(),
                table_pages: self.table_pages.len(),
                private_pages: self.private_pages.len(),
                shared_references: self.shared_attachment_references
                    + self
                        .shared_pages
                        .pages
                        .iter()
                        .flatten()
                        .map(|page| page.references)
                        .sum::<usize>(),
                descriptor_references: self
                    .descriptions
                    .descriptions
                    .iter()
                    .flatten()
                    .map(|description| description.references)
                    .sum(),
                process_resources: 1 + usize::from(self.child_resources.is_some()),
                publishing_processes: self
                    .processes
                    .processes
                    .iter()
                    .filter(|process| process.state == LinuxProcessState::Publishing)
                    .count(),
                published_child_tasks: self
                    .tasks
                    .tasks
                    .iter()
                    .filter(|task| {
                        task.tid != LINUX_ROOT_TID && task.state == LinuxTaskState::Runnable
                    })
                    .count(),
                ready_scheduler_threads: self
                    .scheduler
                    .iter()
                    .filter(|state| **state == HostSchedulerState::Ready)
                    .count(),
                visible_pids,
            }
        }

        fn linux_process_resource_counts(&self) -> HostLinuxProcessResourceCounts {
            let (linux_processes, linux_zombies) = self.processes.resource_counts();
            HostLinuxProcessResourceCounts {
                linux_processes,
                linux_zombies,
                private_pages: self.private_pages.len(),
                shared_pages: self
                    .mapped_pages
                    .iter()
                    .filter(|(_, page)| page.is_shared())
                    .count(),
                page_table_pages: self.table_pages.len(),
            }
        }
    }

    fn host_parent_fork_frame() -> Aarch64ExceptionFrame {
        Aarch64ExceptionFrame {
            regs: core::array::from_fn(|index| 0x1000 + index as u64),
            simd: core::array::from_fn(|index| 0x2000 + index as u128),
            fpcr: 0x3000,
            fpsr: 0x4000,
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct HostForkProcessState {
        process_group: usize,
        credentials: LinuxCredentialsCore,
        cwd: String,
        root: String,
        memory_image: [u8; LINUX_PAGE_SIZE],
        signal_actions: [usize; 2],
        container: LinuxProcessAttributesCore,
        pending_signals: usize,
        aio_requests: usize,
        wait_registrations: usize,
        timers: usize,
    }

    #[derive(Clone, Debug)]
    struct HostForkResult {
        parent_result: usize,
        child: LinuxForkPreparedContext<Aarch64ExceptionFrame>,
        process_state: HostForkProcessState,
    }

    static FORK_FAILPOINT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct HostForkResources {
        resource_clone: Option<LinuxResourceCloneCore<4, 2>>,
        acquired_shared_attachment: bool,
    }

    struct HostForkMemory {
        root_baseline: usize,
        table_baseline: usize,
        private_baseline: usize,
        mapped_baseline: usize,
        memory_baseline: usize,
        pages: Vec<LinuxPageBacking>,
    }

    #[derive(Clone)]
    struct HostForkConfigured {
        child: LinuxForkPreparedContext<Aarch64ExceptionFrame>,
        process_state: HostForkProcessState,
    }

    struct HostForkOps<'a> {
        kernel: &'a mut HostForkKernel,
        parent_bytes: &'a [u8; LINUX_PAGE_SIZE],
    }

    impl<'a> HostForkOps<'a> {
        fn new(kernel: &'a mut HostForkKernel, parent_bytes: &'a [u8; LINUX_PAGE_SIZE]) -> Self {
            Self {
                kernel,
                parent_bytes,
            }
        }
    }

    fn rollback_host_process_resources(kernel: &mut HostForkKernel) {
        let Some(mut child) = kernel.child_resources.take() else {
            return;
        };
        let objects: Vec<u32> = child.objects().iter().copied().rev().collect();
        for description_id in objects {
            assert_eq!(
                child.release_object(description_id, &mut kernel.descriptions),
                None
            );
        }
        let descriptors: Vec<usize> = child
            .descriptors()
            .iter()
            .map(|entry| entry.fd)
            .rev()
            .collect();
        for fd in descriptors {
            assert_eq!(child.close_descriptor(fd, &mut kernel.descriptions), None);
        }
    }

    fn rollback_host_reserved_resources(
        kernel: &mut HostForkKernel,
        mut resources: HostForkResources,
    ) {
        if resources.acquired_shared_attachment {
            kernel.shared_attachment_references -= 1;
        }
        if let Some(resource_clone) = resources.resource_clone.take() {
            assert!(resource_clone
                .rollback(&mut kernel.descriptions)
                .iter()
                .all(Option::is_none));
        }
    }

    struct HostForkPageOps<'a> {
        kernel: &'a mut HostForkKernel,
    }

    impl LinuxForkPageOps for HostForkPageOps<'_> {
        type Page = LinuxPageBacking;
        type Error = ();

        fn failure_error(&self) -> Self::Error {}

        fn is_private(&self, page: Self::Page) -> bool {
            matches!(page, LinuxPageBacking::Private { .. })
        }

        fn allocate_private(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error> {
            if !matches!(parent, LinuxPageBacking::Private { .. }) {
                return Err(());
            }
            let pfn = self
                .kernel
                .private_pages
                .iter()
                .map(|page| page.pfn)
                .max()
                .and_then(|pfn| pfn.checked_add(1))
                .ok_or(())?;
            self.kernel.private_pages.push(HostPrivatePage {
                pfn,
                bytes: [0; LINUX_PAGE_SIZE],
            });
            Ok(LinuxPageBacking::Private { pfn })
        }

        fn copy_private(
            &mut self,
            parent: Self::Page,
            child: Self::Page,
        ) -> Result<(), Self::Error> {
            let LinuxPageBacking::Private { pfn: parent_pfn } = parent else {
                return Err(());
            };
            let LinuxPageBacking::Private { pfn: child_pfn } = child else {
                return Err(());
            };
            let bytes = self
                .kernel
                .private_pages
                .iter()
                .find(|page| page.pfn == parent_pfn)
                .map(|page| page.bytes)
                .ok_or(())?;
            self.kernel
                .private_pages
                .iter_mut()
                .find(|page| page.pfn == child_pfn)
                .ok_or(())?
                .bytes = bytes;
            Ok(())
        }

        fn acquire_shared(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error> {
            let LinuxPageBacking::Shared {
                object_id,
                page_index,
                ..
            } = parent
            else {
                return Err(());
            };
            self.kernel
                .shared_pages
                .acquire(object_id, page_index)
                .then_some(parent)
                .ok_or(())
        }

        fn release_page(&mut self, page: Self::Page) {
            match page {
                LinuxPageBacking::Private { pfn } => {
                    let index = self
                        .kernel
                        .private_pages
                        .iter()
                        .position(|page| page.pfn == pfn)
                        .expect("owned private page");
                    self.kernel.private_pages.remove(index);
                }
                LinuxPageBacking::Shared {
                    object_id,
                    page_index,
                    ..
                } => {
                    assert_eq!(
                        self.kernel.shared_pages.release(object_id, page_index),
                        None
                    );
                }
            }
        }

        fn map_page(
            &mut self,
            address: usize,
            page: Self::Page,
            _prot: usize,
        ) -> Result<(), Self::Error> {
            if self
                .kernel
                .mapped_pages
                .iter()
                .any(|(mapped, _)| *mapped == address)
            {
                return Err(());
            }
            self.kernel.mapped_pages.push((address, page));
            Ok(())
        }

        fn unmap_page(&mut self, address: usize) {
            let index = self
                .kernel
                .mapped_pages
                .iter()
                .position(|(mapped, _)| *mapped == address)
                .expect("mapped fork page");
            self.kernel.mapped_pages.remove(index);
        }
    }

    fn rollback_host_memory(kernel: &mut HostForkKernel, memory: &mut HostForkMemory) {
        kernel.mapped_pages.truncate(memory.mapped_baseline);
        let mut page_ops = HostForkPageOps { kernel };
        for page in memory.pages.drain(..).rev() {
            page_ops.release_page(page);
        }
        kernel.memory_pids.truncate(memory.memory_baseline);
        kernel.private_pages.truncate(memory.private_baseline);
        kernel.table_pages.truncate(memory.table_baseline);
        kernel.address_space_roots.truncate(memory.root_baseline);
    }

    impl LinuxForkOwnershipOps for HostForkOps<'_> {
        type Error = ();
        type Output = HostForkResult;
        type SchedulerThread = usize;
        type Parent = LinuxProcessCore;
        type Task = LinuxTaskReservation;
        type Process = LinuxProcessReservation;
        type Resources = HostForkResources;
        type Memory = HostForkMemory;
        type Configured = HostForkConfigured;
        type Publication = ();

        fn injected_failure(&self) -> Self::Error {}

        fn acquire_scheduler_thread(&mut self) -> Result<Self::SchedulerThread, Self::Error> {
            let slot = self
                .kernel
                .scheduler
                .iter()
                .position(|state| *state == HostSchedulerState::Empty)
                .ok_or(())?;
            self.kernel.scheduler[slot] = HostSchedulerState::Suspended;
            Ok(slot)
        }

        fn acquire_task(
            &mut self,
            scheduler_thread: &Self::SchedulerThread,
        ) -> Result<(Self::Parent, Self::Task), Self::Error> {
            let parent = self.kernel.processes.by_pid(LINUX_ROOT_PID).ok_or(())?;
            let parent_slot = self
                .kernel
                .tasks
                .tasks
                .iter()
                .position(|task| {
                    task.tid == LINUX_ROOT_TID && task.state == LinuxTaskState::Runnable
                })
                .ok_or(())?;
            let parent_mask = self.kernel.tasks.signal_states[parent_slot].mask;
            let task = self
                .kernel
                .tasks
                .reserve_child(0, *scheduler_thread)
                .ok_or(())?;
            self.kernel.tasks.tasks[task.slot].tgid = task.tid;
            self.kernel.tasks.signal_states[task.slot].mask = parent_mask;
            Ok((parent, task))
        }

        fn acquire_process(
            &mut self,
            parent: &Self::Parent,
            scheduler_thread: &Self::SchedulerThread,
            task: &Self::Task,
        ) -> Result<Self::Process, Self::Error> {
            let process = self
                .kernel
                .processes
                .reserve_child_with_pid(parent.pid, *scheduler_thread, task.tid, 17)
                .map_err(|_| ())?;
            assert_eq!(self.kernel.snapshot().visible_pids, vec![LINUX_ROOT_PID]);
            Ok(process)
        }

        fn acquire_resources(
            &mut self,
            _parent: &Self::Parent,
        ) -> Result<Self::Resources, Self::Error> {
            let resource_clone = LinuxResourceCloneCore::reserve_with_failure(
                &self.kernel.parent_resources,
                &mut self.kernel.descriptions,
                || fork_failpoint(LinuxForkFailurePoint::DescriptorReference),
            )
            .ok_or(())?;
            let mut resources = HostForkResources {
                resource_clone: Some(resource_clone),
                acquired_shared_attachment: false,
            };
            let Some(references) = self.kernel.shared_attachment_references.checked_add(1) else {
                rollback_host_reserved_resources(self.kernel, resources);
                return Err(());
            };
            self.kernel.shared_attachment_references = references;
            resources.acquired_shared_attachment = true;
            if fork_failpoint(LinuxForkFailurePoint::SharedReference) {
                rollback_host_reserved_resources(self.kernel, resources);
                return Err(());
            }
            Ok(resources)
        }

        fn acquire_memory(
            &mut self,
            _parent: &Self::Parent,
            process: &Self::Process,
            _resources: &mut Self::Resources,
        ) -> Result<Self::Memory, Self::Error> {
            let memory = HostForkMemory {
                root_baseline: self.kernel.address_space_roots.len(),
                table_baseline: self.kernel.table_pages.len(),
                private_baseline: self.kernel.private_pages.len(),
                mapped_baseline: self.kernel.mapped_pages.len(),
                memory_baseline: self.kernel.memory_pids.len(),
                pages: Vec::new(),
            };
            let mut memory = memory;
            self.kernel.address_space_roots.push(0xa000);
            if fork_failpoint(LinuxForkFailurePoint::ChildRoot) {
                rollback_host_memory(self.kernel, &mut memory);
                return Err(());
            }
            for index in 0..memory.table_baseline {
                self.kernel.table_pages.push(0xb000 + index as u64 * 0x1000);
                if fork_failpoint(LinuxForkFailurePoint::TablePage) {
                    rollback_host_memory(self.kernel, &mut memory);
                    return Err(());
                }
            }
            let private_sources: Vec<LinuxPageBacking> = self.kernel.private_pages
                [..memory.private_baseline]
                .iter()
                .map(|page| LinuxPageBacking::Private { pfn: page.pfn })
                .collect();
            let private_pages = clone_and_map_linux_fork_pages(
                &mut HostForkPageOps {
                    kernel: self.kernel,
                },
                0x1800_0000,
                LINUX_PAGE_SIZE,
                &private_sources,
                LINUX_PROT_READ | LINUX_PROT_WRITE,
                fork_failpoint,
            );
            let private_pages = match private_pages {
                Ok(pages) => pages,
                Err(()) => {
                    rollback_host_memory(self.kernel, &mut memory);
                    return Err(());
                }
            };
            memory.pages.extend(private_pages);
            let shared_sources: Vec<LinuxPageBacking> = self
                .kernel
                .shared_pages
                .pages
                .iter()
                .flatten()
                .map(|page| LinuxPageBacking::Shared {
                    object_id: page.object_id,
                    page_index: page.page_index,
                    pfn: page.pfn,
                })
                .collect();
            let shared_pages = clone_and_map_linux_fork_pages(
                &mut HostForkPageOps {
                    kernel: self.kernel,
                },
                0x1800_2000,
                LINUX_PAGE_SIZE,
                &shared_sources,
                LINUX_PROT_READ | LINUX_PROT_WRITE,
                fork_failpoint,
            );
            let shared_pages = match shared_pages {
                Ok(pages) => pages,
                Err(()) => {
                    rollback_host_memory(self.kernel, &mut memory);
                    return Err(());
                }
            };
            memory.pages.extend(shared_pages);
            self.kernel.memory_pids.push(process.pid);
            Ok(memory)
        }

        fn configure_child(
            &mut self,
            _process: &Self::Process,
            _scheduler_thread: &Self::SchedulerThread,
            _memory: &Self::Memory,
        ) -> Result<Self::Configured, Self::Error> {
            let child = prepare_linux_fork_context(
                host_parent_fork_frame(),
                0x4000,
                0x3c5,
                0x8000,
                0x9000,
                *self.kernel.address_space_roots.last().ok_or(())?,
                |frame| frame.regs[0] = 0,
            );
            let process_state = HostForkProcessState {
                process_group: 1,
                credentials: LinuxCredentialsCore {
                    real_uid: 10,
                    effective_uid: 11,
                    saved_uid: 12,
                    filesystem_uid: 13,
                    real_gid: 20,
                    effective_gid: 21,
                    saved_gid: 22,
                    filesystem_gid: 23,
                }
                .fork_child(),
                cwd: try_clone_linux_fork_path("/parent/cwd").map_err(|_| ())?,
                root: try_clone_linux_fork_path("/parent/root").map_err(|_| ())?,
                memory_image: *self.parent_bytes,
                signal_actions: [0x11, 0x22],
                container: LinuxProcessAttributesCore {
                    namespace_flags: 0x0200_0000,
                    setns_count: 3,
                    mount_count: 2,
                    mount_flags: 0x4000,
                    pivot_rooted: true,
                    chrooted: true,
                    no_new_privs: true,
                    seccomp_mode: 2,
                    seccomp_filters: 4,
                    cap_effective: 0x55,
                    cap_permitted: 0xaa,
                    cap_inheritable: 0x11,
                    hostname_set: true,
                    domainname_set: false,
                }
                .fork_child(0),
                pending_signals: 0,
                aio_requests: 0,
                wait_registrations: 0,
                timers: 0,
            };
            Ok(HostForkConfigured {
                child,
                process_state,
            })
        }

        fn install_resources(
            &mut self,
            _process: &Self::Process,
            resources: &mut Option<Self::Resources>,
        ) -> Result<(), Self::Error> {
            if self.kernel.child_resources.is_some() {
                return Err(());
            }
            let mut child = LinuxProcessResourceCore::<4, 2>::new();
            let resource_clone = resources
                .as_mut()
                .and_then(|resources| resources.resource_clone.take())
                .ok_or(())?;
            if !resource_clone.commit(&mut child) {
                return Err(());
            }
            self.kernel.child_resources = Some(child);
            Ok(())
        }

        fn begin_publication(&mut self) -> Result<Self::Publication, Self::Error> {
            Ok(())
        }

        fn publish_process(
            &mut self,
            process: &Self::Process,
            _configured: &Self::Configured,
        ) -> Result<(), Self::Error> {
            self.kernel
                .processes
                .publish_fork(*process)
                .then_some(())
                .ok_or(())
        }

        fn publish_task(&mut self, task: &Self::Task) -> Result<(), Self::Error> {
            self.kernel.tasks.publish(*task).then_some(()).ok_or(())
        }

        fn publish_scheduler_thread(
            &mut self,
            scheduler_thread: &Self::SchedulerThread,
        ) -> Result<(), Self::Error> {
            if self.kernel.scheduler[*scheduler_thread] != HostSchedulerState::Suspended {
                return Err(());
            }
            self.kernel.scheduler[*scheduler_thread] = HostSchedulerState::Ready;
            Ok(())
        }

        fn complete_publication(&mut self, process: &Self::Process) -> Result<(), Self::Error> {
            self.kernel
                .processes
                .complete_fork_publish(*process)
                .then_some(())
                .ok_or(())
        }

        fn finish(
            &mut self,
            process: &Self::Process,
            configured: &Self::Configured,
        ) -> Result<Self::Output, Self::Error> {
            Ok(HostForkResult {
                parent_result: process.pid,
                child: configured.child,
                process_state: configured.process_state.clone(),
            })
        }

        fn restore_publication(&mut self, _publication: Self::Publication) {}

        fn rollback_configured(&mut self, _configured: Self::Configured) {}

        fn rollback_memory(&mut self, mut memory: Self::Memory) {
            rollback_host_memory(self.kernel, &mut memory);
        }

        fn rollback_reserved_resources(&mut self, resources: Self::Resources) {
            rollback_host_reserved_resources(self.kernel, resources);
        }

        fn rollback_installed_resources(&mut self, _process: &Self::Process) {
            rollback_host_process_resources(self.kernel);
        }

        fn rollback_process(&mut self, process: Self::Process) {
            assert!(self.kernel.processes.rollback_fork(process));
        }

        fn rollback_task(&mut self, task: Self::Task) {
            assert!(self.kernel.tasks.rollback(task));
        }

        fn rollback_scheduler_thread(&mut self, scheduler_thread: Self::SchedulerThread) {
            self.kernel.scheduler[scheduler_thread] = HostSchedulerState::Empty;
        }
    }

    fn run_shared_adapter_fork(
        parent_bytes: &[u8; LINUX_PAGE_SIZE],
        kernel: &mut HostForkKernel,
    ) -> Result<HostForkResult, ()> {
        run_linux_fork_transaction(host_fork_backend(kernel, parent_bytes), fork_failpoint)
    }

    fn host_fork_backend<'a>(
        kernel: &'a mut HostForkKernel,
        parent_bytes: &'a [u8; LINUX_PAGE_SIZE],
    ) -> LinuxForkOwnershipCore<HostForkOps<'a>> {
        LinuxForkOwnershipCore::new(HostForkOps::new(kernel, parent_bytes))
    }

    fn host_failure_occurrences(
        parent: &[u8; LINUX_PAGE_SIZE],
        point: LinuxForkFailurePoint,
    ) -> usize {
        for occurrence in 0..32 {
            let mut kernel = HostForkKernel::parent(parent);
            let baseline = kernel.snapshot();
            configure_fork_failure(point, occurrence);
            let result = run_shared_adapter_fork(parent, &mut kernel);
            clear_fork_failure();
            if result.is_ok() {
                return occurrence;
            }
            assert_eq!(kernel.snapshot(), baseline);
        }
        panic!("unbounded failpoint occurrences for {point:?}");
    }

    #[test]
    fn host_failpoints_cover_each_private_and_shared_page_operation() {
        let _guard = FORK_FAILPOINT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let parent = core::array::from_fn(|index| (index as u8).wrapping_mul(37));

        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::PrivatePage),
            6,
            "two private pages require allocation, copy, and map boundaries"
        );
        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::PrivatePageAllocation),
            2,
            "each private allocation has its own boundary"
        );
        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::PrivatePageCopy),
            2,
            "each complete private-page copy has its own boundary"
        );
        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::PrivatePageMap),
            2,
            "each private-page map has its own boundary"
        );
        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::SharedReference),
            5,
            "one attachment plus two shared references and maps require boundaries"
        );
        assert_eq!(
            host_failure_occurrences(&parent, LinuxForkFailurePoint::SharedPageMap),
            2,
            "each shared-page map has its own boundary"
        );
    }

    #[test]
    fn host_failpoints_execute_the_production_ownership_core() {
        let parent = [0xa5; LINUX_PAGE_SIZE];
        let mut kernel = HostForkKernel::parent(&parent);
        let backend = host_fork_backend(&mut kernel, &parent);

        assert!(
            core::any::type_name_of_val(&backend).contains("LinuxForkOwnershipCore"),
            "host tests must instantiate the production ownership core"
        );
    }

    #[test]
    fn shared_transaction_failpoints_restore_every_authoritative_core_baseline() {
        let _guard = FORK_FAILPOINT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let points = [
            LinuxForkFailurePoint::SchedulerThread,
            LinuxForkFailurePoint::Task,
            LinuxForkFailurePoint::Process,
            LinuxForkFailurePoint::DescriptorReference,
            LinuxForkFailurePoint::SharedReference,
            LinuxForkFailurePoint::ChildRoot,
            LinuxForkFailurePoint::TablePage,
            LinuxForkFailurePoint::PrivatePage,
            LinuxForkFailurePoint::PrivatePageAllocation,
            LinuxForkFailurePoint::PrivatePageCopy,
            LinuxForkFailurePoint::PrivatePageMap,
            LinuxForkFailurePoint::SharedPageMap,
            LinuxForkFailurePoint::Memory,
            LinuxForkFailurePoint::Configured,
            LinuxForkFailurePoint::ProcessPublication,
            LinuxForkFailurePoint::TaskPublication,
            LinuxForkFailurePoint::SchedulerPublication,
        ];
        let parent = core::array::from_fn(|index| (index as u8).wrapping_mul(37));
        let parent_snapshot = parent;

        for point in points {
            let mut occurrence = 0;
            loop {
                let mut kernel = HostForkKernel::parent(&parent);
                let baseline = kernel.snapshot();
                configure_fork_failure(point, occurrence);
                let result = run_shared_adapter_fork(&parent, &mut kernel);
                clear_fork_failure();
                if result.is_ok() {
                    assert_ne!(occurrence, 0, "failpoint {point:?} was never reached");
                    break;
                }
                assert_eq!(parent, parent_snapshot, "parent changed after {point:?}");
                assert_eq!(
                    kernel.snapshot(),
                    baseline,
                    "resource leak after {point:?} occurrence {occurrence}"
                );
                assert_eq!(kernel.snapshot().visible_pids, vec![LINUX_ROOT_PID]);
                occurrence += 1;
                assert!(
                    occurrence < 16,
                    "unbounded failpoint occurrences for {point:?}"
                );
            }
        }
    }

    #[test]
    fn successful_shared_adapter_fork_copies_the_complete_child_context() {
        let _guard = FORK_FAILPOINT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_fork_failure();
        let parent = core::array::from_fn(|index| (index as u8).wrapping_mul(37));
        let mut kernel = HostForkKernel::parent(&parent);
        let result = run_shared_adapter_fork(&parent, &mut kernel).expect("fork succeeds");

        let mut expected_frame = host_parent_fork_frame();
        expected_frame.regs[0] = 0;
        assert_eq!(result.parent_result, 2);
        assert_eq!(result.child.frame.regs, expected_frame.regs);
        assert_eq!(result.child.frame.simd, expected_frame.simd);
        assert_eq!(result.child.frame.fpcr, expected_frame.fpcr);
        assert_eq!(result.child.frame.fpsr, expected_frame.fpsr);
        assert_eq!(result.child.return_pc, 0x4000);
        assert_eq!(result.child.pstate, 0x3c5);
        assert_eq!(result.child.user_sp, 0x8000);
        assert_eq!(result.child.tls, 0x9000);
        assert_eq!(result.child.root_paddr, 0xa000);
        assert_ne!(result.child.root_paddr, 0x1000);
        assert_eq!(kernel.snapshot().visible_pids, vec![1, 2]);
        assert_eq!(result.process_state.process_group, 1);
        assert_eq!(result.process_state.credentials.effective_uid, 11);
        assert_eq!(result.process_state.credentials.filesystem_gid, 23);
        assert_eq!(result.process_state.cwd, "/parent/cwd");
        assert_eq!(result.process_state.root, "/parent/root");
        assert_eq!(result.process_state.memory_image, parent);
        assert_eq!(result.process_state.signal_actions, [0x11, 0x22]);
        assert!(result.process_state.container.no_new_privs);
        assert_eq!(result.process_state.container.seccomp_mode, 2);
        assert_eq!(result.process_state.pending_signals, 0);
        assert_eq!(result.process_state.aio_requests, 0);
        assert_eq!(result.process_state.wait_registrations, 0);
        assert_eq!(result.process_state.timers, 0);

        let child_private_pages = &kernel.private_pages[2..];
        assert_eq!(child_private_pages.len(), 2);
        assert!(![17, 18].contains(&child_private_pages[0].pfn));
        assert!(![17, 18].contains(&child_private_pages[1].pfn));
        assert_ne!(child_private_pages[0].pfn, child_private_pages[1].pfn);
        assert_eq!(child_private_pages[0].bytes, parent);
        assert_eq!(
            child_private_pages[1].bytes,
            core::array::from_fn(|index| parent[LINUX_PAGE_SIZE - 1 - index])
        );
        assert_eq!(
            &kernel.mapped_pages[4..],
            &[
                (
                    0x1800_0000,
                    LinuxPageBacking::Private {
                        pfn: child_private_pages[0].pfn,
                    },
                ),
                (
                    0x1800_1000,
                    LinuxPageBacking::Private {
                        pfn: child_private_pages[1].pfn,
                    },
                ),
                (
                    0x1800_2000,
                    LinuxPageBacking::Shared {
                        object_id: 9,
                        page_index: 0,
                        pfn: 31,
                    },
                ),
                (
                    0x1800_3000,
                    LinuxPageBacking::Shared {
                        object_id: 9,
                        page_index: 1,
                        pfn: 32,
                    },
                ),
            ]
        );
    }

    #[test]
    fn exit_wait_lifecycle_snapshots_release_memory_before_one_time_reap() {
        let _guard = FORK_FAILPOINT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_fork_failure();
        let parent = core::array::from_fn(|index| (index as u8).wrapping_mul(37));
        let mut kernel = HostForkKernel::parent(&parent);
        assert_eq!(
            kernel.linux_process_resource_counts(),
            HostLinuxProcessResourceCounts {
                linux_processes: 1,
                linux_zombies: 0,
                private_pages: 2,
                shared_pages: 2,
                page_table_pages: 3,
            }
        );

        let child_pid = run_shared_adapter_fork(&parent, &mut kernel)
            .expect("fork succeeds")
            .parent_result;
        assert_eq!(
            kernel.linux_process_resource_counts(),
            HostLinuxProcessResourceCounts {
                linux_processes: 2,
                linux_zombies: 0,
                private_pages: 4,
                shared_pages: 4,
                page_table_pages: 6,
            }
        );

        let child_task = kernel.tasks.by_tid(child_pid).expect("child task");
        assert_eq!(
            kernel
                .tasks
                .exit_with_clear_child_tid(child_task.tid, child_task.scheduler_thread),
            Some(0)
        );
        assert!(kernel
            .tasks
            .retire(child_task.tid, child_task.scheduler_thread));
        kernel.scheduler[child_task.scheduler_thread] = HostSchedulerState::Empty;
        let child_pages: Vec<LinuxPageBacking> = kernel
            .mapped_pages
            .split_off(4)
            .into_iter()
            .map(|(_, page)| page)
            .collect();
        let mut page_ops = HostForkPageOps {
            kernel: &mut kernel,
        };
        for page in child_pages.into_iter().rev() {
            page_ops.release_page(page);
        }
        kernel.memory_pids.retain(|pid| *pid != child_pid);
        kernel.address_space_roots.truncate(1);
        kernel.table_pages.truncate(3);
        kernel.shared_attachment_references -= 1;
        rollback_host_process_resources(&mut kernel);

        let status = linux_wait_status_exit(37);
        assert!(kernel.processes.exit(child_pid, status));
        let exited = HostLinuxProcessResourceCounts {
            linux_processes: 1,
            linux_zombies: 1,
            private_pages: 2,
            shared_pages: 2,
            page_table_pages: 3,
        };
        assert_eq!(kernel.linux_process_resource_counts(), exited);
        assert_eq!(
            kernel
                .processes
                .wait_outcome(LINUX_ROOT_PID, LinuxWaitSelector::Pid(child_pid)),
            LinuxWaitOutcome::Ready {
                pid: child_pid,
                status,
            }
        );
        assert_eq!(
            kernel.linux_process_resource_counts(),
            exited,
            "failed status copyout must leave the selected zombie waitable"
        );

        assert!(kernel.processes.reap(LINUX_ROOT_PID, child_pid).is_some());
        assert_eq!(
            kernel.linux_process_resource_counts(),
            HostLinuxProcessResourceCounts {
                linux_processes: 1,
                linux_zombies: 0,
                private_pages: 2,
                shared_pages: 2,
                page_table_pages: 3,
            }
        );
        assert_eq!(kernel.processes.reap(LINUX_ROOT_PID, child_pid), None);
    }

    #[test]
    fn forked_credentials_are_inherited_then_mutated_per_process() {
        let parent = LinuxCredentialsCore {
            real_uid: 10,
            effective_uid: 11,
            saved_uid: 12,
            filesystem_uid: 13,
            real_gid: 20,
            effective_gid: 21,
            saved_gid: 22,
            filesystem_gid: 23,
        };
        let mut child = parent.fork_child();

        child.set_resuid(30, 31, 32);
        child.set_resgid(40, 41, 42);
        child.set_filesystem_uid(33);
        child.set_filesystem_gid(43);

        assert_eq!(parent.real_uid, 10);
        assert_eq!(parent.effective_gid, 21);
        assert_eq!(child.real_uid, 30);
        assert_eq!(child.effective_uid, 31);
        assert_eq!(child.saved_uid, 32);
        assert_eq!(child.filesystem_uid, 33);
        assert_eq!(child.real_gid, 40);
        assert_eq!(child.effective_gid, 41);
        assert_eq!(child.saved_gid, 42);
        assert_eq!(child.filesystem_gid, 43);
    }

    #[test]
    fn forked_cwd_and_root_paths_are_independent_allocations() {
        let parent_cwd = String::from("/parent/cwd");
        let parent_root = String::from("/parent/root");
        let mut child_cwd = try_clone_linux_fork_path(&parent_cwd).expect("clone cwd");
        let mut child_root = try_clone_linux_fork_path(&parent_root).expect("clone root");

        child_cwd.push_str("/child");
        child_root.clear();
        child_root.push('/');

        assert_eq!(parent_cwd, "/parent/cwd");
        assert_eq!(parent_root, "/parent/root");
        assert_eq!(child_cwd, "/parent/cwd/child");
        assert_eq!(child_root, "/");
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

    #[test]
    fn empty_peer_queue_read_reports_wait_only_while_peer_is_open() {
        const WOULD_WAIT: i32 = -31;
        const PEER_CLOSED: i32 = -34;

        assert_eq!(
            smros_ko_empty_peer_queue_read_error_body!(true, WOULD_WAIT, PEER_CLOSED),
            WOULD_WAIT
        );
        assert_eq!(
            smros_ko_empty_peer_queue_read_error_body!(false, WOULD_WAIT, PEER_CLOSED),
            PEER_CLOSED
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

    pub(crate) fn mmio_addr(base: usize, offset: usize) -> Option<usize> {
        smros_ll_mmio_addr_body!(base, offset)
    }

    pub(crate) fn dt_reg_valid(base: usize, size: usize) -> bool {
        smros_ll_dt_reg_valid_body!(base, size)
    }

    pub(crate) fn dt_irq_valid(irq: u32, max_irqs: u32) -> bool {
        smros_ll_dt_irq_valid_body!(irq, max_irqs)
    }

    pub(crate) fn dt_platform_index(
        candidate: usize,
        platform_count: usize,
        fallback: usize,
    ) -> usize {
        smros_ll_dt_platform_index_body!(candidate, platform_count, fallback)
    }

    pub(crate) fn fdt_range_valid(offset: usize, len: usize, total: usize) -> bool {
        smros_ll_fdt_range_valid_body!(offset, len, total)
    }

    pub(crate) fn fdt_align4(offset: usize) -> Option<usize> {
        smros_ll_fdt_align4_body!(offset)
    }

    pub(crate) fn fdt_cells_to_bytes(cells: usize) -> Option<usize> {
        smros_ll_fdt_cells_to_bytes_body!(cells)
    }

    pub(crate) fn fdt_reg_tuple_bytes(address_cells: usize, size_cells: usize) -> Option<usize> {
        smros_ll_fdt_reg_tuple_bytes_body!(address_cells, size_cells)
    }

    pub(crate) fn fdt_reg_tuple_offset(
        index: usize,
        address_cells: usize,
        size_cells: usize,
    ) -> Option<usize> {
        smros_ll_fdt_reg_tuple_offset_body!(index, address_cells, size_cells)
    }

    pub(crate) fn dt_gic_irq(kind: u32, hwirq: u32, max_irqs: u32) -> Option<u32> {
        smros_ll_dt_gic_irq_body!(kind, hwirq, max_irqs)
    }

    pub(crate) fn dt_timer_irq_index(entry_count: usize) -> usize {
        smros_ll_dt_timer_irq_index_body!(entry_count)
    }

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

    #[test]
    fn kernel_lowlevel_logic_detected_ram_overrides_fallback_and_rejects_invalid_ranges() {
        assert_eq!(
            memory_reg(
                Some((0x8000_0000usize, 0x4000_0000usize)),
                0x4000_0000usize,
                0x2000_0000usize
            ),
            Some((0x8000_0000, 0x4000_0000))
        );
        assert_eq!(
            memory_reg(None, 0x4000_0000usize, 0x2000_0000usize),
            Some((0x4000_0000, 0x2000_0000))
        );
        assert_eq!(
            memory_reg(
                Some((0x8000_0000usize, 0usize)),
                0x4000_0000usize,
                0x2000_0000usize
            ),
            None
        );
        assert_eq!(
            memory_reg(
                Some((usize::MAX - 0xfff, 0x2000usize)),
                0x4000_0000usize,
                0x2000_0000usize
            ),
            None
        );
    }

    #[test]
    fn kernel_lowlevel_logic_allocator_init_range_checks_alignment_capacity_and_reset() {
        const PAGE_SIZE: usize = 4096;
        const MAX_RAM_BYTES: usize = 2 * 1024 * 1024 * 1024;
        const MAX_BITMAP_WORDS: usize = (MAX_RAM_BYTES / PAGE_SIZE) / 64;

        let mut allocator = PageFrameAllocatorCore::<2>::new(4);
        assert!(!allocator.init_range(0x8001, 0xc000, PAGE_SIZE));
        assert!(!allocator.init_range(0x8000, 0xc001, PAGE_SIZE));
        assert!(!allocator.init_range(0x8000, 0x8000, PAGE_SIZE));
        assert!(allocator.init_range(0x8000, 0xc000, PAGE_SIZE));
        assert_eq!(allocator.total_pages(), 4);
        assert_eq!(allocator.alloc(), Some(8));
        assert_eq!(allocator.allocated_pages(), 1);

        assert!(allocator.init_range(0x1_0000, 0x1_2000, PAGE_SIZE));
        assert_eq!(allocator.total_pages(), 2);
        assert_eq!(allocator.allocated_pages(), 0);
        assert_eq!(allocator.free_pages(), 2);
        assert_eq!(allocator.alloc(), Some((0x1_0000 / PAGE_SIZE) as u64));

        let mut maximum = PageFrameAllocatorCore::<MAX_BITMAP_WORDS>::new(0);
        assert!(maximum.init_range(0x4000_0000, 0xc000_0000, PAGE_SIZE));
        assert_eq!(maximum.total_pages(), MAX_RAM_BYTES / PAGE_SIZE);
        assert!(!maximum.init_range(0x4000_0000, 0xc000_1000, PAGE_SIZE));
        assert_eq!(maximum.total_pages(), MAX_RAM_BYTES / PAGE_SIZE);
    }

    #[test]
    fn kernel_lowlevel_logic_allocator_stays_inside_its_physical_range() {
        const PAGE_SIZE: usize = 4096;
        let start = 0x4fb0_8000usize;
        let end = start + 3 * PAGE_SIZE;
        let first_pfn = (start / PAGE_SIZE) as u64;
        let mut allocator = PageFrameAllocatorCore::<1>::new(0);

        assert!(allocator.init_range(start, end, PAGE_SIZE));
        assert_eq!(allocator.pfn_address(first_pfn - 1, PAGE_SIZE), None);
        assert_eq!(allocator.pfn_address(first_pfn, PAGE_SIZE), Some(start));
        assert_eq!(
            allocator.pfn_address(first_pfn + 2, PAGE_SIZE),
            Some(end - PAGE_SIZE)
        );
        assert_eq!(allocator.pfn_address(first_pfn + 3, PAGE_SIZE), None);

        assert_eq!(allocator.alloc(), Some(first_pfn));
        assert_eq!(allocator.alloc(), Some(first_pfn + 1));
        assert_eq!(allocator.alloc(), Some(first_pfn + 2));
        assert_eq!(allocator.alloc(), None);
    }

    #[test]
    fn kernel_lowlevel_logic_allocator_reuses_the_first_freed_pfn() {
        const PAGE_SIZE: usize = 4096;
        let start = 0x5000_0000usize;
        let first_pfn = (start / PAGE_SIZE) as u64;
        let mut allocator = PageFrameAllocatorCore::<1>::new(0);

        assert!(allocator.init_range(start, start + 2 * PAGE_SIZE, PAGE_SIZE));
        assert_eq!(allocator.alloc(), Some(first_pfn));
        assert_eq!(allocator.alloc(), Some(first_pfn + 1));
        assert!(allocator.free(first_pfn));
        assert_eq!(allocator.alloc(), Some(first_pfn));
        assert_eq!(allocator.allocated_pages(), 2);
    }

    #[test]
    fn physical_pfn_offsets_round_trip_within_ram_range() {
        const PAGE_SIZE: usize = 4096;
        let start = 0x4fb0_8000usize;
        let end = 0x6000_0000usize;
        let base_pfn = (start / PAGE_SIZE) as u64;
        let total_pages = (end - start) / PAGE_SIZE;

        assert_eq!(
            smros_ll_pfn_from_index_body!(0usize, base_pfn),
            Some(base_pfn)
        );
        assert_eq!(
            smros_ll_pfn_index_body!(base_pfn, base_pfn, total_pages),
            Some(0)
        );
        assert_eq!(
            smros_ll_pfn_address_body!(base_pfn, base_pfn, total_pages, PAGE_SIZE),
            Some(start)
        );
        assert_eq!(
            smros_ll_pfn_index_body!(base_pfn - 1, base_pfn, total_pages),
            None
        );
        assert_eq!(
            smros_ll_pfn_address_body!(
                base_pfn + total_pages as u64,
                base_pfn,
                total_pages,
                PAGE_SIZE
            ),
            None
        );
    }
}

#[cfg(test)]
mod kernel_lowlevel {
    pub(crate) mod serial {
        pub(crate) struct Serial;

        impl Serial {
            pub(crate) fn write_str(&mut self, _value: &str) {}

            pub(crate) fn write_hex(&mut self, _value: u64) {}

            pub(crate) fn write_byte(&mut self, _value: u8) {}
        }
    }
}

#[path = "../../../src/kernel_lowlevel/ARM64/drivers.rs"]
#[cfg(test)]
mod aarch64_drivers;

#[cfg(test)]
fn fdt_push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
fn fdt_pad(bytes: &mut Vec<u8>) {
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
}

#[cfg(test)]
fn fdt_begin_node(structure: &mut Vec<u8>, name: &str) {
    fdt_push_u32(structure, 1);
    structure.extend_from_slice(name.as_bytes());
    structure.push(0);
    fdt_pad(structure);
}

#[cfg(test)]
fn fdt_string_offset(strings: &[u8], wanted: &str) -> u32 {
    let mut offset = 0usize;
    while offset < strings.len() {
        let end = strings[offset..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|len| offset + len)
            .expect("terminated FDT property name");
        if &strings[offset..end] == wanted.as_bytes() {
            return offset as u32;
        }
        offset = end + 1;
    }
    panic!("missing FDT property name: {wanted}");
}

#[cfg(test)]
fn fdt_property(structure: &mut Vec<u8>, strings: &[u8], name: &str, value: &[u8]) {
    fdt_push_u32(structure, 3);
    fdt_push_u32(structure, value.len() as u32);
    fdt_push_u32(structure, fdt_string_offset(strings, name));
    structure.extend_from_slice(value);
    fdt_pad(structure);
}

#[cfg(test)]
fn fdt_cells(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        fdt_push_u32(&mut bytes, *value);
    }
    bytes
}

#[cfg(test)]
fn fdt_reg_cells(base: u64, size: u64) -> [u32; 4] {
    [
        (base >> 32) as u32,
        base as u32,
        (size >> 32) as u32,
        size as u32,
    ]
}

#[cfg(test)]
fn test_qemu_fdt(memory_base: u64, memory_size: u64) -> Vec<u32> {
    const HEADER_SIZE: usize = 40;
    const RESERVE_MAP_SIZE: usize = 16;
    let strings = b"compatible\0#address-cells\0#size-cells\0device_type\0reg\0interrupts\0";
    let mut structure = Vec::new();

    fdt_begin_node(&mut structure, "");
    fdt_property(&mut structure, strings, "compatible", b"linux,dummy-virt\0");
    fdt_property(&mut structure, strings, "#address-cells", &fdt_cells(&[2]));
    fdt_property(&mut structure, strings, "#size-cells", &fdt_cells(&[2]));

    fdt_begin_node(&mut structure, "memory@80000000");
    fdt_property(&mut structure, strings, "device_type", b"memory\0");
    fdt_property(
        &mut structure,
        strings,
        "reg",
        &fdt_cells(&fdt_reg_cells(memory_base, memory_size)),
    );
    fdt_push_u32(&mut structure, 2);

    fdt_begin_node(&mut structure, "pl011@9000000");
    fdt_property(&mut structure, strings, "compatible", b"arm,pl011\0");
    fdt_property(
        &mut structure,
        strings,
        "reg",
        &fdt_cells(&fdt_reg_cells(0x0900_0000, 0x1000)),
    );
    fdt_property(
        &mut structure,
        strings,
        "interrupts",
        &fdt_cells(&[0, 1, 4]),
    );
    fdt_push_u32(&mut structure, 2);

    fdt_begin_node(&mut structure, "intc@8000000");
    fdt_property(&mut structure, strings, "compatible", b"arm,gic-v3\0");
    let mut gic_reg = Vec::new();
    gic_reg.extend_from_slice(&fdt_reg_cells(0x0800_0000, 0x1_0000));
    gic_reg.extend_from_slice(&fdt_reg_cells(0x080a_0000, 0x00f6_0000));
    fdt_property(&mut structure, strings, "reg", &fdt_cells(&gic_reg));
    fdt_push_u32(&mut structure, 2);

    fdt_begin_node(&mut structure, "timer");
    fdt_property(&mut structure, strings, "compatible", b"arm,armv8-timer\0");
    fdt_property(
        &mut structure,
        strings,
        "interrupts",
        &fdt_cells(&[0, 14, 4]),
    );
    fdt_push_u32(&mut structure, 2);

    fdt_push_u32(&mut structure, 2);
    fdt_push_u32(&mut structure, 9);

    let structure_offset = HEADER_SIZE + RESERVE_MAP_SIZE;
    let strings_offset = structure_offset + structure.len();
    let mut bytes = vec![0; structure_offset];
    bytes.extend_from_slice(&structure);
    bytes.extend_from_slice(strings);
    fdt_pad(&mut bytes);

    let header = [
        0xd00d_feed,
        bytes.len() as u32,
        structure_offset as u32,
        strings_offset as u32,
        HEADER_SIZE as u32,
        17,
        16,
        0,
        strings.len() as u32,
        structure.len() as u32,
    ];
    for (index, value) in header.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }

    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("four-byte FDT word")))
        .collect()
}

#[test]
fn kernel_lowlevel_logic_real_fdt_memory_overrides_fallback_and_rejects_invalid_ranges() {
    use aarch64_drivers::{ResourceSource, QEMU_VIRT_MEMORY_BASE, QEMU_VIRT_MEMORY_SIZE};

    let detected_base = 0x8000_0000usize;
    let detected_size = 0x4000_0000usize;
    let detected = test_qemu_fdt(detected_base as u64, detected_size as u64);
    assert!(aarch64_drivers::init_from_fdt(detected.as_ptr() as usize));
    let stats = aarch64_drivers::stats();
    assert_eq!(stats.source, ResourceSource::Fdt);
    assert_eq!(stats.memory_base, detected_base);
    assert_eq!(stats.memory_size, detected_size);
    assert_ne!(stats.memory_size, QEMU_VIRT_MEMORY_SIZE);

    let zero_size = test_qemu_fdt(detected_base as u64, 0);
    assert!(aarch64_drivers::init_from_fdt(zero_size.as_ptr() as usize));
    let stats = aarch64_drivers::stats();
    assert_eq!(stats.source, ResourceSource::StaticFallback);
    assert_eq!(stats.memory_base, QEMU_VIRT_MEMORY_BASE);
    assert_eq!(stats.memory_size, QEMU_VIRT_MEMORY_SIZE);

    let overflowing = test_qemu_fdt((usize::MAX - 0xfff) as u64, 0x2000);
    assert!(aarch64_drivers::init_from_fdt(overflowing.as_ptr() as usize));
    let stats = aarch64_drivers::stats();
    assert_eq!(stats.source, ResourceSource::StaticFallback);
    assert_eq!(stats.memory_base, QEMU_VIRT_MEMORY_BASE);
    assert_eq!(stats.memory_size, QEMU_VIRT_MEMORY_SIZE);

    let base_zero_size = aarch64_drivers::RPI4_MEMORY_SIZE;
    let base_zero = test_qemu_fdt(0, base_zero_size as u64);
    assert!(aarch64_drivers::init_from_fdt(base_zero.as_ptr() as usize));
    assert_eq!(aarch64_drivers::stats().source, ResourceSource::Fdt);
    let memory = aarch64_drivers::memory_reg().expect("base-zero FDT RAM remains valid");
    assert_eq!(memory.base, 0);
    assert_eq!(memory.size, base_zero_size);
}

mod aarch64_exception_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/aarch64_exception_logic_shared.rs"
    ));

    fn esr(ec: u64, iss: u64) -> u64 {
        (ec << 26) | iss
    }

    #[test]
    fn lower_el_abort_decoder_preserves_access_and_fault_kind() {
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, 0x07)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Read,
                kind: Aarch64El0AbortKind::Translation,
            })
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, (1 << 6) | 0x0f)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Write,
                kind: Aarch64El0AbortKind::Permission,
            })
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x20, 0x09)),
            Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault {
                access: Aarch64El0MemoryAccess::Execute,
                kind: Aarch64El0AbortKind::AccessFlag,
            })
        );
    }

    #[test]
    fn decoder_separates_svc_and_unsupported_lower_el_exceptions() {
        assert_eq!(aarch64_lower_el_sync(esr(0x15, 0)), Aarch64LowerElSync::Svc);
        assert_eq!(
            aarch64_lower_el_sync(esr(0x24, 0x21)),
            Aarch64LowerElSync::Unsupported
        );
        assert_eq!(
            aarch64_lower_el_sync(esr(0x3c, 0)),
            Aarch64LowerElSync::Unsupported
        );
    }
}

mod aarch64_vm_logic {
    extern crate alloc;

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_lowlevel/aarch64_vm_logic_shared.rs"
    ));

    #[test]
    fn three_level_indices_distinguish_adjacent_user_pages() {
        assert_eq!(aarch64_table_indices(0x1000_0000), Some([0, 128, 0]));
        assert_eq!(aarch64_table_indices(0x1000_1000), Some([0, 128, 1]));
        assert_eq!(aarch64_table_indices(0x4000_0000), Some([1, 0, 0]));
        assert_eq!(aarch64_table_indices(1usize << 39), None);
    }

    #[test]
    fn descriptors_encode_tables_blocks_pages_and_permissions() {
        let paddr = 0x1234_5678_9000;
        assert_eq!(
            aarch64_table_descriptor(paddr),
            (paddr as u64 & AARCH64_DESC_ADDR_MASK)
                | AARCH64_DESC_VALID
                | AARCH64_DESC_TABLE_OR_PAGE
        );

        let supervisor = aarch64_supervisor_block_descriptor(paddr, false, true);
        assert_eq!(supervisor & AARCH64_DESC_TABLE_OR_PAGE, 0);
        assert_eq!(supervisor & AARCH64_DESC_AF, AARCH64_DESC_AF);
        assert_eq!(
            supervisor & AARCH64_DESC_INNER_SHAREABLE,
            AARCH64_DESC_INNER_SHAREABLE
        );
        let device = aarch64_supervisor_block_descriptor(paddr, true, false);
        assert_eq!(device & (1 << 2), 1 << 2);
        assert_eq!(
            device & AARCH64_DESC_INNER_SHAREABLE,
            AARCH64_DESC_INNER_SHAREABLE
        );
        assert_eq!(
            device & (AARCH64_DESC_PXN | AARCH64_DESC_UXN),
            AARCH64_DESC_PXN | AARCH64_DESC_UXN
        );

        let read_only = aarch64_user_page_descriptor(paddr, true, false, true);
        assert_eq!(read_only & AARCH64_DESC_AP_USER, AARCH64_DESC_AP_USER);
        assert_eq!(
            read_only & AARCH64_DESC_AP_READ_ONLY,
            AARCH64_DESC_AP_READ_ONLY
        );
        assert_eq!(read_only & AARCH64_DESC_UXN, 0);
        assert_eq!(read_only & AARCH64_DESC_PXN, AARCH64_DESC_PXN);

        let read_write = aarch64_user_page_descriptor(paddr, true, true, false);
        assert_eq!(read_write & AARCH64_DESC_AP_READ_ONLY, 0);
        assert_eq!(read_write & AARCH64_DESC_UXN, AARCH64_DESC_UXN);
        assert_eq!(
            read_write & AARCH64_DESC_TABLE_OR_PAGE,
            AARCH64_DESC_TABLE_OR_PAGE
        );

        let no_access = aarch64_user_page_descriptor(paddr, false, false, false);
        assert_eq!(no_access & AARCH64_DESC_AP_USER, 0);
        assert_eq!(
            no_access & AARCH64_DESC_AP_READ_ONLY,
            AARCH64_DESC_AP_READ_ONLY
        );
    }

    #[test]
    fn execute_only_user_page_is_el0_accessible() {
        let descriptor = aarch64_user_page_descriptor(0x1234_5678_9000, false, false, true);
        assert_eq!(descriptor & AARCH64_DESC_AP_USER, AARCH64_DESC_AP_USER);
        assert_eq!(
            descriptor & AARCH64_DESC_AP_READ_ONLY,
            AARCH64_DESC_AP_READ_ONLY
        );
        assert_eq!(descriptor & AARCH64_DESC_UXN, 0);
        assert_eq!(descriptor & AARCH64_DESC_PXN, AARCH64_DESC_PXN);
    }

    #[test]
    fn executable_supervisor_blocks_remain_execute_never_at_el0() {
        let descriptor = aarch64_supervisor_block_descriptor(0x4020_0000, false, true);
        assert_eq!(descriptor & AARCH64_DESC_PXN, 0);
        assert_eq!(descriptor & AARCH64_DESC_UXN, AARCH64_DESC_UXN);
    }

    #[test]
    fn supervisor_block_output_is_two_mib_aligned() {
        let paddr = 0x1234_5678_9000usize;
        let descriptor = aarch64_supervisor_block_descriptor(paddr, false, true);
        assert_eq!(
            descriptor & AARCH64_DESC_ADDR_MASK,
            paddr as u64 & 0x0000_ffff_ffe0_0000
        );
        assert_eq!(descriptor & 0x001f_f000, 0);
    }

    #[test]
    fn three_level_model_maps_exact_pages_with_independent_roots() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut first = Aarch64AddressSpaceModel::new(&mut allocator).expect("first root");
        let second = Aarch64AddressSpaceModel::new(&mut allocator).expect("second root");
        assert_ne!(first.root_pfn(), second.root_pfn());

        first
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map first page");
        first
            .map_user_page(&mut allocator, 0x1000_1000, 0x9001, true, false, true)
            .expect("map adjacent page");
        assert_eq!(
            first.translate_user(&allocator, 0x1000_0123, true),
            Some(0x9000_123)
        );
        assert_eq!(first.translate_user(&allocator, 0x1000_1123, true), None);
        assert_eq!(
            first.translate_user(&allocator, 0x1000_1123, false),
            Some(0x9001_123)
        );
        assert!(first
            .map_user_page(&mut allocator, 0x1000_0000, 0xa000, true, true, false)
            .is_err());
        assert_eq!(
            first.unmap_user_page(&mut allocator, 0x1000_0000),
            Ok(0x9000)
        );
        assert_eq!(first.translate_user(&allocator, 0x1000_0000, false), None);
        assert_eq!(
            first.translate_user(&allocator, 0x1000_1000, false),
            Some(0x9001_000)
        );
    }

    #[test]
    fn three_level_model_rejects_non_page_level_three_descriptors() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        let vaddr = 0x1000_0000;
        address_space
            .map_user_page(&mut allocator, vaddr, 0x9000, true, true, false)
            .expect("map page");

        allocator.set_l3_descriptor(
            address_space.root_pfn(),
            vaddr,
            0x9000_000 | AARCH64_DESC_VALID | AARCH64_DESC_AP_USER,
        );

        assert_eq!(address_space.translate_user(&allocator, vaddr, false), None);
        assert!(address_space
            .protect_user_page(&mut allocator, vaddr, true, false, false)
            .is_err());
        assert!(address_space
            .unmap_user_page(&mut allocator, vaddr)
            .is_err());
    }

    #[test]
    fn three_level_model_protects_pages_and_supports_no_access() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map writable page");

        address_space
            .protect_user_page(&mut allocator, 0x1000_0000, true, false, true)
            .expect("protect read-only executable");
        assert_eq!(
            address_space.translate_user(&allocator, 0x1000_0042, false),
            Some(0x9000_042)
        );
        assert_eq!(
            address_space.translate_user(&allocator, 0x1000_0042, true),
            None
        );

        address_space
            .protect_user_page(&mut allocator, 0x1000_0000, false, false, false)
            .expect("protect no-access");
        assert_eq!(
            address_space.translate_user(&allocator, 0x1000_0042, false),
            None
        );
    }

    #[test]
    fn three_level_model_checked_copies_cross_pages_without_partial_faults() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        allocator.insert_data_page(0x9000);
        allocator.insert_data_page(0x9001);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map first data page");
        address_space
            .map_user_page(&mut allocator, 0x1000_1000, 0x9001, true, true, false)
            .expect("map second data page");

        let bytes: Vec<u8> = (0..32).map(|value| value as u8).collect();
        address_space
            .copy_to_user(&mut allocator, 0x1000_0ff0, &bytes)
            .expect("copy across page boundary");
        let mut copied = [0u8; 32];
        address_space
            .copy_from_user(&allocator, 0x1000_0ff0, &mut copied)
            .expect("read across page boundary");
        assert_eq!(copied.as_slice(), bytes.as_slice());

        address_space
            .protect_user_page(&mut allocator, 0x1000_1000, true, false, false)
            .expect("protect second page read-only");
        let replacement = [0xa5u8; 32];
        assert!(address_space
            .copy_to_user(&mut allocator, 0x1000_0ff0, &replacement)
            .is_err());
        let mut first_page_tail = [0u8; 16];
        address_space
            .copy_from_user(&allocator, 0x1000_0ff0, &mut first_page_tail)
            .expect("read unchanged first-page tail");
        assert_eq!(first_page_tail.as_slice(), &bytes[..16]);
    }

    #[test]
    fn three_level_copy_preflights_missing_physical_backing() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        allocator.insert_data_page(0x9000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map backed page");
        address_space
            .map_user_page(&mut allocator, 0x1000_1000, 0x9001, true, true, false)
            .expect("map page with missing test backing");

        assert!(address_space
            .copy_to_user(&mut allocator, 0x1000_0ff0, &[0xa5; 32])
            .is_err());
        let mut first_page_tail = [0u8; 16];
        address_space
            .copy_from_user(&allocator, 0x1000_0ff0, &mut first_page_tail)
            .expect("read backed first-page tail");
        assert_eq!(first_page_tail, [0; 16]);
    }

    #[test]
    fn three_level_model_destruction_returns_every_table_page() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let baseline = allocator.allocated_pages();
        let address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        let mut address_space = address_space;
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map page");
        assert_eq!(allocator.allocated_pages(), baseline + 3);
        drop(address_space);
        assert_eq!(allocator.allocated_pages(), baseline);
    }

    #[test]
    fn page_table_resource_count_tracks_owned_table_pages() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        assert_eq!(address_space.table_page_count(), 1);
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map page");
        assert_eq!(address_space.table_page_count(), 3);
    }

    #[test]
    fn destroying_one_process_root_preserves_another_process_mapping() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        allocator.insert_data_page(0x9000);
        allocator.insert_data_page(0xa000);
        let baseline = allocator.allocated_pages();
        let mut first = Aarch64AddressSpaceModel::new(&mut allocator).expect("first root");
        let mut second = Aarch64AddressSpaceModel::new(&mut allocator).expect("second root");

        first
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("map first process page");
        second
            .map_user_page(&mut allocator, 0x1000_0000, 0xa000, true, true, false)
            .expect("map second process page");
        second
            .copy_to_user(&mut allocator, 0x1000_0000, b"second")
            .expect("write second process page");
        let second_root = second.root_pfn();

        drop(first);

        assert_eq!(second.root_pfn(), second_root);
        assert_eq!(
            second.translate_user(&allocator, 0x1000_0000, true),
            Some(0xa000_000)
        );
        let mut copied = [0u8; 6];
        second
            .copy_from_user(&allocator, 0x1000_0000, &mut copied)
            .expect("read surviving process page");
        assert_eq!(&copied, b"second");

        drop(second);
        assert_eq!(allocator.allocated_pages(), baseline);
    }

    #[test]
    fn three_level_model_rolls_back_partial_table_allocation() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        let baseline = allocator.allocated_pages();
        allocator.fail_after_table_allocations(1);

        assert!(address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .is_err());
        assert_eq!(allocator.allocated_pages(), baseline);
        assert_eq!(
            address_space.translate_user(&allocator, 0x1000_0000, false),
            None
        );

        allocator.allow_table_allocations();
        address_space
            .map_user_page(&mut allocator, 0x1000_0000, 0x9000, true, true, false)
            .expect("mapping succeeds after rollback");
        assert_eq!(
            address_space.translate_user(&allocator, 0x1000_0000, true),
            Some(0x9000_000)
        );
    }

    #[test]
    fn three_level_region_mapping_rolls_back_leaves_and_tables() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        let baseline = allocator.allocated_pages();
        let start = 0x101f_f000;
        allocator.fail_after_table_allocations(2);

        assert!(address_space
            .map_user_region(&mut allocator, start, 0x9000, 2, true, true, false)
            .is_err());
        assert_eq!(allocator.allocated_pages(), baseline);
        assert_eq!(address_space.translate_user(&allocator, start, false), None);
        assert_eq!(
            address_space.translate_user(&allocator, start + AARCH64_PAGE_SIZE, false),
            None
        );
    }

    #[test]
    fn page_mutations_request_publish_and_break_before_make_maintenance() {
        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        let vaddr = 0x1000_0000;

        address_space
            .map_user_page(&mut allocator, vaddr, 0x9000, true, true, false)
            .expect("map page");
        assert_eq!(
            allocator.maintenance_events(),
            vec![Aarch64MaintenanceEvent::Publish(vaddr)]
        );

        allocator.clear_maintenance_events();
        address_space
            .protect_user_page(&mut allocator, vaddr, true, false, false)
            .expect("protect page");
        assert_eq!(
            allocator.maintenance_events(),
            vec![
                Aarch64MaintenanceEvent::Break(vaddr),
                Aarch64MaintenanceEvent::Make,
            ]
        );

        allocator.clear_maintenance_events();
        address_space
            .unmap_user_page(&mut allocator, vaddr)
            .expect("unmap page");
        assert_eq!(
            allocator.maintenance_events(),
            vec![Aarch64MaintenanceEvent::Break(vaddr)]
        );
    }

    #[test]
    fn production_owner_uses_the_backend_parameterized_core() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/kernel_lowlevel/ARM64/user_address_space.rs"
        ));
        assert!(source.contains("Aarch64AddressSpaceCore<PageFrameBackend>"));
        assert!(source.contains("impl Aarch64AddressSpaceBackend for PageFrameBackend"));
        assert!(!source.contains("table_pfns: Vec<u64>"));
    }

    #[test]
    fn production_backend_enforces_user_page_tlb_maintenance() {
        let address_space = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/kernel_lowlevel/ARM64/user_address_space.rs"
        ));
        let cpu = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/kernel_lowlevel/ARM64/cpu.rs"
        ));
        assert!(address_space.contains("cpu::invalidate_user_page(vaddr)"));
        assert!(address_space.contains("cpu::complete_user_page_update()"));

        let invalidate = cpu
            .find("pub fn invalidate_user_page")
            .expect("user-page invalidation helper");
        let invalidate = &cpu[invalidate..];
        let first_dsb = invalidate
            .find("dsb ishst")
            .expect("publish descriptor write");
        let tlbi = invalidate.find("tlbi vae1is").expect("invalidate page");
        let second_dsb = invalidate[tlbi..]
            .find("dsb ish")
            .map(|offset| tlbi + offset)
            .expect("complete invalidation");
        let isb = invalidate[second_dsb..]
            .find("isb")
            .map(|offset| second_dsb + offset)
            .expect("synchronize translation use");
        assert!(first_dsb < tlbi && tlbi < second_dsb && second_dsb < isb);

        let complete = cpu
            .find("pub fn complete_user_page_update")
            .expect("break-before-make completion helper");
        let complete = &cpu[complete..];
        assert!(
            complete.find("dsb ishst").expect("publish replacement")
                < complete.find("isb").expect("synchronize replacement")
        );
    }

    #[test]
    fn aarch64_manager_delegates_regions_without_a_legacy_kernel_root() {
        let mmu = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/kernel_lowlevel/mmu.rs"
        ));
        assert!(mmu.contains("let kernel_root_vaddr = core::ptr::null_mut()"));
        assert!(mmu.contains(".user_address_space"));
        assert!(mmu.contains(".map_user_region("));
        assert!(mmu.contains("return false; // AArch64 kernel mappings live in the shared root."));
    }

    #[test]
    fn physical_allocator_range_excludes_kernel_and_partial_pages() {
        assert_eq!(
            aarch64_frame_range(0x4fb0_7001, 0x4000_0000, 0x4000_0000),
            None
        );
        assert_eq!(
            aarch64_frame_range(0x4fb0_7001, 0x4000_0000, 0x6000_0000),
            Some((0x4fb0_8000, 0x6000_0000))
        );
    }

    #[test]
    fn base_zero_ram_reserves_the_user_window_for_el0_mappings() {
        assert_eq!(
            aarch64_frame_range(0x0800_0000, 0, 0x3c00_0000),
            Some((AARCH64_USER_LIMIT, 0x3c00_0000))
        );
        assert_eq!(
            aarch64_frame_range(0x0800_0000, 0, 0x1800_0000),
            Some((0x0800_0000, AARCH64_USER_BASE))
        );

        let mut allocator = Aarch64TestAllocator::new(0x8000);
        let mut address_space = Aarch64AddressSpaceModel::new(&mut allocator).expect("root");
        address_space
            .map_supervisor_ram_range(0, 0x3c00_0000)
            .expect("map base-zero RAM around the user window");
        address_space
            .map_user_page(&mut allocator, AARCH64_USER_BASE, 0x9000, true, true, false)
            .expect("map a user page after installing base-zero RAM");
        assert_eq!(
            address_space.translate_user(&allocator, AARCH64_USER_BASE, true),
            Some(0x9000_000)
        );
    }

    #[test]
    fn oversized_physical_allocator_range_is_capped_without_leaking_frames() {
        const PAGE_SIZE: usize = AARCH64_PAGE_SIZE;
        const BITMAP_CAPACITY_BYTES: usize = 2 * 1024 * 1024 * 1024;
        const BITMAP_WORDS: usize = (BITMAP_CAPACITY_BYTES / PAGE_SIZE) / 64;
        let detected_range = aarch64_frame_range(0x4fb0_7001, 0x4000_0000, 0x1_4000_0000)
            .expect("oversized valid RAM remains detectable");
        let range =
            aarch64_frame_range_cap(detected_range.0, detected_range.1, BITMAP_CAPACITY_BYTES)
                .expect("oversized valid RAM remains usable");

        assert_eq!(range, (0x4fb0_8000, 0x4fb0_8000 + BITMAP_CAPACITY_BYTES));
        let mut allocator = crate::lowlevel_logic::PageFrameAllocatorCore::<BITMAP_WORDS>::new(0);
        assert!(allocator.init_range(range.0, range.1, PAGE_SIZE));
        assert_eq!(
            allocator.pfn_address((range.1 / PAGE_SIZE) as u64 - 1, PAGE_SIZE),
            Some(range.1 - PAGE_SIZE)
        );
        assert_eq!(
            allocator.pfn_address((range.1 / PAGE_SIZE) as u64, PAGE_SIZE),
            None
        );
        assert_eq!(allocator.alloc(), Some((range.0 / PAGE_SIZE) as u64));
    }

    #[test]
    fn supported_high_physical_allocator_range_is_unchanged() {
        let start = usize::MAX - 0x2fff;
        let end = usize::MAX - 0xfff;
        assert_eq!(
            aarch64_frame_range_cap(start, end, 2 * 1024 * 1024 * 1024),
            Some((start, end))
        );
    }

    #[test]
    fn user_window_is_below_qemu_ram_and_page_aligned() {
        assert!(aarch64_user_range_valid(0x1000_0000, 0x1000));
        assert!(aarch64_user_range_valid(0x1fff_e000, 0x2000));
        assert!(!aarch64_user_range_valid(0x0fff_f000, 0x2000));
        assert!(!aarch64_user_range_valid(0x1fff_f000, 0x2000));
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
        assert_eq!(offset_of!(CpuContext, ttbr0_el1), 0x130);
        assert_eq!(offset_of!(CpuContext, fpcr), 0x138);
        assert_eq!(offset_of!(CpuContext, fpsr), 0x140);
        assert_eq!(offset_of!(CpuContext, simd), 0x150);
        assert_eq!(size_of::<CpuContext>(), 0x350);
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
    fn fxfs_hard_link_counts_retain_the_inode_until_the_last_name_is_removed() {
        assert_eq!(fxfs_link_count_after_link(1), Some(2));
        assert_eq!(fxfs_link_count_after_unlink(2), Some(1));
        assert_eq!(fxfs_link_count_after_unlink(1), Some(0));
        assert_eq!(fxfs_link_count_after_link(0), None);
        assert_eq!(fxfs_link_count_after_link(u32::MAX), None);
        assert_eq!(fxfs_link_count_after_unlink(0), None);
        assert!(!fxfs_unlinked_object_reclaimable(0, 1));
        assert!(fxfs_unlinked_object_reclaimable(0, 0));
        assert!(!fxfs_unlinked_object_reclaimable(1, 0));
    }

    #[test]
    fn run_elf_page_permissions_union_overlaps_and_leave_holes_unmapped() {
        const READ: usize = 1;
        const WRITE: usize = 2;
        const EXEC: usize = 4;
        let segments = [
            (0x1003usize, 0x0fedusize, READ),
            (0x1ff0usize, 0x0040usize, EXEC),
            (0x4000usize, 0x1000usize, READ | WRITE),
        ];

        assert_eq!(
            run_elf_page_protection(0x1000, 0x1000, &segments),
            Some(READ | EXEC)
        );
        assert_eq!(
            run_elf_page_protection(0x2000, 0x1000, &segments),
            Some(EXEC)
        );
        assert_eq!(run_elf_page_protection(0x3000, 0x1000, &segments), None);
        assert_eq!(
            run_elf_page_protection(0x4000, 0x1000, &segments),
            Some(READ | WRITE)
        );
        assert_eq!(run_elf_page_protection(usize::MAX, 0x1000, &segments), None);
        assert_eq!(run_elf_page_protection(0x1000, 0, &segments), None);
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
        let exiting = tasks.reserve_child(LINUX_ROOT_TID, 8).unwrap();
        let peer = tasks.reserve_child(LINUX_ROOT_TID, 9).unwrap();
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
