#![allow(unused_comparisons, unused_macros)]

#[test]
fn fxfs_cursor_identity_drives_record_lock_size_lookup() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS implementation");

    assert!(fxfs.contains("pub fn object_id(self) -> u64"));
    assert!(fxfs.contains("pub fn cursor_attrs(cursor: FxfsCursor)"));

    let cursor_attrs_start = fxfs
        .find("fn cursor_attrs(&mut self, cursor: FxfsCursor)")
        .expect("FxFS cursor attribute lookup");
    let cursor_attrs = braced_body(&fxfs[cursor_attrs_start..]);
    assert!(cursor_attrs.contains("cursor.object_id"));
    assert!(!cursor_attrs.contains("resolve_path("));
}

#[test]
fn linux_fcntl_marshals_aarch64_record_locks() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    for declaration in [
        "const LINUX_FLOCK_BYTES: usize = 32;",
        "const LINUX_FLOCK_TYPE_OFFSET: usize = 0;",
        "const LINUX_FLOCK_WHENCE_OFFSET: usize = 2;",
        "const LINUX_FLOCK_START_OFFSET: usize = 8;",
        "const LINUX_FLOCK_LEN_OFFSET: usize = 16;",
        "const LINUX_FLOCK_PID_OFFSET: usize = 24;",
    ] {
        assert!(syscall.contains(declaration), "missing {declaration}");
    }

    let read_start = syscall
        .find("fn linux_read_flock(")
        .expect("AArch64 flock copy-in helper");
    let read = braced_body(&syscall[read_start..]);
    assert!(read.contains("linux_copy_from_user("));
    assert!(read.contains("linux_wire_field"));
    assert!(!read.contains("as *const LinuxFlock"));

    let write_start = syscall
        .find("fn linux_write_flock(")
        .expect("AArch64 flock copy-out helper");
    let write = braced_body(&syscall[write_start..]);
    assert!(write.contains("linux_put_wire_field"));
    assert!(write.contains("linux_copy_to_user("));
    assert!(!write.contains("as *mut LinuxFlock"));
}

#[test]
fn linux_fcntl_routes_process_owned_record_locks_without_state_locking() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let fcntl_start = syscall.find("pub fn sys_fcntl(").expect("fcntl syscall");
    let fcntl = braced_body(&syscall[fcntl_start..]);
    assert!(fcntl.contains("F_GETLK | F_SETLK | F_SETLKW => linux_fcntl_record_lock(fd, cmd, arg)"));
    let route_start = syscall
        .find("fn linux_fcntl_record_lock(")
        .expect("record-lock fcntl route");
    let route = braced_body(&syscall[route_start..]);

    for token in [
        "F_GETLK",
        "F_SETLK",
        "F_SETLKW",
        "ObjectType::LinuxFile",
        "linux_read_flock(arg)",
        "linux_process::current_pid()",
        "file.cursor.object_id()",
        "fxfs::cursor_attrs(file.cursor)",
        "normalize_linux_record_lock_range(",
        "LinuxRecordLockRangeError::Invalid => SysError::EINVAL",
        "LinuxRecordLockRangeError::Overflow => SysError::EOVERFLOW",
        "linux_record_lock::first_conflict(",
        "linux_record_lock::set_nonblocking(",
        "linux_record_lock::set_blocking(",
        "LinuxRecordLockRuntimeError::Conflict => SysError::EAGAIN",
        "LinuxRecordLockRuntimeError::Capacity => SysError::ENOLCK",
        "SysError::EBADF",
        "linux_write_flock(arg, flock)",
    ] {
        assert!(route.contains(token), "missing fcntl routing token {token}");
    }

    let snapshot_start = route
        .find("let (readable, writable, file_id, offset, file_size) = {")
        .expect("bounded descriptor snapshot");
    let snapshot = braced_body(&route[snapshot_start..]);
    assert!(snapshot.contains("memory_state()"));
    let blocking = route
        .find("linux_record_lock::set_blocking(")
        .expect("blocking record-lock route");
    let last_state_borrow = route
        .rfind("memory_state()")
        .expect("descriptor state borrow");
    assert!(last_state_borrow < blocking);
    assert!(!route[blocking..].contains("memory_state()"));
    let descriptor_lookup = route
        .find("state.get_fd(fd).ok_or(SysError::EBADF)")
        .expect("record-lock descriptor validation");
    let owner_lookup = route
        .find("linux_process::current_pid()")
        .expect("process record-lock ownership");
    let flock_copy = route
        .find("linux_read_flock(arg)")
        .expect("record-lock flock copy-in");
    let access_check = route
        .find("if cmd != F_GETLK")
        .expect("set-lock access-mode validation");
    let normalization = route
        .find("normalize_linux_record_lock_range(")
        .expect("record-lock range normalization");
    assert!(descriptor_lookup < owner_lookup && owner_lookup < flock_copy);
    assert!(flock_copy < access_check && access_check < normalization);
    assert!(!route.contains("fork/11-1.c"));
    assert!(!route.contains("timeout"));
}

#[test]
fn linux_record_locks_follow_process_associated_lifecycle() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let modules = std::fs::read_to_string(repository.join("src/syscall/mod.rs"))
        .expect("read syscall modules");

    let close_helper_start = syscall
        .find("fn close_linux_fd_for_current_process(")
        .expect("central current-process descriptor close helper");
    let close_helper = braced_body(&syscall[close_helper_start..]);
    assert!(close_helper.contains("linux_process::current_pid().ok()"));
    assert!(!close_helper.contains("linux_resource_pid()"));
    let object_id = close_helper
        .find("file.cursor.object_id()")
        .expect("stable close file identity");
    let remove = close_helper
        .find("remove_fd_entry(fd)")
        .expect("descriptor removal");
    let release_lock = close_helper
        .find("linux_record_lock::release_owner_file(owner, file_id)")
        .expect("process/file record-lock release");
    let release_description = close_helper
        .find("release_open_description(entry.description_id)")
        .expect("open-description release");
    assert!(object_id < remove && remove < release_lock && release_lock < release_description);

    let close_start = syscall.find("pub fn sys_close(").expect("close syscall");
    let close = braced_body(&syscall[close_start..]);
    assert!(close.contains("close_linux_fd_for_current_process(fd)"));
    assert!(close.contains("fd <= 2"));

    let dup_start = syscall.find("pub fn sys_dup3(").expect("dup3 syscall");
    let dup = braced_body(&syscall[dup_start..]);
    assert!(dup.contains("close_linux_fd_for_current_process(new_fd)"));

    let close_range_start = syscall
        .find("pub fn sys_close_range(")
        .expect("close_range syscall");
    let close_range = braced_body(&syscall[close_range_start..]);
    assert!(close_range.contains("sys_close(fd)"));

    let release_start = syscall
        .find("pub(crate) fn release_linux_process_resources(")
        .expect("process resource release");
    let release = braced_body(&syscall[release_start..]);
    let owner_release = release
        .find("linux_record_lock::release_owner(pid)")
        .expect("process record-lock release");
    let resource_release = release
        .find("memory_state().release_process_resources(pid)")
        .expect("descriptor/object release");
    assert!(owner_release < resource_release);

    for function in [
        "pub(crate) fn reserve_linux_resource_clone(",
        "pub(crate) fn install_linux_resource_clone(",
        "pub(crate) fn rollback_linux_fork_process_resources(",
    ] {
        let start = syscall
            .find(function)
            .expect("fork resource lifecycle function");
        assert!(!braced_body(&syscall[start..]).contains("linux_record_lock"));
    }

    let normal_exit = braced_body(
        &process[process
            .find("pub(crate) fn exit_current_process(")
            .expect("normal process exit")..],
    );
    let signal_exit = braced_body(
        &process[process
            .find("pub(crate) fn terminate_by_signal(")
            .expect("signal process exit")..],
    );
    assert!(normal_exit.contains("release_linux_process_resources(process.pid)"));
    assert!(signal_exit.contains("release_linux_process_resources(tgid)"));

    let reset = braced_body(
        &syscall[syscall
            .find("pub fn reset_linux_process_state(")
            .expect("Linux launch reset")..],
    );
    let task_reset = reset.find("linux_task::reset()").expect("task reset");
    let lock_reset = reset
        .find("linux_record_lock::reset()")
        .expect("record-lock reset");
    assert!(task_reset < lock_reset);

    let retire = braced_body(
        &task[task
            .find("fn complete_task_retirements(")
            .expect("task retirement")..],
    );
    assert!(retire.contains("linux_record_lock::remove_task_waiters("));

    let execve = braced_body(&syscall[syscall.find("pub fn sys_execve(").expect("execve stub")..]);
    assert!(!execve.contains("linux_record_lock"));
    assert!(!modules.contains("#[allow(dead_code)]\npub(crate) mod linux_record_lock;"));
}

#[test]
fn linux_record_lock_runtime_blocks_without_missed_wakeups() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = std::fs::read_to_string(repository.join("src/syscall/linux_record_lock.rs"))
        .unwrap_or_default();
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task shared logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    assert!(task_logic.contains("RecordLock"));
    assert!(runtime.contains("LinuxRecordLockState<"));
    assert!(runtime.contains("LinuxRuntimeLock<LinuxRecordLockRuntimeState>"));
    assert!(runtime.contains("current_cpu_id() != 0"));

    let blocking_start = runtime
        .find("pub(crate) fn set_blocking(")
        .expect("blocking record-lock operation");
    let blocking = braced_body(&runtime[blocking_start..]);
    let interrupt_mask = blocking.find("mask_interrupts()").expect("interrupt mask");
    let runtime_lock = blocking
        .find("LINUX_RECORD_LOCK_RUNTIME.lock()")
        .expect("record-lock runtime guard");
    let conflict = blocking
        .find("first_conflict(")
        .expect("conflict recheck under runtime guard");
    let publication = blocking
        .find("runtime.push(waiter)")
        .expect("waiter publication");
    let runtime_drop = publication
        + blocking[publication..]
            .find("drop(runtime)")
            .expect("record-lock runtime unlock after waiter publication");
    let task_block = blocking
        .find("linux_task::block_current(LinuxBlockReason::RecordLock)")
        .expect("task-runtime block publication");
    let schedule = blocking
        .find("scheduler::schedule()")
        .expect("scheduler handoff");
    let interrupt_restore = blocking
        .rfind("restore_interrupts(interrupt_state)")
        .expect("interrupt restore");
    assert!(interrupt_mask < runtime_lock);
    assert!(runtime_lock < conflict);
    assert!(conflict < publication);
    assert!(publication < runtime_drop);
    assert!(runtime_drop < task_block);
    assert!(task_block < schedule);
    assert!(schedule < interrupt_restore);
    let take_outcome = blocking
        .find("runtime.take_outcome(tid, scheduler_thread.0)")
        .expect("terminal waiter outcome consumption");
    let missing_outcome = blocking[take_outcome..]
        .find("None =>")
        .expect("missing record-lock outcome branch");
    let cleanup_relative = blocking[take_outcome + missing_outcome..]
        .find("runtime.remove_task(tid, scheduler_thread.0)")
        .expect("spurious-resume waiter cleanup");
    let cleanup = take_outcome + missing_outcome + cleanup_relative;
    assert!(take_outcome + missing_outcome < cleanup);

    let wake_start = runtime
        .find("fn wake_ready_tasks(")
        .expect("post-commit record-lock wake helper");
    let wake = braced_body(&runtime[wake_start..]);
    let lock = wake
        .find("LINUX_RECORD_LOCK_RUNTIME.lock()")
        .expect("record-lock wake guard");
    let collect = wake
        .find("wake_ready()")
        .expect("ready identity collection");
    let drop = wake.find("drop(runtime)").expect("record-lock wake unlock");
    let wake_task = wake
        .find("linux_task::wake_blocked(")
        .expect("task wake after unlock");
    assert!(lock < collect && collect < drop && drop < wake_task);

    let interrupt_target = braced_body(
        &syscall[syscall
            .find("fn interrupt_linux_signal_target(")
            .expect("signal target interruption")..],
    );
    assert!(interrupt_target.contains("LinuxBlockReason::RecordLock"));
    assert!(interrupt_target.contains("linux_record_lock::interrupt_task("));

    let retire = braced_body(
        &task[task
            .find("fn complete_task_retirements(")
            .expect("task retirement completion")..],
    );
    let futex_cleanup = retire
        .find("linux_futex::remove_task_waiters(")
        .expect("futex waiter cleanup");
    let lock_cleanup = retire
        .find("linux_record_lock::remove_task_waiters(")
        .expect("record-lock waiter cleanup");
    assert!(futex_cleanup < lock_cleanup);
}

#[test]
fn arm64_posix_mqueue_syscalls_are_real_handlers_not_compat_noops() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let modules = std::fs::read_to_string(repository.join("src/syscall/mod.rs"))
        .expect("read syscall modules");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task shared logic");

    for declaration in [
        "const ARM64_SYS_MQ_OPEN: u32 = 180;",
        "const ARM64_SYS_MQ_UNLINK: u32 = 181;",
        "const ARM64_SYS_MQ_TIMEDSEND: u32 = 182;",
        "const ARM64_SYS_MQ_TIMEDRECEIVE: u32 = 183;",
        "const ARM64_SYS_MQ_NOTIFY: u32 = 184;",
        "const ARM64_SYS_MQ_GETSETATTR: u32 = 185;",
    ] {
        assert!(syscall.contains(declaration), "missing {declaration}");
    }

    let dispatch = braced_body(
        &syscall[syscall
            .find("pub fn dispatch_linux_syscall(")
            .expect("Linux syscall dispatcher")..],
    );
    for route in [
        "ARM64_SYS_MQ_OPEN => sys_mq_open(args[0], args[1], args[2], args[3])",
        "ARM64_SYS_MQ_UNLINK => sys_mq_unlink(args[0])",
        "ARM64_SYS_MQ_TIMEDSEND => sys_mq_timedsend(args[0], args[1], args[2], args[3], args[4])",
        "ARM64_SYS_MQ_TIMEDRECEIVE =>",
        "sys_mq_timedreceive(args[0], args[1], args[2], args[3], args[4])",
        "ARM64_SYS_MQ_NOTIFY => sys_mq_notify(args[0], args[1])",
        "ARM64_SYS_MQ_GETSETATTR => sys_mq_getsetattr(args[0], args[1], args[2])",
    ] {
        assert!(dispatch.contains(route), "missing dispatch route {route}");
    }
    let unsupported_start = dispatch
        .find("ARM64_SYS_LOOKUP_DCOOKIE")
        .expect("compatibility unsupported cluster");
    let unsupported_end = dispatch[unsupported_start..]
        .find("=> Ok(0)")
        .expect("unsupported cluster end")
        + unsupported_start;
    let unsupported = &dispatch[unsupported_start..unsupported_end];
    assert!(!unsupported.contains("ARM64_SYS_MQ_"));

    let close = braced_body(&syscall[syscall.find("pub fn sys_close(").expect("close syscall")..]);
    assert!(close.contains("Err(SysError::EBADF)"));
    assert!(!close.contains("Err(SysError::EBUSY)"));

    assert!(modules.contains("pub(crate) mod linux_mqueue;"));
    assert!(task_logic.contains("Mqueue"));
    assert!(syscall.contains("linux_mqueue::close_handle(handle);"));
    assert!(syscall.contains("linux_mqueue::reset();"));
}

#[test]
fn mq_open_creation_ignores_mq_attr_fields_posix_marks_ignored() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let reader = braced_body(
        &syscall[syscall
            .find("fn linux_read_user_mq_attr(")
            .expect("mq_attr reader")..],
    );

    assert!(!reader.contains("flags < 0"));
    assert!(!reader.contains("curmsgs < 0"));
    assert!(reader.contains("maxmsg <= 0"));
    assert!(reader.contains("msgsize <= 0"));
}

#[test]
fn posix_clock_timer_clock_runtime_applies_checked_realtime_offsets() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let settime_start = syscall
        .find("pub fn sys_clock_settime(")
        .expect("clock_settime implementation");
    let settime = braced_body(&syscall[settime_start..]);
    let gettime_start = syscall
        .find("pub fn sys_clock_gettime(")
        .expect("clock_gettime implementation");
    let gettime = braced_body(&syscall[gettime_start..]);
    let time_start = syscall
        .find("pub fn sys_time(")
        .expect("time implementation");
    let time = braced_body(&syscall[time_start..]);
    let gettimeofday_start = syscall
        .find("pub fn sys_gettimeofday(")
        .expect("gettimeofday implementation");
    let gettimeofday = braced_body(&syscall[gettimeofday_start..]);
    let reset_start = syscall
        .find("pub fn reset_linux_signal_timer_state(")
        .expect("Linux signal and timer reset");
    let reset = braced_body(&syscall[reset_start..]);

    assert!(syscall.contains("static LINUX_REALTIME_OFFSET_NANOS: AtomicI64"));
    assert!(settime.contains("linux_posix_clock_settable(clockid)"));
    assert!(settime.contains("linux_read_user_timespec(tp)?"));
    assert!(settime.contains("linux_realtime_offset_for_set("));
    assert!(settime.contains("LINUX_REALTIME_OFFSET_NANOS.store("));
    assert!(gettime.contains("linux_clock_nanoseconds(clock)?"));
    assert!(time.contains("linux_realtime_nanos()?"));
    assert!(gettimeofday.contains("linux_realtime_nanos()?"));
    assert!(syscall.contains("const LINUX_DEFAULT_REALTIME_OFFSET_NANOS: i64"));
    assert!(reset.contains(
        "LINUX_REALTIME_OFFSET_NANOS.store(LINUX_DEFAULT_REALTIME_OFFSET_NANOS"
    ));
}

#[test]
fn posix_clock_timer_syscalls_copy_validate_and_publish_owned_state() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let timer_create = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_create(")
            .expect("POSIX timer creation")..],
    );
    let timer_settime = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_settime(")
            .expect("POSIX timer arming")..],
    );
    let timer_gettime = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_gettime(")
            .expect("POSIX timer query")..],
    );
    let timer_delete = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_delete(")
            .expect("POSIX timer deletion")..],
    );
    let reset = braced_body(
        &syscall[syscall
            .find("fn reset_linux_process_state(&mut self) -> Vec<u32>")
            .expect("process-owned resource reset")..],
    );

    assert!(syscall.contains("struct LinuxItimerspec"));
    assert!(syscall.contains("struct LinuxSigevent"));
    assert!(syscall.contains("posix_timers: Vec<LinuxPosixTimerCore>"));
    assert!(timer_create.contains("LinuxPosixClock::from_id(clockid)"));
    assert!(timer_create.contains("linux_read_user_sigevent(sevp)?"));
    assert!(timer_create.contains("register_linux_timer(pid, handle.0, timer_id, clock, signal)"));
    assert!(timer_settime.contains("linux_read_user_itimerspec(new_value)?"));
    assert!(timer_settime.contains("linux_posix_timespec_nanoseconds("));
    assert!(timer_settime.contains("timer.arm("));
    assert!(timer_gettime.contains("timer.snapshot("));
    assert!(timer_delete.contains("remove_linux_timer(pid, timerid as u32)"));
    assert!(reset.contains("resources.posix_timers"));

    let preflight = timer_create
        .find("linux_user_buffer_writable(")
        .expect("complete timer-ID preflight");
    let create = timer_create
        .find("compat::create_object(ObjectType::Timer)")
        .expect("timer compatibility object allocation");
    let register = timer_create
        .find("register_linux_timer(pid, handle.0, timer_id, clock, signal)")
        .expect("timer state registration");
    let failed_copyout = timer_create
        .find("if let Err(error) = linux_write_user_i32(")
        .expect("fallible timer-ID copyout");
    let remove = timer_create[failed_copyout..]
        .find("remove_linux_timer(pid, timer_id)")
        .map(|offset| failed_copyout + offset)
        .expect("timer state rollback");
    let close = timer_create[failed_copyout..]
        .find("sys_handle_close(handle.0)")
        .map(|offset| failed_copyout + offset)
        .expect("timer handle rollback");

    assert!(preflight < create);
    assert!(create < register);
    assert!(register < failed_copyout);
    assert!(failed_copyout < remove);
    assert!(remove < close);
}

#[test]
fn posix_timer_create_copies_out_a_kernel_width_timer_id() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let timer_create = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_create(")
            .expect("POSIX timer creation")..],
    );

    assert!(timer_create.contains("core::mem::size_of::<i32>()"));
    assert!(timer_create.contains("linux_write_user_i32(timerid, timer_id as i32)"));
    assert!(!timer_create.contains("linux_write_user_usize(timerid"));
}

#[test]
fn posix_timer_ids_do_not_expose_negative_compatibility_handles() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let timer_create = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_create(")
            .expect("POSIX timer creation")..],
    );
    let timer_settime = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_settime(")
            .expect("POSIX timer arming")..],
    );

    assert!(timer_create.contains("let timer_id = handle.0 & i32::MAX as u32"));
    assert!(timer_create.contains("register_linux_timer(pid, handle.0, timer_id, clock, signal)"));
    assert!(timer_create.contains("linux_write_user_i32(timerid, timer_id as i32)"));
    assert!(
        syscall.contains("fn linux_timer_handle(&self, pid: usize, timer_id: u32) -> Option<u32>")
    );
    assert!(timer_settime.contains("linux_timer_handle(pid, timerid as u32)"));
    assert!(!timer_settime.contains("compat::handle_known(HandleValue(timerid as u32))"));
}

#[test]
fn posix_clock_timer_cpu0_expiry_queues_process_signals() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let main = std::fs::read_to_string(repository.join("src/main.rs")).expect("read kernel main");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let timer = braced_body(
        &main[main
            .find("extern \"C\" fn timer_interrupt_handler()")
            .expect("timer interrupt handler")..],
    );
    let expiry = braced_body(
        &syscall[syscall
            .find("pub fn deliver_linux_posix_timer_signals_from_irq()")
            .expect("POSIX timer IRQ expiry entry point")..],
    );

    assert!(timer.contains("if current_cpu_id() == 0"));
    assert!(timer.contains("deliver_linux_posix_timer_signals_from_irq()"));
    assert!(expiry.contains("linux_realtime_nanos()"));
    assert!(expiry.contains("timer.expire(now_monotonic, now_realtime)"));
    assert!(expiry.contains("queue_process_linux_signal_and_wake("));
    assert!(expiry.contains("LinuxPendingSignal::standard(signal)"));

    let scheduler = timer
        .find("scheduler().on_timer_tick()")
        .expect("scheduler timer accounting");
    let linux_task = timer
        .find("linux_task::on_timer_tick(now)")
        .expect("Linux task timeout expiry");
    let posix_timer = timer
        .find("deliver_linux_posix_timer_signals_from_irq()")
        .expect("POSIX timer signal expiry");
    let futex = timer
        .find("linux_futex::on_timer_tick(now, now)")
        .expect("Linux futex timeout expiry");
    let completion = timer
        .find("interrupt::end_of_interrupt(interrupt_id)")
        .expect("timer interrupt completion");

    assert!(scheduler < linux_task);
    assert!(linux_task < posix_timer);
    assert!(posix_timer < futex);
    assert!(futex < completion);
}

#[test]
fn aarch64_page_allocator_uses_detected_ram_after_kernel_end() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let memory = std::fs::read_to_string(repository.join("src/kernel_lowlevel/memory.rs"))
        .expect("read memory module");
    let drivers = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/drivers.rs"))
        .expect("read AArch64 drivers");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/lowlevel_logic_shared.rs"))
            .expect("read shared low-level logic");
    let main = std::fs::read_to_string(repository.join("src/main.rs")).expect("read kernel main");

    assert!(drivers.contains("pub memory_base: usize"));
    assert!(drivers.contains("pub memory_size: usize"));
    assert!(drivers.contains("pub fn memory_reg() -> Option<DeviceReg>"));
    assert!(drivers.contains("lowlevel_logic::memory_reg("));
    assert!(memory.contains("static __kernel_end"));
    assert!(memory.contains("aarch64_frame_range("));
    assert!(shared.contains("struct PageFrameAllocatorCore"));
    assert!(memory.contains("PageFrameAllocatorCore<PAGE_FRAME_BITMAP_WORDS>"));
    assert!(memory.contains("PageFrameAllocator::init_range(frame_start, frame_end)"));

    let init_start = memory
        .find("pub fn init()")
        .expect("memory initialization entry point");
    let init_body = braced_body(&memory[init_start..]);
    let range = init_body
        .find("aarch64_frame_range(")
        .expect("derive allocator range");
    let allocator = init_body
        .find("PageFrameAllocator::init_range(frame_start, frame_end)")
        .expect("initialize physical page allocator");
    assert!(range < allocator);

    let memory_init = main
        .find("kernel_lowlevel::memory::init()")
        .expect("initialize memory");
    let mmu_init = main
        .find("kernel_lowlevel::mmu::init()")
        .expect("initialize MMU");
    assert!(memory_init < mmu_init);
}

#[test]
fn aarch64_memory_uses_one_vm_logic_module_without_broad_dead_code_suppression() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let modules = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mod.rs"))
        .expect("read kernel low-level modules");
    let memory = std::fs::read_to_string(repository.join("src/kernel_lowlevel/memory.rs"))
        .expect("read memory module");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/lowlevel_logic_shared.rs"))
            .expect("read shared low-level logic");

    assert!(modules.contains("pub(crate) mod aarch64_vm_logic_shared;"));
    assert!(memory.contains("use super::aarch64_vm_logic_shared as aarch64_vm_logic;"));
    assert!(!memory.contains("#![allow(dead_code)]"));
    assert!(!memory.contains("use alloc::vec::Vec;"));
    assert!(!memory.contains("#[path = \"aarch64_vm_logic_shared.rs\"]"));
    assert!(!memory.contains("pub struct Shell"));
    assert!(!memory.contains("pub fn demo_processes()"));
    for model_macro in [
        "smros_ll_segment_contains_body",
        "smros_ll_heap_alloc_body",
        "smros_ll_stack_alloc_body",
        "smros_ll_page_to_vaddr_body",
        "smros_ll_pfn_valid_body",
        "smros_ll_bitmap_word_index_body",
        "smros_ll_bitmap_bit_index_body",
        "smros_ll_bitmap_mask_body",
    ] {
        assert!(
            shared.contains(&format!(
                "#[cfg(not(target_os = \"none\"))]\nmacro_rules! {model_macro}"
            )),
            "model-only macro {model_macro} must not enter the kernel build"
        );
    }
}

#[test]
fn aarch64_process_roots_walk_distinct_four_kib_pages() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mmu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mmu.rs"))
        .expect("read MMU module");
    let address_space =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/user_address_space.rs"))
            .expect("read AArch64 address-space owner");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/aarch64_vm_logic_shared.rs"))
            .expect("read shared AArch64 address-space core");
    let arm64 = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/mod.rs"))
        .expect("read AArch64 module");
    let user_logic =
        std::fs::read_to_string(repository.join("src/user_level/services/user_logic.rs"))
            .expect("read user layout constants");

    assert!(!mmu.contains("let user_root_pfn = PageFrameAllocator::alloc()?"));
    assert!(!mmu.contains("fn page_table_slot(vaddr: usize)"));
    assert!(arm64.contains("pub mod user_address_space;"));
    assert!(address_space.contains("pub struct Aarch64AddressSpace"));
    assert!(address_space.contains("Aarch64AddressSpaceCore<PageFrameBackend>"));
    assert!(shared.contains("aarch64_table_indices(vaddr)"));
    assert!(shared.contains("indices[..2]"));
    assert!(shared.contains("indices[2]"));
    assert!(user_logic.contains("USER_CODE_VADDR: usize = 0x1000_0000"));
    assert!(user_logic.contains("USER_DATA_VADDR: usize = 0x1000_1000"));
    assert!(user_logic.contains("USER_HEAP_VADDR: usize = 0x1000_2000"));
    assert!(user_logic.contains("USER_STACK_VADDR: usize = 0x1FFF_D000"));
}

#[test]
fn aarch64_bootstrap_mmu_maps_ram_mmio_and_secondary_cpus() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mmu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mmu.rs"))
        .expect("read MMU module");
    let address_space =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/user_address_space.rs"))
            .expect("read AArch64 address-space owner");
    let shared =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/aarch64_vm_logic_shared.rs"))
            .expect("read shared AArch64 VM logic");
    let cpu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/cpu.rs"))
        .expect("read AArch64 CPU module");
    let smp = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/smp.rs"))
        .expect("read AArch64 SMP module");

    assert!(mmu.contains("static BOOTSTRAP_ROOT: AtomicU64"));
    assert!(mmu.contains("pub fn bootstrap_root() -> u64"));
    assert!(mmu.contains("pub fn activate_bootstrap_on_current_cpu() -> bool"));
    let kernel_map = address_space
        .find("fn new_with_kernel_map_backend(")
        .map(|start| braced_body(&address_space[start..]))
        .expect("bootstrap address-space backend constructor");
    let kernel_map: String = kernel_map
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(kernel_map
        .contains("address_space.map_supervisor_ram_range(memory.base,memory.size,true)?;"));
    for device_mapping in [
        "address_space.map_supervisor_range(drivers::uart_base(),drivers::uart_size(),true,false,)?;",
        "address_space.map_supervisor_range(drivers::gicd_base(),drivers::gicd_size(),true,false,)?;",
        "address_space.map_supervisor_range(drivers::gicr_base(),drivers::gicr_size(),true,false,)?;",
        "address_space.map_supervisor_range(reg.base,reg.size,true,false)?;",
    ] {
        assert!(
            kernel_map.contains(device_mapping),
            "missing exact supervisor device mapping: {device_mapping}"
        );
    }
    let supervisor_descriptor = shared
        .find("pub(crate) fn aarch64_supervisor_block_descriptor(")
        .map(|start| braced_body(&shared[start..]))
        .expect("supervisor descriptor constructor");
    assert!(!supervisor_descriptor.contains("AARCH64_DESC_AP_USER"));

    let install = cpu
        .find("pub unsafe fn install_stage1_translation")
        .expect("stage-one installation helper");
    let install_body = braced_body(&cpu[install..]);
    let compact_install: String = install_body
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact_install.contains("letmair=0xffu64|(0x04u64<<8);"));
    assert!(compact_install.contains("lettcr=25u64|(1<<8)|(1<<10)|(3<<12)|(1<<23)|(2u64<<32);"));
    assert!(compact_install.contains("sctlr|=(1<<0)|(1<<2)|(1<<12);"));

    let mair = install_body.find("msr mair_el1").expect("install MAIR");
    let tcr = install_body.find("msr tcr_el1").expect("install TCR");
    let ttbr = install_body.find("msr ttbr0_el1").expect("install TTBR0");
    let first_dsb = install_body[ttbr..]
        .find("dsb ish")
        .map(|offset| ttbr + offset)
        .expect("barrier before TLB invalidation");
    let tlbi = install_body[first_dsb..]
        .find("tlbi vmalle1is")
        .map(|offset| first_dsb + offset)
        .expect("invalidate EL1 TLB");
    let second_dsb = install_body[tlbi..]
        .find("dsb ish")
        .map(|offset| tlbi + offset)
        .expect("barrier after TLB invalidation");
    let first_isb = install_body[second_dsb..]
        .find("isb")
        .map(|offset| second_dsb + offset)
        .expect("instruction barrier before enabling translation");
    let sctlr = install_body[first_isb..]
        .find("msr sctlr_el1")
        .map(|offset| first_isb + offset)
        .expect("enable stage-one translation and caches");
    assert!(mair < tcr && tcr < ttbr && ttbr < first_dsb);
    assert!(first_dsb < tlbi && tlbi < second_dsb && second_dsb < first_isb);
    assert!(first_isb < sctlr);

    let bootstrap_root = mmu
        .find("pub fn bootstrap_root() -> u64")
        .map(|start| braced_body(&mmu[start..]))
        .expect("bootstrap-root accessor");
    assert!(bootstrap_root.contains("BOOTSTRAP_ROOT.load(Ordering::Acquire)"));

    let activate = mmu
        .find("pub fn activate_bootstrap_on_current_cpu() -> bool")
        .map(|start| braced_body(&mmu[start..]))
        .expect("bootstrap activation helper");
    assert!(activate.contains("activate_root_on_current_cpu(bootstrap_root())"));
    let activate_candidate = mmu
        .find("fn activate_root_on_current_cpu(root: u64) -> bool")
        .map(|start| braced_body(&mmu[start..]))
        .expect("candidate-root activation helper");
    let reject_zero = activate_candidate
        .find("if root == 0")
        .expect("reject a zero root");
    let install_root = activate_candidate
        .find("install_stage1_translation(root)")
        .expect("install the candidate root");
    assert!(reject_zero < install_root);

    let init = mmu
        .find("pub fn init()")
        .map(|start| braced_body(&mmu[start..]))
        .expect("MMU initialization");
    let publish = init
        .find("BOOTSTRAP_ROOT.store(root, Ordering::Release)")
        .expect("publish bootstrap root");
    let retain_owner = init
        .find("KERNEL_PAGETABLE_MANAGER = Some(manager)")
        .expect("retain bootstrap address-space owner");
    let activate_cpu0 = init
        .find("activate_root_on_current_cpu(root)")
        .expect("activate bootstrap root on CPU0");
    assert!(activate_cpu0 < retain_owner && retain_owner < publish);

    let secondary = smp
        .find("pub extern \"C\" fn secondary_cpu_entry()")
        .expect("secondary CPU entry");
    let secondary_body = braced_body(&smp[secondary..]);
    let activate = secondary_body
        .find("mmu::activate_bootstrap_on_current_cpu()")
        .expect("activate bootstrap root");
    let serial = secondary_body
        .find("Serial::new()")
        .expect("initialize secondary serial");
    let unmask = secondary_body
        .find("msr daif")
        .expect("unmask secondary interrupts");
    assert!(activate < serial);
    assert!(activate < unmask);
}

#[test]
fn aarch64_bootstrap_mmu_initialization_fails_closed() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mmu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mmu.rs"))
        .expect("read MMU module");
    let main =
        std::fs::read_to_string(repository.join("src/main.rs")).expect("read kernel entry point");

    assert!(mmu.contains("pub enum MmuInitError"));
    let init_start = mmu
        .find("pub fn init() -> Result<(), MmuInitError>")
        .expect("fallible MMU initialization");
    let init = braced_body(&mmu[init_start..]);
    let reject_reinitialization = init
        .find("KERNEL_PAGETABLE_MANAGER.is_some()")
        .expect("reject repeated initialization");
    let construct = init
        .find("PageTableManager::new().ok_or(MmuInitError::OutOfMemory)?")
        .expect("propagate manager construction failure");
    let activate = init
        .find("activate_root_on_current_cpu(root)")
        .expect("activate the candidate root");
    let retain_owner = init
        .find("KERNEL_PAGETABLE_MANAGER = Some(manager)")
        .expect("retain the active root owner");
    let publish = init
        .find("BOOTSTRAP_ROOT.store(root, Ordering::Release)")
        .expect("publish the retained root");
    assert!(reject_reinitialization < construct);
    assert!(construct < activate && activate < retain_owner && retain_owner < publish);

    assert!(main.contains(
        "kernel_lowlevel::mmu::init().expect(\"initialize MMU before continuing boot\")"
    ));
}

#[test]
fn aarch64_context_switch_preserves_process_ttbr0() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let context =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_shared.rs"))
            .expect("read AArch64 context layout");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread initialization");

    let context_start = context.find("pub struct CpuContext").expect("CPU context");
    let context_body = braced_body(&context[context_start..]);
    let tls = context_body.find("pub tpidr_el0: u64").expect("TLS field");
    let root = context_body
        .find("pub ttbr0_el1: u64")
        .expect("TTBR0 field");
    let fpcr = context_body.find("pub fpcr: u64").expect("FPCR field");
    assert!(tls < root && root < fpcr);
    assert!(thread.contains("ttbr0_el1: crate::kernel_lowlevel::mmu::bootstrap_root()"));
    assert!(thread.contains("ttbr0_el1: 0"));

    let context_switch = assembly_routine(&switch, "context_switch");
    let context_switch_start = assembly_routine(&switch, "context_switch_start");
    assert_eq!(context_switch.matches("mrs     x17, ttbr0_el1").count(), 1);
    assert_eq!(
        context_switch.matches("str     x17, [x16, #0x130]").count(),
        1
    );

    for (name, body) in [
        ("context_switch", context_switch),
        ("context_switch_start", context_switch_start),
    ] {
        let load = body
            .find("ldr     x17, [x16, #0x130]")
            .unwrap_or_else(|| panic!("{name} must load its own TTBR0"));
        let restore_sequence = concat!(
            "msr     ttbr0_el1, x17\n",
            "    dsb     ish\n",
            "    tlbi    vmalle1is\n",
            "    dsb     ish\n",
            "    isb",
        );
        let restore = body
            .find(restore_sequence)
            .unwrap_or_else(|| panic!("{name} missing ordered TTBR0 restore sequence"));
        let restore_end = restore + restore_sequence.len();
        let simd = body
            .find("ldp     q0, q1, [x16, #0x150]")
            .expect("restore SIMD state");
        let return_branch = body
            .find("br      x16")
            .expect("return to restored context");
        assert!(load < restore, "{name} must load TTBR0 before restoring it");
        assert!(
            restore_end < simd,
            "{name} must synchronize before SIMD restore"
        );
        assert!(
            simd < return_branch,
            "{name} must restore SIMD state before returning"
        );
    }

    assert!(switch.contains("0x138 = fpcr"));
    assert!(switch.contains("0x140 = fpsr"));
    assert!(switch.contains("0x150 = q0-q31"));
}

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
fn linux_file_tail_fault_metadata_is_preserved_at_every_mapping_boundary() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read Linux process memory runtime");
    let fork = std::fs::read_to_string(repository.join("src/syscall/linux_fork_logic_shared.rs"))
        .expect("read Linux fork page logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux mmap syscall runtime");

    for token in [
        "backing_len: usize",
        "backing_len: *backing_len",
        "linux_effective_mapping_page_prot(",
        "classify_current_memory_fault(",
    ] {
        assert!(
            memory.contains(token),
            "missing file-fault metadata token {token}"
        );
    }
    assert!(syscall.contains("backing_len: attrs.size"));
    assert!(syscall.contains("linux_read_mmap_contents(&source, len)"));
    assert!(fork.contains("pub(crate) fn map_linux_fork_pages_with_protection"));
    assert!(memory.contains("super::linux_process::map_linux_fork_pages_with_protection("));

    let slice = braced_body(
        &memory[memory
            .find("fn try_slice(")
            .expect("mapping source slice implementation")..],
    );
    assert!(slice.contains("backing_len: *backing_len"));
    assert!(slice.contains("offset: offset.saturating_add(delta as u64)"));

    for boundary in [
        "fn try_mapping_piece(",
        "pub(crate) fn clone_for_fork(",
        "fn replace_mapping_transactionally(",
        "fn protect(",
        "fn remap(",
    ] {
        assert!(
            memory.contains(boundary),
            "missing mapping boundary {boundary}"
        );
    }
}

#[test]
fn synchronous_memory_fault_delivery_is_immediate_complete_and_fail_closed() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    for constant in [
        "const LINUX_SIGBUS: usize = 7;",
        "const LINUX_SIGSEGV: usize = 11;",
        "const LINUX_SEGV_MAPERR: i32 = 1;",
        "const LINUX_SEGV_ACCERR: i32 = 2;",
        "const LINUX_BUS_ADRERR: i32 = 2;",
    ] {
        assert!(
            syscall.contains(constant),
            "missing fault ABI constant {constant}"
        );
    }

    let delivery_start = syscall
        .find("pub(crate) fn deliver_linux_synchronous_memory_fault(")
        .expect("synchronous memory-fault entry");
    let delivery = braced_body(&syscall[delivery_start..]);
    for required in [
        "linux_task::current_task()",
        "LinuxMemoryFaultSignal::SegvMaperr => (LINUX_SIGSEGV, LINUX_SEGV_MAPERR)",
        "LinuxMemoryFaultSignal::SegvAccerr => (LINUX_SIGSEGV, LINUX_SEGV_ACCERR)",
        "LinuxMemoryFaultSignal::BusAdrerr => (LINUX_SIGBUS, LINUX_BUS_ADRERR)",
        "LinuxPendingSignal::synchronous_fault(signum, code, fault_address)",
        "linux_signal_action(signum)",
        "signal_state.mask & linux_signal_bit(signum)",
        "install_linux_signal_handler(",
        "terminate_linux_process_by_signal(current.tgid, signum)",
        "regs[0] = launch_id as u64",
    ] {
        assert!(
            delivery.contains(required),
            "missing synchronous delivery step {required}"
        );
    }
    assert!(!delivery.contains("queue_process_linux_signal"));
    assert!(!delivery.contains("requeue_linux_signal"));

    let installer_start = syscall
        .find("fn install_linux_signal_handler(")
        .expect("shared signal-handler installer");
    let installer = braced_body(&syscall[installer_start..]);
    for required in [
        "linux_aarch64_signal_user_frame(",
        "linux_signal_user_range_writable(",
        "linux_zero_user(context as usize, LINUX_AARCH64_UCONTEXT_BYTES)",
        "linux_aarch64_ucontext_core(",
        "linux_copy_to_user(context as usize, &context_core)",
        "signal_state.push_frame(frame)",
        "set_user_stack_pointer(frame_sp)",
        "regs[2] = context",
        "set_exception_return_pc(trampoline as u64)",
    ] {
        assert!(
            installer.contains(required),
            "missing signal-frame step {required}"
        );
    }
    assert!(!installer.contains("[u8; LINUX_AARCH64_UCONTEXT_BYTES]"));
    assert!(!syscall.contains("LINUX_SIGNAL_INFO_STORAGE_BYTES"));
    assert!(!syscall.contains("LINUX_SIGNAL_INFO_OFFSET"));

    let termination_start = syscall
        .find("fn terminate_linux_process_by_signal(")
        .expect("signal termination lifecycle");
    let termination = braced_body(&syscall[termination_start..]);
    assert!(termination.contains("linux_task::finish_current_without_el0_return()"));
    assert!(termination.contains("prepare_run_elf_return(exit_code)"));
}

#[test]
fn aarch64_synchronous_exception_vectors_are_origin_specific_and_fail_closed() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let boot = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/boot.rs"))
        .expect("read AArch64 exception vectors");
    let dispatch = std::fs::read_to_string(repository.join("src/syscall/syscall_dispatch.rs"))
        .expect("read AArch64 exception bridge");
    let lowlevel = std::fs::read_to_string(repository.join("src/kernel_lowlevel/mod.rs"))
        .expect("read architecture exports");

    let vectors_start = boot
        .find("exception_vectors:")
        .expect("AArch64 vector table");
    let vectors_end = boot[vectors_start..]
        .find("current_sync_sp0:")
        .expect("first synchronous vector routine");
    let vectors = &boot[vectors_start..vectors_start + vectors_end];
    for target in [
        "b       current_sync_sp0",
        "b       current_sync_spx",
        "b       lower_sync_a64",
        "b       lower_sync_a32",
    ] {
        assert!(
            vectors.contains(target),
            "missing origin-specific vector {target}"
        );
    }

    let routine = |name: &str, next: &str| {
        let start = boot
            .find(&format!("{name}:"))
            .unwrap_or_else(|| panic!("missing AArch64 routine {name}"));
        let end = boot[start..]
            .find(&format!("{next}:"))
            .unwrap_or_else(|| panic!("missing end marker {next} for {name}"));
        &boot[start..start + end]
    };
    let current_sp0 = routine("current_sync_sp0", "current_sync_spx");
    assert!(current_sp0.contains("msr     spsel, #1"));
    for body in [
        current_sp0,
        routine("current_sync_spx", "lower_sync_a32"),
        routine("lower_sync_a32", "irq_handler_sp"),
    ] {
        for instruction in [
            "mrs     x0, esr_el1",
            "mrs     x1, far_el1",
            "mrs     x2, elr_el1",
            "bl      fatal_aarch64_sync_exception",
        ] {
            assert!(
                body.contains(instruction),
                "fatal vector is missing {instruction}"
            );
        }
        assert!(!body.contains("eret"));
    }

    let lower = routine("lower_sync_a64", "restore_lower_el_frame");
    for instruction in [
        "sub     sp, sp, #0x310",
        "stp     x30, xzr, [sp, #240]",
        "stp     q30, q31, [sp, #0x2e0]",
        "str     x16, [sp, #0x308]",
        "bl      handle_syscall_simple",
        "bl      complete_linux_signal_syscall_return",
        "bl      syscall_should_advance_elr",
        "mov     x0, sp",
        "mrs     x1, esr_el1",
        "mrs     x2, far_el1",
        "mrs     x3, elr_el1",
        "bl      handle_aarch64_lower_el_sync",
        "b       restore_lower_el_frame",
    ] {
        assert!(
            lower.contains(instruction),
            "lower-EL frame is missing {instruction}"
        );
    }
    let fault_start = lower
        .find("lower_sync_a64_non_svc:")
        .expect("lower-EL non-SVC path");
    let fault = &lower[fault_start..];
    assert!(!fault.contains("mov     x0, #-38"));
    assert!(!fault.contains("str     x0, [sp, #0]"));
    assert!(!fault.contains("syscall_should_advance_elr"));
    assert!(!fault.contains("add     x0, x0, #4"));
    assert!(!fault.contains("msr     elr_el1, x0"));

    let restore = &boot[boot
        .find("restore_lower_el_frame:")
        .expect("lower-EL frame restore")..];
    assert!(restore.contains("add     sp, sp, #0x310"));
    assert!(restore.contains("eret"));

    let bridge_start = dispatch
        .find("pub extern \"C\" fn handle_aarch64_lower_el_sync(")
        .expect("AArch64 lower-EL Rust bridge");
    let bridge = braced_body(&dispatch[bridge_start..]);
    for token in [
        "aarch64_lower_el_sync(esr)",
        "Aarch64LowerElSync::MemoryFault(fault)",
        "Aarch64El0MemoryAccess::Read => LinuxMemoryFaultAccess::Read",
        "Aarch64El0MemoryAccess::Write => LinuxMemoryFaultAccess::Write",
        "Aarch64El0MemoryAccess::Execute => LinuxMemoryFaultAccess::Execute",
        "deliver_linux_synchronous_memory_fault(",
        "fatal_aarch64_sync_exception(esr, far, return_pc)",
    ] {
        assert!(
            bridge.contains(token),
            "exception bridge is missing {token}"
        );
    }
    let delivery = bridge
        .find("deliver_linux_synchronous_memory_fault(")
        .expect("synchronous fault delivery call");
    let delivery = &bridge[delivery..bridge.len().min(delivery + 180)];
    for argument in ["saved_frame", "return_pc", "far", "access"] {
        assert!(
            delivery.contains(argument),
            "delivery call is missing {argument}"
        );
    }
    assert!(lowlevel
        .contains("pub use arch::{boot, cpu, drivers, interrupt, serial, smp, thread, timer};"));

    let fatal_start = boot
        .find("pub extern \"C\" fn fatal_aarch64_sync_exception(")
        .expect("fatal AArch64 synchronous exception diagnostic");
    let fatal = braced_body(&boot[fatal_start..]);
    for token in [
        "fatal synchronous exception ESR=",
        "serial.write_hex(esr)",
        "serial.write_hex(far)",
        "serial.write_hex(elr)",
        "super::cpu::wait_for_interrupt()",
    ] {
        assert!(fatal.contains(token), "fatal diagnostic is missing {token}");
    }
    assert!(
        boot[fatal_start..fatal_start + boot[fatal_start..].find('{').unwrap()].contains(") -> !")
    );
}

fn assembly_routine<'a>(source: &'a str, name: &str) -> &'a str {
    let label = format!("{name}:");
    let start = source.find(&label).expect("assembly routine label");
    let end_marker = format!(".size {name},");
    let end = source[start..]
        .find(&end_marker)
        .expect("assembly routine end");
    &source[start..start + end]
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
fn aarch64_warning_gate_is_strict_and_target_scoped() {
    let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"));
    let docs = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/TESTING.md"
    ));

    assert!(makefile.contains("AARCH64_RUSTFLAGS = $(strip $(RUSTFLAGS) -D warnings)"));
    assert!(makefile
        .contains("aarch64-warning-check:\n\t@$(MAKE) build-test ARCH=aarch64-unknown-none"));
    assert!(makefile.contains(
        "RUSTFLAGS='$(AARCH64_RUSTFLAGS)' SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET)"
    ));
    assert!(makefile.contains(
        "SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET)"
    ));
    assert!(docs.contains("make aarch64-warning-check"));
    assert!(docs.contains("x86_64 and RISC-V64 warning policy is unchanged"));
}

#[test]
fn host_coverage_runs_outside_repository_cargo_configuration() {
    let coverage = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/run-host-coverage.sh"
    ));

    let change_directory = coverage
        .find("\ncd /\n")
        .expect("coverage wrapper changes outside the repository");
    let run_tarpaulin = coverage
        .find("\ncargo tarpaulin \\\n")
        .expect("coverage wrapper invokes Tarpaulin");
    assert!(
        change_directory < run_tarpaulin,
        "coverage wrapper must leave the repository before invoking Tarpaulin"
    );
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
    assert!(memory_body.contains("linux_process_memory::total_mapping_count()"));
    assert!(memory_body.contains("linux_process_resources"));
    assert!(memory_body.contains("resources.descriptors.len()"));
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
        "core::mem::take(&mut self.linux_process_resources)",
        "self.release_resource_clone(&resources.descriptors, &resources.objects)",
        "released_handles.extend(resources.timer_handles)",
        "core::mem::take(&mut self.linux_shared_memory)",
        "released_handles.push(record.handle)",
        "self.next_open_description_id = 1",
        "self.next_shared_memory_id = LINUX_SHM_ID_START",
    ] {
        assert!(
            method.contains(required),
            "missing reset operation: {required}"
        );
    }
    assert!(public.contains("linux_process_memory::reset_launch()"));
    assert!(public.contains("sys_handle_close(handle)"));
    assert!(public.contains("reset_linux_signal_timer_state()"));
    assert!(syscall.contains("timer_handles: Vec<u32>"));
    assert!(!syscall.contains("linux_timer_handles: Vec<u32>"));
    assert!(!method.contains("MemorySyscallState::new()"));
    assert!(!public.contains("MEMORY_SYSCALL_STATE = None"));

    let close_start = syscall
        .find("pub fn sys_close(fd: usize)")
        .expect("Linux close");
    let close = braced_body(&syscall[close_start..]);
    let close_helper_start = syscall
        .find("fn close_linux_fd_for_current_process(fd: usize)")
        .expect("central descriptor close helper");
    let close_helper = braced_body(&syscall[close_helper_start..]);
    assert!(close_helper.contains("remove_fd_entry(fd)"));
    let tracked_close = close
        .find("close_linux_fd_for_current_process(fd)")
        .expect("tracked descriptor close");
    let stdio_noop = close.find("fd <= 2").expect("untracked stdio no-op");
    assert!(
        tracked_close < stdio_noop,
        "tracked descriptors installed on stdin/stdout/stderr must be reclaimed"
    );
}

#[test]
fn linux_shared_memory_uses_nonnegative_process_ids_separate_from_compat_handles() {
    let syscall = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/syscall.rs"
    ));

    assert!(syscall.contains("const LINUX_SHM_ID_START: u32 = 1;"));

    let record_start = syscall
        .find("struct LinuxSharedMemoryRecord")
        .expect("shared-memory record");
    let record = braced_body(&syscall[record_start..]);
    assert!(record.contains("id: u32"));
    assert!(record.contains("handle: u32"));

    let register_start = syscall
        .find("fn register_shared_memory(")
        .expect("shared-memory registration");
    let register = braced_body(&syscall[register_start..]);
    assert!(register.contains("self.next_shared_memory_id"));
    assert!(register.contains("handle"));
    assert!(register.contains("Some(id)"));

    let get_start = syscall.find("pub fn sys_shmget(").expect("shmget");
    let get = braced_body(&syscall[get_start..]);
    assert!(get.contains("register_shared_memory(handle.0"));
    assert!(get.contains("Ok(id as usize)"));
    assert!(!get.contains("Ok(handle.0 as usize)"));

    let attach_start = syscall.find("pub fn sys_shmat(").expect("shmat");
    let attach = braced_body(&syscall[attach_start..]);
    assert!(attach.contains("shared_memory_handle(id as u32)"));

    let control_start = syscall.find("pub fn sys_shmctl(").expect("shmctl");
    let control = braced_body(&syscall[control_start..]);
    assert!(control.contains("remove_shared_memory_name(id as u32)"));
    assert!(control.contains("remove_shared_page_name(id as u32)"));
    assert!(!control.contains("compat::close_handle(HandleValue(record.handle))"));
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
    assert!(launcher.contains("RunElfActiveRequest<RunLaunchInputs, fxfs::FxfsPersistGuard>"));
    assert!(launcher.contains("linux_process_memory::register_root"));
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
        2,
        "normal exit and signal termination each complete the launcher exactly once"
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
    assert!(sys_exit.contains("exit_current_linux_process(exit_code, false)"));
    assert!(sys_exit.contains("return Ok(launch_id)"));
    let linux_exit_start = syscall
        .find("fn exit_current_linux_process(")
        .expect("shared Linux process exit helper");
    let linux_exit = braced_body(&syscall[linux_exit_start..]);
    assert!(linux_exit.contains("LinuxProcessExitOutcome::LaunchRoot"));
    assert!(linux_exit.contains("prepare_run_elf_return(exit_code)"));

    let exception_start = aarch64
        .find("lower_sync_a64:")
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
    assert!(sys_exit.contains("exit_current_linux_process(exit_code, false)"));

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
    assert!(group_exit.contains("exit_current_linux_process(exit_code, true)"));
    assert!(!group_exit.contains("linux_futex::reset()"));
    assert!(!group_exit.contains("linux_task::reset()"));

    let retire_tasks_start = task
        .find("fn retire_tasks(")
        .expect("shared task retirement");
    let retire_tasks = braced_body(&task[retire_tasks_start..]);
    let clear_tid = retire_tasks
        .find("exit_with_clear_child_tid")
        .expect("one-shot clear-child-TID transition");
    let retire = retire_tasks
        .find("runtime.tasks.retire(")
        .expect("task retirement");
    assert!(clear_tid < retire);

    let complete_start = task
        .find("fn complete_task_retirements(")
        .expect("retirement cleanup");
    let complete = braced_body(&task[complete_start..]);
    let remove_waiters = complete
        .find("linux_futex::remove_task_waiters")
        .expect("task futex cleanup");
    let zero_write = complete
        .find("linux_process_memory::copy_to_process(")
        .expect("checked clear-child-TID zero write");
    let wake = complete
        .find("linux_futex::wake_address(")
        .expect("pthread join futex wake");
    let terminate = complete
        .find("scheduler::scheduler().terminate_thread(")
        .expect("non-current task termination");
    assert!(remove_waiters < zero_write && zero_write < wake && wake < terminate);

    let finish_start = task
        .find("pub(crate) fn finish_current_without_el0_return() -> !")
        .expect("non-returning current task exit");
    let finish_current = braced_body(&task[finish_start..]);
    let finish = finish_current
        .find("finish_current_without_stack_free()")
        .expect("deferred current stack retirement");
    let schedule = finish_current
        .find("scheduler::schedule()")
        .expect("reschedule");
    let wait = finish_current
        .find("wait_for_interrupt()")
        .expect("no-runnable-thread wait");
    assert!(finish < schedule && schedule < wait);

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
        .find("// Synchronous exception from a lower EL using AArch64.")
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
        .find("lower_sync_a64:")
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
    let memory_root = launcher
        .find("linux_process_memory::register_root(pid)")
        .expect("root process memory registration");
    let root_paddr = launcher
        .find("linux_process_memory::current_root_paddr()")
        .expect("root translation address");
    let enter = launcher
        .find("user_process::switch_to_el0(entry, stack_top, root_paddr)")
        .expect("EL0 transfer");
    assert!(root < memory_root && memory_root < root_paddr && root_paddr < enter);

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

    assert!(syscall.contains("linux_process::current_pid()"));
    assert!(syscall.contains("linux_process::current_parent_pid()"));
    assert!(syscall.contains("linux_task::current_tid()"));
}

#[test]
fn aarch64_runtime_has_no_obsolete_warning_only_surface() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cpu = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/cpu.rs"))
        .expect("read AArch64 CPU module");
    let address = std::fs::read_to_string(repository.join("src/syscall/address_logic.rs"))
        .expect("read address wrappers");
    let address_shared =
        std::fs::read_to_string(repository.join("src/syscall/address_logic_shared.rs"))
            .expect("read shared address logic");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read Linux process memory runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");

    assert!(!cpu.contains("pub fn set_user_tls("));
    for helper in [
        "pub(crate) fn range_overlaps(",
        "pub(crate) fn range_within_window(",
        "pub(crate) fn linux_user_range_writable(",
        "pub(crate) fn linux_user_range_readable(",
    ] {
        assert!(
            !address.contains(helper),
            "obsolete address wrapper {helper}"
        );
    }
    for model_macro in [
        "smros_range_overlaps_body",
        "smros_linux_user_range_writable_body",
        "smros_linux_user_range_readable_body",
    ] {
        assert!(
            address_shared.contains(&format!(
                "#[cfg(not(target_os = \"none\"))]\nmacro_rules! {model_macro}"
            )),
            "model-only macro {model_macro} must not enter the kernel build"
        );
    }
    assert!(!process.contains("pub(crate) fn descriptors(&self)"));
    assert!(!process.contains("pub(crate) fn shared_attachments("));
    assert!(!task.contains("pub(crate) fn lookup_tid("));

    let clone_start = memory
        .find("pub(crate) struct LinuxSharedAttachmentClone")
        .expect("shared attachment clone");
    let clone_body = braced_body(&memory[clone_start..]);
    assert!(!clone_body.contains("pub prot:"));
    assert!(!clone_body.contains("pub flags:"));
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
    assert!(destination_validation.contains("linux_process_memory::user_range_writable("));
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
    assert!(copy.contains("slot.parent_tid.address"));
    assert!(copy.contains("slot.child_tid.address"));
    let first_checked_read = copy
        .find("linux_process_memory::copy_from_process(")
        .expect("clone TID snapshot read");
    assert!(copy.contains("linux_process_memory::copy_to_process("));
    assert!(tid_conversion < first_checked_read);
    assert!(copy_validation < first_checked_read);
    assert!(!copy.contains("core::ptr::read"));
    assert!(!copy.contains("reservation.tid as u32"));

    let launcher = run_elf
        .find("extern \"C\" fn run_elf_launcher_entry() -> !")
        .expect("ELF launcher entry");
    let launcher = &run_elf[launcher..];
    let register_memory = launcher
        .find("linux_process_memory::register_root(pid)")
        .expect("process memory ownership registration");
    let prepare = launcher
        .find("prepare_dynamic_loader(&request)")
        .expect("loader preparation");
    let root_paddr = launcher
        .find("linux_process_memory::current_root_paddr()")
        .expect("process root selection");
    let enter_el0 = launcher
        .find("user_process::switch_to_el0(")
        .expect("EL0 entry");
    assert!(register_memory < prepare && prepare < root_paddr && root_paddr < enter_el0);
    let prepare_start = run_elf
        .find("fn prepare_dynamic_loader(")
        .expect("dynamic loader preparation");
    let prepare_body = braced_body(&run_elf[prepare_start..]);
    assert!(prepare_body.contains("register_linux_initial_stack("));
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
fn aarch64_clone_child_installs_process_translation_root_before_el0() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 context switch");

    let clone_layout_start = task
        .find("pub(crate) struct Aarch64CloneStart")
        .expect("clone startup image");
    let clone_layout_end = task[clone_layout_start..]
        .find("#[derive(Clone, Copy)]\n    struct TidDestination")
        .expect("end of clone startup layout");
    let clone_layout = &task[clone_layout_start..clone_layout_start + clone_layout_end];
    assert!(clone_layout.contains("pub root_paddr: u64"));
    assert!(clone_layout
        .contains("assert!(core::mem::offset_of!(Aarch64CloneStart, root_paddr) == 0x330)"));

    let reserve_start = task
        .find("pub(crate) fn reserve_clone(")
        .expect("clone reservation");
    let reserve = braced_body(&task[reserve_start..]);
    let root = reserve
        .find("linux_process_memory::current_root_paddr()")
        .expect("current process translation root");
    let task_reservation = reserve
        .find(".reserve_child(parent.tgid, scheduler_id.0)")
        .expect("Linux task reservation");
    let scheduler_context = reserve
        .find(".set_linux_process_start(")
        .expect("suspended scheduler context configuration");
    let process_binding = reserve
        .find("bind_thread_process(scheduler_id, parent.tgid)")
        .expect("scheduler process binding");
    let startup_slot = reserve
        .find("runtime.clone_slots[reservation.slot] = LinuxCloneSlot")
        .expect("clone startup publication");
    assert!(root < task_reservation);
    assert!(task_reservation < scheduler_context);
    assert!(scheduler_context < process_binding);
    assert!(process_binding < startup_slot);
    assert!(reserve.contains("root_paddr,"));
    assert!(reserve.contains("runtime.tasks.rollback(reservation)"));

    let clone_start = switch
        .find("start_linux_clone_child:")
        .expect("clone child assembly entry");
    let clone_end = switch[clone_start..]
        .find(".size start_linux_clone_child")
        .expect("end of clone child assembly entry");
    let clone = &switch[clone_start..clone_start + clone_end];
    let load_root = clone
        .find("ldr     x17, [x16, #0x330]")
        .expect("clone process root load");
    let install_root = clone
        .find("msr     ttbr0_el1, x17")
        .expect("clone process root install");
    let first_dsb = clone.find("dsb     ish").expect("pre-TLBI barrier");
    let tlbi = clone
        .find("tlbi    vmalle1is")
        .expect("clone TLB invalidation");
    let second_dsb = tlbi
        + clone[tlbi..]
            .find("dsb     ish")
            .expect("post-TLBI barrier");
    let isb = clone.find("isb").expect("clone instruction barrier");
    let register_restore = clone
        .find("b       start_linux_child_register_restore")
        .expect("shared child register restore");
    assert!(load_root < install_root);
    assert!(install_root < first_dsb);
    assert!(first_dsb < tlbi);
    assert!(tlbi < second_dsb);
    assert!(second_dsb < isb);
    assert!(isb < register_restore);
}

#[test]
fn aarch64_glibc_fork_clone_sets_and_clears_child_tid() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");

    let clone = braced_body(
        &syscall[syscall
            .find("pub fn sys_clone(")
            .expect("clone implementation")..],
    );
    assert!(clone.contains("CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID"));
    assert!(clone.contains("linux_clone_tid_destination_valid(child_tid)"));
    assert!(clone.contains("flags & CLONE_CHILD_SETTID != 0"));
    assert!(clone.contains("flags & CLONE_CHILD_CLEARTID != 0"));

    let fork_backend = &process[process
        .find("impl LinuxForkOwnershipOps for Aarch64LinuxForkOps")
        .expect("production fork ownership operations")..];
    assert!(fork_backend.contains("copy_to_process("));
    assert!(fork_backend.contains("process.pid"));
    assert!(fork_backend.contains("child_tid"));
    assert!(fork_backend.contains("linux_tid_to_user_value"));

    let publish_fork_task = braced_body(
        &task[task
            .find("pub(crate) fn publish_fork_task(")
            .expect("fork task publication")..],
    );
    assert!(publish_fork_task.contains("clear_child_tid"));
    assert!(publish_fork_task.contains("set_clear_child_tid("));
    assert!(fork_backend.contains("child_tid_write"));
    assert!(fork_backend.contains("original.to_ne_bytes()"));
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
    assert!(readable.contains("linux_process_memory::user_range_readable(address, len)"));
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
    assert!(futex.contains("linux_process_memory::copy_from_current("));
    assert!(!futex.contains("core::ptr::read(uaddr as *const u32)"));
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
        .find("linux_process_memory::copy_from_current(uaddr")
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
        .find("linux_process_memory::copy_from_current(timeout_pointer")
        .expect("checked timeout read");
    assert!(timeout_revalidation < timeout_read);
    assert!(!deadline.contains("core::ptr::read_unaligned("));

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
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");

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
    assert!(task_logic.contains("pub(crate) fn linux_aarch64_signal_user_frame("));
    assert!(task_logic.contains("pub(crate) fn inherit_signal_mask("));
    assert!(task_logic.contains("pub(crate) fn route_signal("));
    assert!(task_logic.contains("pub(crate) fn process_signal_target("));
    assert!(task.contains("pub(crate) fn queue_task_signal("));
    assert!(task.contains("pub(crate) fn process_signal_target("));
    assert!(task.contains("pub(crate) fn with_current_signal_state<R>("));
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
    assert!(process.contains("type LinuxProcessSignalState = LinuxProcessSignalStateCore<"));
    assert!(!syscall.contains("static mut LINUX_PROCESS_PENDING"));
    assert!(syscall.contains("const LINUX_SIGNAL_TRAMPOLINE_BYTES: usize = PAGE_SIZE;"));
    assert!(!syscall.contains("LINUX_SIGNAL_INFO_STORAGE_BYTES"));
    let trampoline_start = syscall
        .find("fn ensure_linux_signal_trampoline()")
        .expect("signal trampoline allocation");
    let trampoline = braced_body(&syscall[trampoline_start..]);
    assert!(trampoline.contains("LINUX_SIGNAL_TRAMPOLINE_BYTES"));
    let process_queue_start = syscall
        .find("fn with_linux_process_signal_state<R>(")
        .expect("process signal-state guard");
    let process_queue = braced_body(&syscall[process_queue_start..]);
    assert!(process_queue.contains("crate::kernel_lowlevel::smp::is_boot_cpu()"));
    let process_mask = process_queue
        .find("mask_interrupts()")
        .expect("process queue interrupt mask");
    let process_access = process_queue
        .find("linux_process::with_signal_state(pid, operation)")
        .expect("serialized process signal-state access");
    assert!(process.contains("&mut state.signal_actions"));
    assert!(process.contains("&mut state.process_pending"));
    let process_restore = process_queue
        .find("restore_interrupts(interrupt_state)")
        .expect("process queue interrupt restore");
    assert!(process_mask < process_access && process_access < process_restore);

    let reset_start = syscall
        .find("pub fn reset_linux_signal_timer_state()")
        .expect("signal process reset");
    let reset = braced_body(&syscall[reset_start..]);
    assert!(reset.contains("linux_process::reset_current_signal_state()"));

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
        .find("linux_signal_disposition_for(validated_target.tgid, signum)")
        .expect("centralized directed disposition");
    let ignore = directed
        .find("LinuxSignalDisposition::Ignore => Ok(0)")
        .expect("ignored directed signal result");
    let build_record = directed
        .find("make_record()?")
        .expect("lazy queued record construction");
    let route_record = directed[build_record..]
        .find("linux_task::route_signal_and_complete_wait(")
        .expect("directed pending queue or wait completion")
        + build_record;
    let interrupt = directed
        .find("interrupt_linux_signal_target(")
        .expect("blocked target interruption");
    assert!(route < disposition && disposition < ignore && ignore < build_record);
    assert!(build_record < route_record && route_record < interrupt);
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
        .find("pending.take_eligible_reserved(")
        .expect("shared process signal selection");
    assert!(standard_selection > 0);
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
        .find("terminate_linux_process_by_signal(current.tgid, signum)")
        .expect("current process default action");
    assert!(dequeue < disposition && disposition < current && current < terminate);
    assert!(delivery.contains("install_linux_signal_handler("));
    assert!(delivery.contains("requeue_linux_signal(deliverable)"));
    let installer_start = syscall
        .find("fn install_linux_signal_handler(")
        .expect("shared signal frame installation");
    let installer = braced_body(&syscall[installer_start..]);
    assert!(installer.contains("linux_task::with_current_signal_state("));
    assert!(installer.contains("linux_aarch64_signal_user_frame(stack_top)"));
    assert!(installer.contains("signal_state.push_frame(frame)"));
    assert!(installer.contains("signal_state.alt_stack.flags = LINUX_SS_ONSTACK as u32"));

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
    assert!(kill.contains("linux_signal_disposition_for(target_pid, signum)"));
    assert!(kill.contains("LinuxSignalDisposition::Terminate"));
    assert!(kill.contains("queue_process_linux_signal_and_wake("));
    assert!(!kill.contains("terminate_process("));

    let rt_queue_start = syscall
        .find("pub fn sys_rt_sigqueueinfo(")
        .expect("process queued signal syscall");
    let rt_queue = braced_body(&syscall[rt_queue_start..]);
    let disposition = rt_queue
        .find("linux_signal_disposition_for(pid, sig)")
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
fn linux_signal_waits_block_and_restart_from_the_original_svc() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let futex = std::fs::read_to_string(repository.join("src/syscall/linux_futex.rs"))
        .expect("read Linux futex runtime");
    let context = std::fs::read_to_string(repository.join("src/syscall/linux_syscall_context.rs"))
        .expect("read Linux syscall frame context");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal implementation");
    let main = std::fs::read_to_string(repository.join("src/main.rs"))
        .expect("read timer interrupt boundary");

    for owned_state in [
        "pub(crate) struct LinuxSignalWait",
        "pub wait_mask: u64",
        "pub deadline: Option<u64>",
        "pub output_address: usize",
        "pub previous_mask: Option<u64>",
        "pub outcome: LinuxSignalWaitOutcome",
        "pub(crate) struct LinuxRestartBlock",
        "pub syscall_number: u64",
        "pub arguments: [u64; 6]",
        "pub svc_address: u64",
        "pub timeout: LinuxRestartTimeout",
        "pub signal_wait: Option<LinuxSignalWait>",
        "pub restart_block: Option<LinuxRestartBlock>",
        "pub restart: Option<LinuxRestartBlock>",
    ] {
        assert!(task_logic.contains(owned_state), "missing {owned_state}");
    }
    assert!(task_logic.contains("pub(crate) fn take_matching("));
    assert!(task_logic.contains("pub(crate) fn expire_signal_waits("));
    assert!(task_logic.contains("route_signal_and_complete_wait("));

    let timed_start = syscall
        .find("pub fn sys_rt_sigtimedwait(")
        .expect("rt_sigtimedwait syscall");
    let timed = braced_body(&syscall[timed_start..]);
    assert!(timed.contains("linux_read_u64_user(set)?"));
    assert!(timed.contains("LINUX_SIGNAL_INFO_BYTES"));
    assert!(timed.contains("LinuxSignalWait::timed("));
    assert!(timed.contains("LinuxBlockReason::SignalWait"));
    assert!(timed.contains("scheduler::schedule();"));
    assert!(timed.contains("LinuxSignalWaitOutcome::TimedOut => Err(SysError::EAGAIN)"));
    assert!(timed.contains("finish_linux_signal_wait("));
    assert!(!timed.contains("deliver_next_linux_signal("));

    let suspend_start = syscall
        .find("pub fn sys_rt_sigsuspend(")
        .expect("rt_sigsuspend syscall");
    let suspend = braced_body(&syscall[suspend_start..]);
    assert!(suspend.contains("LinuxSignalWait::suspend("));
    assert!(suspend.contains("LinuxBlockReason::SignalSuspend"));
    assert!(suspend.contains("scheduler::schedule();"));
    assert!(suspend.contains("Err(SysError::EINTR)"));

    for body in [timed, suspend] {
        let mask = body.find("mask_interrupts()").expect("outer IRQ mask");
        let install = body
            .find("install_current_signal_wait(")
            .expect("task wait installation");
        let block = body
            .find("linux_task::block_current(")
            .expect("task block transition");
        let scheduler_block = task
            .find("scheduler::scheduler().block_thread(")
            .expect("scheduler block transition");
        let schedule = body.find("scheduler::schedule();").expect("schedule call");
        assert!(mask < install && install < block && block < schedule);
        assert!(scheduler_block > 0);
    }

    let directed_start = syscall
        .find("fn queue_directed_linux_signal(")
        .expect("directed signal routing");
    let directed = braced_body(&syscall[directed_start..]);
    assert!(directed.contains("mask_interrupts()"));
    assert!(directed.contains("route_signal_and_complete_wait("));
    assert!(directed.contains("wake_blocked("));
    assert!(directed.contains("LinuxBlockReason::SignalWait"));
    assert!(directed.contains("LinuxBlockReason::SignalSuspend"));

    let timer_start = main
        .find("extern \"C\" fn timer_interrupt_handler()")
        .expect("timer interrupt handler");
    let timer = braced_body(&main[timer_start..]);
    let scheduler_tick = timer
        .find("scheduler().on_timer_tick()")
        .expect("scheduler tick");
    let signal_tick = timer
        .find("linux_task::on_timer_tick(")
        .expect("signal wait expiry");
    let futex_tick = timer
        .find("linux_futex::on_timer_tick(")
        .expect("futex expiry");
    assert!(scheduler_tick < signal_tick && signal_tick < futex_tick);

    let install_start = context
        .find("pub(crate) fn with_linux_syscall_frame(")
        .expect("syscall-frame installation");
    let install = braced_body(&context[install_start..]);
    assert!(install.contains("linux_futex::restartable_wait_operation("));
    assert!(install.contains("LinuxRestartBlock"));
    assert!(install.contains("frame_snapshot.regs[8]"));
    for register in 0..6 {
        assert!(
            install.contains(&format!("frame_snapshot.regs[{register}]")),
            "missing x{register} restart capture"
        );
    }
    assert!(install.contains(".checked_sub(4)"));
    assert!(install.contains("result != Err(SysError::EINTR)"));

    let delivery_start = syscall
        .find("fn deliver_next_linux_signal(")
        .expect("signal delivery");
    let delivery = braced_body(&syscall[delivery_start..]);
    assert!(delivery.contains("const LINUX_SA_RESTART: u64 = 0x1000_0000"));
    assert!(delivery.contains("take_restart_for_signal("));
    assert!(delivery.contains("action.flags & LINUX_SA_RESTART != 0"));
    assert!(delivery.contains("restart,"));

    let restore_start = syscall
        .find("fn restore_linux_signal_frame(")
        .expect("signal frame restoration");
    let restore = braced_body(&syscall[restore_start..]);
    assert!(restore.contains("set_exception_return_pc(frame.return_pc)"));
    assert!(!restore.contains("set_exception_return_pc(restart.svc_address)"));
    assert!(task_logic.contains("if let Some(restart) = frame.restart"));
    assert!(task_logic.contains("self.install_restart_block(restart)"));

    let replay_start = syscall
        .find("fn apply_linux_restart_block(")
        .expect("deferred restart replay helper");
    let replay = braced_body(&syscall[replay_start..]);
    for register in 0..6 {
        assert!(
            replay.contains(&format!("regs[{register}] = restart.arguments[{register}]")),
            "missing x{register} restart restoration"
        );
    }
    assert!(replay.contains("regs[8] = restart.syscall_number"));
    assert!(replay.contains("set_exception_return_pc(restart.svc_address)"));

    let completion_start = syscall
        .find("pub extern \"C\" fn complete_linux_signal_syscall_return(")
        .expect("signal syscall return completion");
    let completion = braced_body(&syscall[completion_start..]);
    let pending_delivery = completion
        .find("deliver_next_linux_signal(saved_regs, return_pc)")
        .expect("nested signal delivery decision");
    let staged_check = completion
        .find("signal_state.restart_block == Some(restart)")
        .expect("staged restart identity check");
    let replay = completion
        .find("apply_linux_restart_block(saved_regs, restart)")
        .expect("deferred restart replay");
    assert!(pending_delivery < staged_check && staged_check < replay);

    assert!(futex.contains("pub(crate) fn restartable_wait_operation("));
    assert!(futex.contains("linux_task::current_restart_timeout()"));
    assert!(futex.contains("linux_task::set_current_restart_timeout("));
    assert!(task.contains("pub(crate) fn install_current_signal_wait("));
    assert!(task.contains("pub(crate) fn on_timer_tick("));
}

#[test]
fn linux_standard_pending_records_are_bounded_and_shared_with_process_routing() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");

    assert!(task_logic.contains("pub(crate) struct LinuxPendingSignals"));
    assert!(task_logic
        .contains("pub standard_records: [LinuxPendingSignal; LINUX_REALTIME_SIGNAL_MIN]"));
    assert!(task_logic.contains("pub(crate) fn requeue_front("));
    assert!(process.contains("type LinuxProcessSignalState = LinuxProcessSignalStateCore<"));
    assert!(!syscall.contains("static mut LINUX_PROCESS_PENDING"));
    assert!(process.contains("process_pending.reset_in_place()"));
    assert!(!syscall.contains("static LINUX_PROCESS_PENDING_SIGNALS: AtomicU64"));
}

#[test]
fn linux_signal_runtime_resets_large_state_in_place() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");

    assert!(task_logic.contains("pub(crate) fn reset_in_place(&mut self)"));
    assert!(task_logic.contains("self.pending.reset_in_place()"));
    assert!(task_logic.contains("signal_states: [LinuxTaskSignalState::new(); N]"));
    assert_eq!(task_logic.matches("LinuxTaskSignalState::new()").count(), 1);
    assert!(!task_logic.contains("signal_states.fill(LinuxTaskSignalState::new())"));
    assert!(!task_logic.contains("= LinuxTaskSignalState::new()"));

    for method in [
        "pub(crate) fn register_root(",
        "pub(crate) fn reserve_child(",
        "pub(crate) fn inherit_signal_mask(",
        "pub(crate) fn rollback(",
        "pub(crate) fn exit_with_clear_child_tid(",
        "pub(crate) fn retire(",
        "pub(crate) fn reset(&mut self)",
    ] {
        let start = task_logic.find(method).expect("signal-state reset method");
        let body = braced_body(&task_logic[start..]);
        assert!(
            body.contains("reset_in_place()"),
            "{method} must reset signal state in place"
        );
    }

    let process_reset_start = process
        .find("pub(crate) fn reset_current_signal_state()")
        .expect("process pending reset");
    let process_reset = braced_body(&process[process_reset_start..]);
    assert!(process_reset.contains("process_pending.reset_in_place()"));
    assert!(!syscall.contains("*pending = LinuxPendingSignals::new()"));
}

#[test]
fn linux_fxfs_stat_preserves_file_identity_for_dynamic_loader_deduplication() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS service");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux stat implementation");

    assert!(fxfs.contains("pub fn attrs_with_object_id("));
    assert!(syscall.contains("fxfs::attrs_with_object_id("));
    assert!(syscall.contains("linux_fxfs_stat_identity(object_id)"));
    assert!(!syscall.contains("(stat_ptr + ST_INO_OFF) as *mut u64, 1"));
}

#[test]
fn linux_named_semaphore_publication_uses_atomic_fxfs_links_and_inode_mmap_identity() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS service");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux syscall implementation");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read Linux process memory implementation");

    assert!(fxfs.contains("pub fn link_file("));
    assert!(fxfs.contains("pub fn unlink_file("));
    assert!(fxfs.contains("pub fn delete_file("));
    assert!(fxfs.contains("pub fn release_unlinked_file("));

    let link_start = syscall.find("pub fn sys_linkat(").expect("linkat syscall");
    let link = braced_body(&syscall[link_start..]);
    assert!(link.contains("fxfs::link_file("));

    let unlink_start = syscall
        .find("pub fn sys_unlinkat(")
        .expect("unlinkat syscall");
    let unlink = braced_body(&syscall[unlink_start..]);
    assert!(unlink.contains("let object_id = fxfs::unlink_file(path.as_str())"));
    assert!(!unlink.contains("fxfs::attrs_with_object_id("));
    assert!(unlink.contains("linux_fxfs_object_is_open(object_id)"));
    assert!(unlink.contains("fxfs::release_unlinked_file(object_id)"));

    assert!(syscall.contains("object_id: file.cursor.object_id()"));
    assert!(syscall.contains("fxfs::release_unlinked_file(object_id)"));
    assert!(process_memory.contains("file_object_id: Option<u64>"));
    assert!(process_memory.contains("linux_shared_file_identity_matches(current, file_object_id)"));
}

#[test]
fn linux_sigtimedwait_selects_the_lowest_signal_across_pending_sources() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    assert!(task_logic.contains("pub(crate) enum LinuxPendingSignalSource"));
    assert!(task_logic.contains("pub(crate) fn select_linux_pending_signal("));
    assert!(task.contains("pub(crate) fn peek_current_matching_signal("));
    assert!(task.contains("pub(crate) fn take_current_matching_signal("));

    let timed_start = syscall
        .find("pub fn sys_rt_sigtimedwait(")
        .expect("rt_sigtimedwait syscall");
    let timed = braced_body(&syscall[timed_start..]);
    let task_peek = timed
        .find("peek_current_matching_signal(")
        .expect("task pending peek");
    let process_peek = timed
        .find("peek_process_linux_signal_matching(")
        .expect("process pending peek");
    let selection = timed
        .find("select_linux_pending_signal(")
        .expect("global pending selection");
    let take = timed
        .find("take_selected_linux_signal(")
        .expect("source-aware pending take");
    assert!(task_peek < selection && process_peek < selection && selection < take);
}

#[test]
fn linux_sigtimedwait_requeues_the_original_source_when_copyout_becomes_invalid() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    assert!(task_logic.contains("pub signal_source: Option<LinuxPendingSignalSource>"));
    assert!(syscall.contains("fn take_selected_linux_signal("));
    assert!(syscall.contains("fn finish_linux_signal_wait("));
    let finish_start = syscall
        .find("fn finish_linux_signal_wait(")
        .expect("signal wait completion helper");
    let finish = braced_body(&syscall[finish_start..]);
    let copyout = finish
        .find("copy_linux_signal_wait_info(")
        .expect("checked copyout helper");
    let requeue = finish
        .find("requeue_linux_signal(")
        .expect("source-preserving requeue");
    let fault = finish.find("return Err(error)").expect("copyout fault");
    assert!(copyout < requeue && requeue < fault);
}

#[test]
fn linux_sigtimedwait_rollback_uses_bounded_source_local_reservations() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    assert!(task_logic.contains("pub standard_reserved: u64"));
    assert!(task_logic.contains("pub realtime_reserved: usize"));
    assert!(task_logic.contains("pub(crate) enum LinuxPendingSignalReservation"));
    assert!(task_logic.contains("pub signal_reservation: Option<LinuxPendingSignalReservation>"));
    assert!(task_logic.contains("self.realtime_len + self.realtime_reserved"));
    assert!(task_logic.contains("pub(crate) fn rollback_reservation("));
    assert!(task_logic.contains("pub(crate) fn handoff_process_pending_signal("));
    assert!(task.contains("pub(crate) fn handoff_process_pending_signal("));

    let handoff_start = syscall
        .find("fn update_process_linux_signals_and_handoff(")
        .expect("bounded process signal handoff helper");
    let handoff = braced_body(&syscall[handoff_start..]);
    assert!(handoff.contains("[None; linux_task::LINUX_TASK_LIMIT]"));
    assert!(handoff.contains("for wake in &mut wakes"));
    let pending_scope = handoff
        .find("with_linux_process_pending(")
        .expect("one outer process-pending critical section");
    let completion = handoff
        .find("linux_task::handoff_process_pending_signal(tgid, pending)")
        .expect("source-local pending handoff");
    let wake = handoff
        .rfind("linux_task::wake_blocked(")
        .expect("scheduler wake after handoff");
    assert!(pending_scope < completion && completion < wake);

    let requeue_start = syscall
        .find("fn requeue_linux_signal(")
        .expect("source-aware requeue helper");
    let requeue = braced_body(&syscall[requeue_start..]);
    assert!(requeue.contains("rollback_reservation("));
    assert!(requeue.contains("update_process_linux_signals_and_handoff("));
    assert!(!requeue.contains("let _ ="));

    let commit_start = syscall
        .find("fn commit_linux_signal(")
        .expect("source-aware commit helper");
    let commit = braced_body(&syscall[commit_start..]);
    assert!(commit.contains("update_process_linux_signals_and_handoff("));

    for delivery_failure in syscall.match_indices("requeue_linux_signal(deliverable)") {
        let suffix = &syscall[delivery_failure.0..];
        assert!(!suffix.starts_with("requeue_linux_signal(deliverable);"));
    }
}

#[test]
fn linux_handler_delivery_reserves_pending_capacity_until_the_frame_is_ready() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    assert!(task_logic.contains("pub(crate) fn take_eligible_reserved("));
    assert!(task_logic.contains("pub(crate) fn take_unblocked_reserved("));
    assert!(task.contains("pub(crate) fn take_current_unblocked_signal("));

    let process_take_start = syscall
        .find("fn take_process_linux_signal(")
        .expect("process signal reserved take");
    let process_take = braced_body(&syscall[process_take_start..]);
    assert!(process_take.contains("pending.take_eligible_reserved("));

    let unblocked_start = syscall
        .find("fn take_unblocked_linux_signal(")
        .expect("unblocked signal take");
    let unblocked = braced_body(&syscall[unblocked_start..]);
    assert!(unblocked.contains("linux_task::take_current_unblocked_signal("));
    assert!(unblocked.matches("reservation: Some(reservation)").count() >= 2);

    let delivery_start = syscall
        .find("fn deliver_next_linux_signal(")
        .expect("signal delivery helper");
    let delivery = braced_body(&syscall[delivery_start..]);
    let install = delivery
        .find("install_linux_signal_handler(")
        .expect("handler frame setup");
    let commit = delivery
        .rfind("commit_linux_signal(deliverable)")
        .expect("successful handler reservation commit");
    assert!(install < commit);
    assert!(delivery.contains("requeue_linux_signal(deliverable)"));
}

#[test]
fn linux_sigtimedwait_copyout_masks_irqs_across_validation_and_write() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");

    let copy_start = syscall
        .find("fn copy_linux_signal_wait_info(")
        .expect("short IRQ-masked sigtimedwait copyout helper");
    let copy = braced_body(&syscall[copy_start..]);
    let mask = copy.find("mask_interrupts()").expect("copyout IRQ mask");
    let validation = copy
        .find("linux_signal_user_range_writable(")
        .expect("copyout range validation");
    let write = copy
        .find("linux_copy_to_user(output_address, &record.info)?")
        .expect("complete checked siginfo write");
    let entry_fence = copy
        .find("compiler_fence(Ordering::SeqCst)")
        .expect("copyout entry compiler fence");
    let exit_fence = copy
        .rfind("compiler_fence(Ordering::SeqCst)")
        .expect("copyout exit compiler fence");
    let restore = copy
        .rfind("restore_interrupts(interrupt_state)")
        .expect("unconditional copyout IRQ restore");
    assert_ne!(entry_fence, exit_fence);
    assert!(mask < entry_fence);
    assert!(entry_fence < validation && validation < write);
    assert!(write < exit_fence && exit_fence < restore);
    assert!(!copy.contains("core::ptr::"));
    assert!(!copy.contains("linux_task::"));
    assert!(!copy.contains("with_linux_process_pending("));

    let finish_start = syscall
        .find("fn finish_linux_signal_wait(")
        .expect("signal wait completion helper");
    let finish = braced_body(&syscall[finish_start..]);
    let copyout = finish
        .find("copy_linux_signal_wait_info(")
        .expect("checked copyout call");
    let rollback = finish
        .find("requeue_linux_signal(deliverable)")
        .expect("copyout failure rollback");
    assert!(copyout < rollback);
}

#[test]
fn linux_process_pending_process_state_access_has_compiler_barriers() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux signal runtime");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let guard_start = syscall
        .find("fn with_linux_process_signal_state<R>(")
        .expect("process pending queue guard");
    let guard = braced_body(&syscall[guard_start..]);
    let mask = guard.find("mask_interrupts()").expect("process IRQ mask");
    let first_fence = guard
        .find("compiler_fence(Ordering::SeqCst)")
        .expect("compiler fence after IRQ mask");
    let access = guard
        .find("linux_process::with_signal_state(pid, operation)")
        .expect("process-owned signal-state access");
    assert!(process.contains("&mut state.signal_actions"));
    assert!(process.contains("&mut state.process_pending"));
    let second_fence = guard
        .rfind("compiler_fence(Ordering::SeqCst)")
        .expect("compiler fence before IRQ restore");
    let restore = guard
        .find("restore_interrupts(interrupt_state)")
        .expect("process IRQ restore");
    assert!(mask < first_fence);
    assert!(first_fence < access && access < second_fence);
    assert!(second_fence < restore);
    assert_ne!(first_fence, second_fence);
}

#[test]
fn aarch64_kernel_threads_reserve_fork_transaction_headroom() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread runtime");
    let declaration = thread
        .lines()
        .find(|line| {
            line.trim_start()
                .starts_with("pub const DEFAULT_STACK_SIZE:")
        })
        .expect("AArch64 default kernel stack size");
    let value = declaration
        .split_once('=')
        .map(|(_, value)| value.trim().trim_end_matches(';').replace('_', ""))
        .expect("AArch64 default kernel stack value");
    let stack_size = value
        .strip_prefix("0x")
        .map(|value| usize::from_str_radix(value, 16))
        .unwrap_or_else(|| value.parse())
        .expect("numeric AArch64 default kernel stack value");

    assert!(
        stack_size >= 0x1_0000,
        "AArch64 fork transaction needs 64 KiB kernel-stack headroom; got {stack_size:#x}"
    );
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
        (
            "irq_handler_lower:",
            "// Synchronous exception from a lower EL using AArch64.",
        ),
        ("lower_sync_a64:", "\n\"#,"),
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
        "mrs     x17, ttbr0_el1",
        "str     x17, [x16, #0x130]",
        "mrs     x17, fpcr",
        "str     x17, [x16, #0x138]",
        "mrs     x17, fpsr",
        "str     x17, [x16, #0x140]",
        "ldr     x17, [x16, #0x110]",
        "msr     sp_el0, x17",
        "ldr     x17, [x16, #0x118]",
        "msr     elr_el1, x17",
        "ldr     x17, [x16, #0x120]",
        "msr     spsr_el1, x17",
        "ldr     x17, [x16, #0x128]",
        "msr     tpidr_el0, x17",
        "ldr     x17, [x16, #0x130]",
        "msr     ttbr0_el1, x17",
        "ldr     x17, [x16, #0x138]",
        "msr     fpcr, x17",
        "ldr     x17, [x16, #0x140]",
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
        let offset = 0x150 + pair * 0x20;
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

#[test]
fn linux_sleeps_expire_or_interrupt_only_the_matching_task() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall_logic = std::fs::read_to_string(repository.join("src/syscall/syscall_logic.rs"))
        .expect("read Linux syscall logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux syscall runtime");

    assert!(task_logic.contains("sleep_waits: [Option<LinuxSleepWait>; N]"));
    assert!(task_logic.contains("pub(crate) struct LinuxSleepRelative"));
    assert!(task_logic.contains("relative: Option<LinuxSleepRelative>"));
    assert!(task_logic.contains("pub(crate) const fn relative_waiting("));
    assert!(task_logic.contains("pub(crate) fn expire_sleeps("));
    assert!(task_logic.contains("pub(crate) fn interrupt_sleep("));
    assert!(task_logic.contains("self.signal_states[slot].mask & bit != 0"));
    assert!(task.contains("pub(crate) fn install_current_sleep("));
    assert!(task.contains("pub(crate) fn take_current_sleep_outcome("));
    assert!(task.contains("pub(crate) fn cancel_current_sleep("));

    let remaining_start = task_logic
        .find("pub(crate) fn linux_sleep_remaining_timespec(")
        .expect("relative remaining-time helper");
    let remaining = braced_body(&task_logic[remaining_start..]);
    assert!(remaining.contains("now.saturating_sub(started_at)"));
    assert!(remaining.contains("requested_nanoseconds"));

    let pending_start = syscall
        .find("fn linux_sleep_has_deliverable_pending_signal(")
        .expect("pending signal sleep check");
    let pending = braced_body(&syscall[pending_start..]);
    assert!(pending.contains("linux_task::current_task()?"));
    assert!(pending.contains("linux_task::with_current_signal_state("));
    assert!(pending.contains("peek_unblocked().is_some()"));
    assert!(pending.contains("with_linux_process_pending("));
    assert!(pending.contains("peek_eligible("));
    assert!(pending
        .contains("linux_task::process_signal_target(current.tgid, signum) == Some(current)"));
    assert!(!pending.contains("take_"));
    assert!(!pending.contains("reserve"));
    assert!(!pending.contains("commit"));

    let timer_start = task
        .find("pub(crate) fn on_timer_tick(")
        .expect("Linux task timer hook");
    let timer = braced_body(&task[timer_start..]);
    let cpu0 = timer.find("current_cpu_id() == 0").expect("CPU0 guard");
    let expire = timer.find("expire_sleeps(now)").expect("sleep expiry");
    assert!(cpu0 < expire);
    let sleep_expiry = &timer[expire..];
    let wake = sleep_expiry
        .find("wake_blocked(tid, scheduler_thread, reason)")
        .expect("exact scheduler wake");
    let cancel = sleep_expiry
        .find("cancel_sleep(tid, scheduler_thread)")
        .expect("failed wake cleanup");
    assert!(wake < cancel);

    let interrupt_start = syscall
        .find("fn interrupt_linux_signal_target(")
        .expect("signal interruption helper");
    let interrupt = braced_body(&syscall[interrupt_start..]);
    assert!(interrupt.contains("LinuxBlockReason::Sleep"));
    assert!(interrupt.contains("linux_task::interrupt_sleep("));
    assert!(interrupt.contains("linux_task::wake_blocked("));
    assert!(interrupt.contains("linux_task::cancel_sleep("));
    let interrupt_sleep = interrupt
        .find("linux_task::interrupt_sleep(")
        .expect("sleep interruption publication");
    let wake_blocked = interrupt
        .find("linux_task::wake_blocked(")
        .expect("sleep wake");
    let cancel_sleep = interrupt
        .find("linux_task::cancel_sleep(")
        .expect("failed wake cleanup");
    assert!(interrupt_sleep < wake_blocked && wake_blocked < cancel_sleep);

    let process_start = syscall
        .find("fn queue_process_linux_signal_and_wake(")
        .expect("process signal routing caller");
    let process = braced_body(&syscall[process_start..]);
    let queue_process = process
        .find("queue_process_linux_signal_for(tgid, record)?")
        .expect("process signal queue");
    let interrupt_process = process
        .find("interrupt_linux_signal_target(target, record.signum)")
        .expect("process sleep interruption");
    assert!(queue_process < interrupt_process);

    let directed_start = syscall
        .find("fn queue_directed_linux_signal(")
        .expect("directed signal routing caller");
    let directed = braced_body(&syscall[directed_start..]);
    let route = directed
        .find("route_signal_and_complete_wait")
        .expect("directed signal routing");
    let interrupt_directed = directed
        .find("interrupt_linux_signal_target(target, record.signum)")
        .expect("directed sleep interruption");
    assert!(route < interrupt_directed);

    assert!(syscall.contains("const LINUX_TIMER_ABSTIME: usize = 1"));
    assert!(syscall.contains("pub fn sys_nanosleep_linux(req: usize)"));
    assert!(syscall.contains("fn sys_nanosleep_linux_with_rem(req: usize, rem: usize)"));
    let dispatch_start = syscall
        .find("pub fn dispatch_linux_syscall(")
        .expect("Linux syscall dispatcher");
    let dispatch = braced_body(&syscall[dispatch_start..]);
    assert!(
        dispatch.contains("ARM64_SYS_NANOSLEEP => sys_nanosleep_linux_with_rem(args[0], args[1])")
    );
    assert!(dispatch.contains(
        "ARM64_SYS_CLOCK_NANOSLEEP => sys_clock_nanosleep(args[0], args[1], args[2], args[3])"
    ));

    let flags_start = syscall_logic
        .find("fn linux_clock_nanosleep_flags_valid(")
        .expect("clock nanosleep flag wrapper");
    let flags = braced_body(&syscall_logic[flags_start..]);
    assert!(flags.contains("smros_linux_clock_nanosleep_flags_valid_body!"));

    let sleep_until_start = syscall
        .find("fn linux_sleep_until(")
        .expect("blocking Linux sleep helper");
    let sleep_until = braced_body(&syscall[sleep_until_start..]);
    let validate = sleep_until
        .find("linux_sleep_user_range_writable(")
        .expect("remaining buffer validation");
    let mask = sleep_until.find("mask_interrupts()").expect("IRQ mask");
    let pending_check = sleep_until
        .find("linux_sleep_has_deliverable_pending_signal()")
        .expect("pending signal check");
    let install = sleep_until
        .find("install_current_sleep(")
        .expect("sleep publication");
    let block = sleep_until
        .find("block_current(LinuxBlockReason::Sleep)")
        .expect("task block");
    let schedule = sleep_until
        .find("scheduler::schedule();")
        .expect("schedule");
    assert!(
        validate < mask
            && mask < pending_check
            && pending_check < install
            && install < block
            && block < schedule
    );
    let pending_path = &sleep_until[pending_check..install];
    assert!(pending_path.contains("linux_write_sleep_remaining(rem, wait)"));
    assert!(pending_path.contains("Err(SysError::EINTR)"));
    assert!(!pending_path.contains("take_"));
    assert!(!pending_path.contains("reserve"));
    assert!(!pending_path.contains("commit"));
    assert!(sleep_until.contains("LinuxSleepOutcome::Completed => Ok(0)"));
    assert!(sleep_until.contains("LinuxSleepOutcome::Interrupted"));
    assert!(sleep_until.contains("linux_write_sleep_remaining(rem, wait)?"));
    assert!(sleep_until.contains("Err(SysError::EINTR)"));
    assert!(sleep_until.contains("let _ = linux_task::cancel_current_sleep();"));

    let write_remaining_start = syscall
        .find("fn linux_write_sleep_remaining(")
        .expect("remaining time copyout helper");
    let write_remaining = braced_body(&syscall[write_remaining_start..]);
    assert!(write_remaining.contains("wait.relative.ok_or(SysError::EINVAL)?"));
    assert!(write_remaining.contains("linux_sleep_remaining_timespec("));

    let clock_nanosleep_start = syscall
        .find("pub fn sys_clock_nanosleep(")
        .expect("Linux clock nanosleep syscall");
    let clock_nanosleep = braced_body(&syscall[clock_nanosleep_start..]);
    assert!(clock_nanosleep.contains("syscall_logic::linux_clock_id_supported(clockid)"));
    assert!(
        clock_nanosleep.contains("linux_clock_nanosleep_flags_valid(flags, LINUX_TIMER_ABSTIME)")
    );
    assert!(clock_nanosleep.contains("let absolute = flags & LINUX_TIMER_ABSTIME != 0"));
    assert!(clock_nanosleep.contains("linux_sleep_absolute_deadline_ticks("));
    assert!(clock_nanosleep.contains("linux_sleep_relative_deadline_ticks("));
    assert!(clock_nanosleep.contains("linux_sleep_timespec_nanoseconds("));
    assert!(clock_nanosleep
        .contains("LinuxSleepWait::relative_waiting(deadline, now, requested_nanoseconds)"));
    assert!(clock_nanosleep.contains("LinuxSleepWait::waiting(deadline)"));
    assert!(clock_nanosleep.contains("linux_sleep_until(wait, rem)"));

    let nanosleep_start = syscall
        .find("pub fn sys_nanosleep_linux(")
        .expect("Linux nanosleep compatibility wrapper");
    let nanosleep = braced_body(&syscall[nanosleep_start..]);
    assert!(nanosleep.contains("sys_nanosleep_linux_with_rem(req, 0)"));
    assert!(!nanosleep.contains("if req == 0"));
    assert!(!nanosleep.contains("Ok(0)"));

    let nanosleep_with_rem_start = syscall
        .find("fn sys_nanosleep_linux_with_rem(")
        .expect("Linux nanosleep syscall helper");
    let nanosleep_with_rem = braced_body(&syscall[nanosleep_with_rem_start..]);
    assert!(nanosleep_with_rem.contains("linux_sleep_relative_deadline_ticks("));
    assert!(nanosleep_with_rem.contains("linux_sleep_timespec_nanoseconds("));
    assert!(nanosleep_with_rem
        .contains("LinuxSleepWait::relative_waiting(deadline, now, requested_nanoseconds)"));
    assert!(nanosleep_with_rem.contains("linux_sleep_until(wait, rem)"));
    assert!(!nanosleep_with_rem.contains("if req == 0"));
    assert!(!nanosleep_with_rem.contains("Ok(0)"));
}

#[test]
fn linux_memory_and_loader_are_process_owned() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let modules = std::fs::read_to_string(repository.join("src/syscall/mod.rs"))
        .expect("read syscall modules");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read process memory runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let run_elf = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");

    assert!(modules.contains("pub(crate) mod linux_process_memory;"));
    assert!(process_memory.contains("struct LinuxProcessMemory"));
    assert!(process_memory.contains("address_space: Aarch64AddressSpace"));
    assert!(process_memory.contains("pub(crate) fn with_current"));
    assert!(process_memory.contains("pub(crate) fn copy_from_current"));
    assert!(process_memory.contains("pub(crate) fn copy_to_current"));
    assert!(process_memory.contains("pub(crate) fn zero_current"));
    assert!(process_memory.contains("pub(crate) fn current_root_paddr"));

    let memory_state = braced_body(
        &syscall[syscall
            .find("struct MemorySyscallState")
            .expect("global compatibility state")..],
    );
    assert!(!memory_state.contains("linux_mappings:"));
    assert!(!memory_state.contains("linux_initial_stack:"));
    assert!(!memory_state.contains("next_linux_addr:"));
    assert!(!memory_state.contains("brk:"));

    assert!(run_elf.contains("linux_process_memory::register_root"));
    assert!(run_elf.contains("linux_process_memory::copy_to_current"));
    assert!(run_elf.contains("linux_process_memory::zero_current"));
    assert!(run_elf.contains("linux_process_memory::current_root_paddr"));
    assert!(!run_elf.contains("core::ptr::write_bytes(dest as *mut u8"));
    assert!(!run_elf.contains("core::ptr::copy_nonoverlapping(\n                bytes.as_ptr()"));
    assert!(!run_elf.contains("self.sp as *mut u8"));
    assert!(!run_elf.contains("self.sp as *mut u64"));
    assert!(!run_elf.contains("user_process::switch_to_el0(entry, stack_top, 0)"));
}

#[test]
fn fxfs_bootstrap_provides_posix_shared_memory_directory() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS service");
    let bootstrap_start = fxfs
        .find("self.suspend_persist();\n        let result = (|| {")
        .expect("FxFS fresh-volume bootstrap");
    let bootstrap = braced_body(&fxfs[bootstrap_start..]);
    let dev = bootstrap
        .find("self.create_dir(\"/dev\")?")
        .expect("POSIX device directory");
    let shm = bootstrap
        .find("self.create_dir(\"/dev/shm\")?")
        .expect("POSIX shared-memory directory");

    assert!(dev < shm);
}

#[test]
fn fxfs_forced_persist_bypasses_suspension_and_preserves_failed_pending_work() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS service");

    let persist_start = fxfs
        .find("fn persist(&mut self)")
        .expect("ordinary persistence path");
    let persist = braced_body(&fxfs[persist_start..]);
    assert!(persist.contains("if self.persist_suspended > 0"));
    assert!(persist.contains("self.persist_pending = true;"));
    assert!(persist.contains("let _ = self.force_persist();"));

    let force_start = fxfs
        .find("fn force_persist(&mut self) -> Result<(), FxfsError>")
        .expect("fallible forced persistence path");
    let force = braced_body(&fxfs[force_start..]);
    let pending = force
        .find("let pending = self.persist_pending;")
        .expect("pending state snapshot");
    let sync = force
        .find("self.sync_to_block()")
        .expect("full image commit");
    let clear = force
        .find("self.persist_pending = false;")
        .expect("successful commit clears pending work");
    assert!(pending < sync && sync < clear);
    assert!(force.contains("self.persist_pending = pending;"));
    assert!(force.contains("self.last_sync_ok = true;"));
    assert!(force.contains("self.last_sync_ok = false;"));
    assert!(force.contains("self.last_storage_error = Some(err);"));
    assert!(force.contains("Err(err)"));

    let public_force_start = fxfs
        .find("pub fn force_persist() -> Result<(), FxfsError>")
        .expect("public forced persistence API");
    let public_force = braced_body(&fxfs[public_force_start..]);
    assert!(public_force.contains("state().force_persist()"));

    let flush_start = fxfs
        .find("pub fn flush_persist()")
        .expect("best-effort compatibility flush");
    let flush = braced_body(&fxfs[flush_start..]);
    assert!(flush.contains("let _ = force_persist();"));
}

#[test]
fn run_elf_batches_fxfs_persistence_for_the_exact_launch_lifecycle() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");

    let compact_launcher: String = launcher
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact_launcher.contains(
        "typeActiveRun=user_logic::RunElfActiveRequest<RunLaunchInputs,fxfs::FxfsPersistGuard>;"
    ));

    let spawn_start = launcher
        .find("pub fn spawn_observed(")
        .expect("observed ELF spawn");
    let spawn = braced_body(&launcher[spawn_start..]);
    let accept = spawn
        .find("run_elf_start_transition(state, request")
        .expect("accepted launch transition");
    let suspend = spawn
        .find("let persist_guard = fxfs::suspend_persist();")
        .expect("persistence suspension");
    let attach = spawn
        .find("run_elf_attach_resource_transition(state, launch_id, persist_guard)")
        .expect("launch-ID-aware guard attachment");
    let bind = spawn
        .find("RUN_CPU_BINDINGS.bind(cpu, launch_id)")
        .expect("CPU binding");
    let create = spawn
        .find("create_thread_on_cpu(")
        .expect("launcher thread publication");
    assert!(accept < suspend && suspend < attach && attach < bind && bind < create);
    assert!(spawn.contains("drop(error.into_resource());"));
    assert!(spawn.contains("clear_launch_state_without_outcome(LINUX_RUNTIME_CPU, launch_id);"));

    let clear_start = launcher
        .find("fn clear_launch_state_without_outcome(")
        .expect("launch cleanup");
    let clear = braced_body(&launcher[clear_start..]);
    assert!(clear.contains("let completion = with_run_state("));
    assert!(clear.contains("drop(completion);"));

    let complete_start = launcher
        .find("fn complete_active_run(")
        .expect("launch completion");
    let complete = braced_body(&launcher[complete_start..]);
    let parts = complete
        .find("active_request.into_parts()")
        .expect("owned launch decomposition");
    let release = complete.find("drop(resource);").expect("guard release");
    let end_tick = complete
        .find("timer::get_tick_count()")
        .expect("completion timestamp");
    let dispatch = complete
        .find("dispatch_outcome(request.observer, outcome)")
        .expect("observer dispatch");
    assert!(parts < release && release < end_tick && end_tick < dispatch);
}

#[test]
fn linux_sync_syscalls_force_fxfs_persistence() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let sync_start = syscall.find("pub fn sys_sync()").expect("sync syscall");
    let sync = braced_body(&syscall[sync_start..]);
    assert!(sync.contains("let _ = fxfs::force_persist();"));
    assert!(sync.contains("Ok(0)"));

    let fsync_start = syscall
        .find("pub fn sys_fsync(fd: usize)")
        .expect("fsync syscall");
    let fsync = braced_body(&syscall[fsync_start..]);
    let validate = fsync
        .find("if !linux_fd_is_file_or_pipe(fd)")
        .expect("descriptor validation");
    let force = fsync
        .find("fxfs::force_persist()")
        .expect("forced FxFS commit");
    assert!(validate < force);
    assert!(fsync.contains("map_err(|_| SysError::EIO)?"));
    assert!(fsync.contains("Err(SysError::ENODEV)"));

    let fdatasync_start = syscall
        .find("pub fn sys_fdatasync(fd: usize)")
        .expect("fdatasync syscall");
    let fdatasync = braced_body(&syscall[fdatasync_start..]);
    assert!(fdatasync.contains("sys_fsync(fd)"));

    let sync_range_start = syscall
        .find("pub fn sys_sync_file_range(")
        .expect("sync_file_range syscall");
    let sync_range = braced_body(&syscall[sync_range_start..]);
    assert!(sync_range.contains("sys_fsync(fd)"));
}

#[test]
fn aarch64_directory_open_flags_match_staged_glibc() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let docker =
        std::fs::read_to_string(repository.join("src/user_level/services/docker_compat.rs"))
            .expect("read Docker compatibility service");
    let shell = std::fs::read_to_string(repository.join("src/user_level/services/user_shell.rs"))
        .expect("read user shell");
    let verification = std::fs::read_to_string(repository.join("verification/syscall/src/lib.rs"))
        .expect("read syscall verification harness");

    assert!(syscall.contains("const LINUX_O_DIRECTORY: usize = 0o40000;"));
    let allowed_start = syscall
        .find("const LINUX_OPEN_ALLOWED_FLAGS: usize =")
        .expect("Linux open flag allowlist");
    let allowed_end = syscall[allowed_start..]
        .find(';')
        .expect("Linux open flag allowlist end")
        + allowed_start;
    let allowed = &syscall[allowed_start..allowed_end];
    for flag in ["LINUX_O_NONBLOCK", "LINUX_O_DIRECTORY", "LINUX_O_CLOEXEC"] {
        assert!(allowed.contains(flag), "missing glibc opendir flag {flag}");
    }
    let openat_start = syscall
        .find("pub fn sys_openat(")
        .expect("Linux openat implementation");
    let openat = braced_body(&syscall[openat_start..]);
    assert!(openat.contains("linux_open_is_directory(flags, LINUX_O_DIRECTORY)"));
    let fstat_start = syscall
        .find("pub fn sys_fstat(")
        .expect("Linux fstat implementation");
    let fstat = braced_body(&syscall[fstat_start..]);
    assert!(fstat.contains(
        "let mode = if linux_fd_is_dir(fd) {\n        0o040755\n    } else {\n        0o100644\n    };\n    linux_write_stat(stat_ptr, mode)"
    ));
    assert!(docker.contains("const O_DIRECTORY: usize = 0o40000;"));
    let compact_shell: String = shell
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact_shell
        .contains("crate::syscall::sys_openat(usize::MAX-99,dir_path.as_ptr()asusize,0o40000,0)"));
    assert!(verification.contains("pub const LINUX_O_DIRECTORY: usize = 0o40000;"));
}

#[test]
fn linux_process_memory_mutations_are_transactional_and_bounded() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read process memory runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let map_start = process_memory
        .find("    fn map(\n")
        .expect("process mapping implementation");
    let map = braced_body(&process_memory[map_start..]);
    assert!(map.contains("allocate_unmapped_pages"));
    assert!(map.contains("replace_mapping_transactionally"));
    assert!(!map.contains("let _ = self.unmap(address, len)"));

    let protect_start = process_memory
        .find("    fn protect(&mut self")
        .expect("process protection implementation");
    let protect = braced_body(&process_memory[protect_start..]);
    assert!(protect.contains("protect_pages_transactionally"));
    assert!(!protect.contains("core::mem::take(&mut self.mappings)"));

    let unmap_start = process_memory
        .find("    fn unmap(&mut self")
        .expect("process unmap implementation");
    let unmap = braced_body(&process_memory[unmap_start..]);
    assert!(unmap.contains("unmap_pages_transactionally"));
    assert!(!unmap.contains("core::mem::take(&mut self.mappings)"));

    let brk_start = process_memory
        .find("    fn update_brk(&mut self")
        .expect("process brk implementation");
    let brk = braced_body(&process_memory[brk_start..]);
    assert!(brk.contains("zero_user_range"));
    assert!(!brk.contains("vec![0u8; len]"));
    assert!(process_memory.contains("impl Drop for LinuxProcessMemory"));

    let mmap_start = syscall
        .find("pub fn sys_mmap(")
        .expect("mmap implementation");
    let mmap = braced_body(&syscall[mmap_start..]);
    let preload = mmap
        .find("linux_read_mmap_contents")
        .expect("file contents are staged");
    let publish = mmap
        .find("map_current_with_contents")
        .expect("mapping publication accepts staged contents");
    assert!(preload < publish);
}

#[test]
fn linux_common_mmap_paths_avoid_quadratic_metadata_cloning() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read process memory runtime");

    let map_start = process_memory
        .find("    fn map(\n")
        .expect("process mapping implementation");
    let map = braced_body(&process_memory[map_start..]);
    assert!(map.contains("self.mappings"));
    assert!(map.contains(".try_reserve(1)"));
    assert!(map.contains("self.mappings.insert(index, mapping)"));
    assert!(!map.contains("LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?"));

    let find_start = process_memory
        .find("    fn find_free_region(")
        .expect("free-region search");
    let find = braced_body(&process_memory[find_start..]);
    assert!(find.contains("self.mappings.partition_point"));

    let unmap_start = process_memory
        .find("    fn unmap(&mut self")
        .expect("process unmap implementation");
    let unmap = braced_body(&process_memory[unmap_start..]);
    let exact = unmap
        .find("exact_mapping_index")
        .expect("exact mapping fast path");
    let clone = unmap
        .find("LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?")
        .expect("general transactional unmap path");
    assert!(exact < clone);
    assert!(unmap.contains("self.next_addr = core::cmp::min(self.next_addr, address)"));
}

#[test]
fn linux_process_memory_metadata_commit_is_allocation_free() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read process memory runtime");

    for required in [
        "struct LinuxMappingMetadataPlan",
        "fn try_clone_mapping_metadata(",
        "fn commit_mapping_metadata(",
        "fn try_mapped_pages_overlapping(",
        "fn try_mapped_pages_from_backings(",
        "try_reserve_exact",
        "linux_shared_attachment_detached_reference",
    ] {
        assert!(
            memory.contains(required),
            "missing allocation-free metadata primitive: {required}",
        );
    }

    for marker in [
        "    fn map(\n",
        "    fn replace_mapping_transactionally(\n",
        "    fn protect(&mut self",
        "    fn unmap(&mut self",
        "    fn update_brk(&mut self",
        "    fn remap(\n",
        "pub(crate) fn mark_shared(",
    ] {
        let start = memory.find(marker).expect("VM mutation entry point");
        let body = braced_body(&memory[start..]);
        let first_hardware_mutation = [
            "map_unmapped_pages(",
            "map_mapping_pages(",
            "protect_pages_transactionally(",
            "unmap_pages_transactionally(",
        ]
        .into_iter()
        .filter_map(|needle| body.find(needle))
        .min()
        .expect("transactional page-table mutation");
        let planning = [
            "try_clone_mapping_metadata(",
            "try_mapped_pages_overlapping(",
            "try_mapped_pages_from_backings(",
            "try_reserve(",
            "try_reserve_exact",
        ]
        .into_iter()
        .filter_map(|needle| body.find(needle))
        .min()
        .expect("fallible metadata planning");
        assert!(
            planning < first_hardware_mutation,
            "metadata planning must precede page-table mutation in {marker}",
        );

        let committed = &body[first_hardware_mutation..];
        for forbidden in [
            ".to_vec()",
            ".collect::<Vec<_>>()",
            ".clone()",
            ".extend(",
            ".push(",
            "Vec::with_capacity(",
            "debug_assert",
        ] {
            assert!(
                !committed.contains(forbidden),
                "post-mutation forbidden operation {forbidden} remains in {marker}",
            );
        }
    }

    for marker in [
        "pub(crate) fn map_current_with_contents(",
        "pub(crate) fn remap_current(",
    ] {
        let start = memory.find(marker).expect("VM replacement wrapper");
        let body = braced_body(&memory[start..]);
        let mutation = body.find("with_current(").expect("locked VM mutation");
        let release = body
            .find("release_detached_attachment_references(")
            .expect("detached SysV reference release");
        assert!(
            mutation < release,
            "detached references must be released after with_current in {marker}",
        );
    }
}

#[test]
fn linux_process_memory_review_paths_are_checked_and_reversible() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read process memory runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let run_elf = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher");

    assert!(process_memory.contains("pub(crate) fn unregister(pid: usize)"));
    assert!(process_memory.contains(".position(|memory| memory.pid == pid)"));

    for (start_marker, end_marker) in [
        (
            "fn linux_read_user_timespec(",
            "fn linux_write_sleep_remaining(",
        ),
        (
            "fn linux_write_sleep_remaining(",
            "fn linux_sleep_has_deliverable_pending_signal(",
        ),
        (
            "fn linux_signal_wait_deadline(",
            "fn copy_linux_signal_wait_info(",
        ),
        (
            "fn copy_linux_signal_wait_info(",
            "fn finish_linux_signal_wait(",
        ),
        ("pub fn sys_rt_sigaction(", "pub fn sys_rt_sigprocmask("),
    ] {
        let start = syscall
            .find(start_marker)
            .expect("reviewed user-copy function");
        let end = syscall[start..]
            .find(end_marker)
            .expect("reviewed function end")
            + start;
        let body = &syscall[start..end];
        assert!(
            body.contains("linux_copy_from_user(")
                || body.contains("linux_copy_to_user(")
                || body.contains("linux_zero_user("),
            "missing checked copy in {start_marker}"
        );
        assert!(
            !body.contains("core::ptr::"),
            "raw user pointer in {start_marker}"
        );
    }

    let remap_start = process_memory
        .find("    fn remap(\n")
        .expect("process remap implementation");
    let remap = braced_body(&process_memory[remap_start..]);
    assert!(remap.contains("linux_mremap_requires_move("));
    assert!(remap.contains("rollback_remap_destination("));
    assert!(!remap.contains("if old_len == new_len"));

    let map_elf_start = run_elf.find("fn map_elf_image(").expect("ELF mapper");
    let map_elf = braced_body(&run_elf[map_elf_start..]);
    assert!(map_elf.contains("map_elf_page_runs("));
    assert!(!map_elf.contains("elf_mapping_span(image)"));
    let page_runs_start = run_elf
        .find("fn map_elf_page_runs(")
        .expect("ELF page-run mapper");
    let page_runs = braced_body(&run_elf[page_runs_start..]);
    assert!(page_runs.contains("run_elf_page_protection("));
}

#[test]
fn linux_process_memory_copies_enforce_metadata_without_blocking_mremap() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read process memory runtime");
    let process_memory = &memory[memory
        .find("impl LinuxProcessMemory {")
        .expect("process memory implementation")..];

    let copy_to_start = process_memory
        .find("    fn copy_to_user(&self")
        .expect("checked user copyout");
    let copy_to = braced_body(&process_memory[copy_to_start..]);
    assert!(copy_to.contains("self.range_accessible(address, bytes.len(), true)"));
    assert!(copy_to.contains("self.copy_to_mapped_pages(address, bytes)"));

    let copy_from_start = process_memory
        .find("    fn copy_from_user(&self")
        .expect("checked user copyin");
    let copy_from = braced_body(&process_memory[copy_from_start..]);
    assert!(copy_from.contains("self.range_accessible(address, out.len(), false)"));

    let range_start = process_memory
        .find("    fn range_accessible(&self")
        .expect("user metadata and PTE validation");
    let range = braced_body(&process_memory[range_start..]);
    assert!(range.contains("linux_mapping_access_range_covered("));

    let remap_start = process_memory
        .find("    fn remap(\n")
        .expect("process remap implementation");
    let remap = braced_body(&process_memory[remap_start..]);
    assert!(remap.contains("copy_mapping_backings("));
    assert!(!remap.contains("copy_from_user(old_address"));

    let backing_copy_start = process_memory
        .find("    fn copy_mapping_backings(")
        .expect("permission-independent mapping copy");
    let backing_copy = braced_body(&process_memory[backing_copy_start..]);
    assert!(backing_copy.contains("PageFrameAllocator::pfn_address"));
    assert!(backing_copy.contains("core::ptr::copy_nonoverlapping"));

    let zero_start = process_memory
        .find("    fn zero_user_range(&self")
        .expect("transactional brk initialization");
    let zero = braced_body(&process_memory[zero_start..]);
    assert!(zero.contains("copy_to_mapped_pages("));
}

#[test]
fn linux_user_struct_paths_use_process_checked_copies() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    for (start_marker, end_marker) in [
        ("pub fn sys_write(", "pub fn sys_read("),
        ("pub fn sys_read(", "pub fn sys_close("),
        ("pub fn sys_getrandom(", "fn linux_fd_known("),
        ("fn linux_write_cstr(", "fn linux_write_stat_from_attrs("),
        ("fn linux_write_stat_from_attrs(", "fn linux_write_stat("),
        ("fn linux_write_statfs(", "fn linux_write_uts_field("),
        ("pub fn sys_pipe2(", "pub fn sys_dup("),
        ("pub fn sys_readlinkat(", "pub fn sys_stat("),
        ("pub fn sys_getdents64(", "fn linux_lseek_target("),
        ("pub fn sys_pread(", "pub fn sys_pwrite("),
        ("pub fn sys_readv(", "pub fn sys_writev("),
        ("pub fn sys_writev(", "pub fn sys_sendfile("),
        ("pub fn sys_copy_file_range(", "pub fn sys_splice("),
        ("pub fn sys_poll(", "pub fn sys_ppoll("),
        ("pub fn sys_socketpair(", "pub fn sys_bind("),
        ("pub fn sys_accept(", "pub fn sys_getsockname("),
        ("pub fn sys_getsockname(", "pub fn sys_getpeername("),
        ("pub fn sys_getsockopt(", "pub fn sys_sendto("),
        ("pub fn sys_recvfrom(", "pub fn sys_recvmsg("),
        ("pub fn sys_msgsnd(", "pub fn sys_msgrcv("),
        ("pub fn sys_msgrcv(", "pub fn sys_shmget("),
        ("pub fn sys_sigaltstack(", "pub fn sys_tkill("),
        (
            "pub fn sys_sched_getaffinity(",
            "pub fn sys_sched_setaffinity(",
        ),
        ("pub fn sys_uname(", "pub fn sys_time("),
        ("pub fn sys_time(", "pub fn sys_getitimer("),
        ("pub fn sys_getitimer(", "pub fn sys_setitimer("),
        ("pub fn sys_setitimer(", "pub fn sys_timerfd_create("),
        (
            "pub fn sys_linux_timer_create(",
            "pub fn sys_linux_timer_settime(",
        ),
        ("pub fn sys_getresuid(", "pub fn sys_setresgid("),
        ("pub fn sys_get_robust_list(", "pub fn sys_sched_yield("),
        ("pub fn sys_capget(", "pub fn sys_capset("),
        ("pub fn sys_capset(", "pub fn sys_sethostname("),
        ("pub fn sys_getcpu(", "pub fn sys_madvise("),
        ("pub fn sys_mincore(", "pub fn sys_readahead("),
        ("pub fn sys_clock_gettime(", "pub fn sys_clock_getres("),
        ("pub fn sys_clock_getres(", "pub fn sys_gettimeofday("),
        ("pub fn sys_gettimeofday(", "pub fn sys_times("),
        ("pub fn sys_times(", "pub fn sys_getrusage("),
        ("pub fn sys_getrusage(", "pub fn sys_prlimit64("),
        ("pub fn sys_prlimit64(", "pub fn sys_getrlimit("),
        ("pub fn sys_sysinfo(", "pub fn sys_nanosleep_linux("),
    ] {
        let start = syscall
            .find(start_marker)
            .expect("checked Linux pointer path");
        let end = syscall[start..]
            .find(end_marker)
            .expect("checked Linux pointer path end")
            + start;
        let body = &syscall[start..end];
        assert!(
            body.contains("linux_copy_from_user(")
                || body.contains("linux_copy_to_user(")
                || body.contains("linux_zero_user(")
                || body.contains("linux_fill_user(")
                || body.contains("linux_read_user_")
                || body.contains("linux_write_user_"),
            "missing process-owned checked copy in {start_marker}"
        );
        assert!(
            !body.contains("core::ptr::"),
            "raw user pointer in {start_marker}"
        );
    }

    let copy_file_range_start = syscall
        .find("pub fn sys_copy_file_range(")
        .expect("copy_file_range implementation");
    let copy_file_range = braced_body(&syscall[copy_file_range_start..]);
    assert!(copy_file_range.contains("linux_copy_file_read_bytes("));
    assert!(copy_file_range.contains("linux_copy_file_write_bytes("));
    assert!(!copy_file_range.contains("sys_read("));
    assert!(!copy_file_range.contains("sys_write("));

    let getrandom_start = syscall
        .find("pub fn sys_getrandom(")
        .expect("bounded getrandom path");
    let getrandom = braced_body(&syscall[getrandom_start..]);
    assert!(getrandom.contains("LINUX_IO_STAGING_BYTES"));
    assert!(!getrandom.contains("linux_kernel_buffer(len)"));

    let fill_start = syscall
        .find("fn linux_fill_user(")
        .expect("checked Linux fill helper");
    let fill = braced_body(&syscall[fill_start..]);
    assert!(fill.contains("linux_user_buffer_writable("));

    let reset_start = syscall
        .find("pub fn reset_linux_process_state(")
        .expect("Linux process reset");
    let reset = braced_body(&syscall[reset_start..]);
    assert!(reset.contains("linux_process_memory::unregister("));
    assert!(reset.contains("linux_process_memory::reset_launch("));
}

#[test]
fn linux_resource_copyouts_preflight_and_rollback_transactionally() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let pair_copy_start = syscall
        .find("fn linux_write_user_i32_pair(")
        .expect("atomic descriptor-pair copyout helper");
    let pair_copy = braced_body(&syscall[pair_copy_start..]);
    assert_eq!(pair_copy.matches("linux_copy_to_user(").count(), 1);
    assert!(!pair_copy.contains("linux_write_user_i32("));

    for (start_marker, end_marker, first_fd, second_fd) in [
        (
            "pub fn sys_pipe2(",
            "pub fn sys_dup(",
            "read_fd",
            "write_fd",
        ),
        (
            "pub fn sys_socketpair(",
            "pub fn sys_bind(",
            "left_fd",
            "right_fd",
        ),
    ] {
        let start = syscall.find(start_marker).expect("pair-producing syscall");
        let end = syscall[start..]
            .find(end_marker)
            .expect("pair-producing syscall end")
            + start;
        let body = &syscall[start..end];
        let preflight = body
            .find("linux_user_buffer_writable(")
            .expect("complete descriptor-pair preflight");
        let create = body.find("create_pair(").expect("pair allocation");
        let failed_copyout = body
            .find("if let Err(error) = linux_write_user_i32_pair(")
            .expect("fallible atomic descriptor-pair copyout");
        let first_rollback = body
            .find(&format!("sys_close({first_fd})"))
            .expect("first descriptor rollback");
        let second_rollback = body
            .find(&format!("sys_close({second_fd})"))
            .expect("second descriptor rollback");

        assert!(body.contains("core::mem::size_of::<[i32; 2]>()"));
        assert!(preflight < create);
        assert!(create < failed_copyout);
        assert!(failed_copyout < first_rollback);
        assert!(failed_copyout < second_rollback);
        assert!(body[failed_copyout..].contains("return Err(error)"));
        assert!(!body.contains("linux_write_user_i32(fds_ptr"));
    }

    let timer_start = syscall
        .find("pub fn sys_linux_timer_create(")
        .expect("POSIX timer creation");
    let timer_end = syscall[timer_start..]
        .find("pub fn sys_linux_timer_settime(")
        .expect("POSIX timer creation end")
        + timer_start;
    let timer = &syscall[timer_start..timer_end];
    let timer_preflight = timer
        .find("linux_user_buffer_writable(")
        .expect("complete timer-ID preflight");
    let timer_create = timer
        .find("compat::create_object(ObjectType::Timer)")
        .expect("timer allocation");
    let timer_failed_copyout = timer
        .find("if let Err(error) = linux_write_user_i32(")
        .expect("fallible timer-ID copyout");
    let timer_register = timer
        .find("register_linux_timer(pid, handle.0, timer_id, clock, signal)")
        .expect("process timer registration");
    let timer_remove = timer[timer_failed_copyout..]
        .find("remove_linux_timer(pid, timer_id)")
        .map(|offset| timer_failed_copyout + offset)
        .expect("process timer registry removal");
    let timer_close = timer[timer_failed_copyout..]
        .find("sys_handle_close(handle.0)")
        .map(|offset| timer_failed_copyout + offset)
        .expect("timer object rollback");

    assert!(timer.contains("core::mem::size_of::<i32>()"));
    assert!(timer_preflight < timer_create);
    assert!(timer_create < timer_register);
    assert!(timer_register < timer_failed_copyout);
    assert!(timer_failed_copyout < timer_remove);
    assert!(timer_remove < timer_close);
    assert!(timer[timer_failed_copyout..].contains("return Err(error)"));
}

#[test]
fn linux_copy_file_range_uses_explicit_positions_without_moving_descriptor_cursors() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let compat = std::fs::read_to_string(repository.join("src/kernel_objects/compat.rs"))
        .expect("read compatibility object implementation");

    let copy_start = syscall
        .find("pub fn sys_copy_file_range(")
        .expect("copy_file_range implementation");
    let copy = braced_body(&syscall[copy_start..]);
    let input_offset = copy
        .find("linux_read_user_copy_offset(in_offset)?")
        .expect("input offset preflight and read");
    let output_offset = copy
        .find("linux_read_user_copy_offset(out_offset)?")
        .expect("output offset preflight and read");
    let positioned_input = copy
        .find("linux_positioned_fxfs_file(in_fd, input_offset, false)?")
        .expect("positioned input preflight");
    let positioned_output = copy
        .find("linux_positioned_fxfs_file(out_fd, output_offset, true)?")
        .expect("positioned output preflight");
    let first_read = copy
        .find("linux_copy_file_read_bytes(")
        .expect("first file-descriptor read");
    let transactional_copyout = copy
        .find("linux_commit_copy_offsets(")
        .expect("offset copyout before destination mutation");
    let destination_write = copy
        .find("linux_copy_file_write_bytes(")
        .expect("destination file-descriptor write");
    let rollback = copy
        .find("linux_copy_file_rollback_read(")
        .expect("failed offset-copyout input rollback");

    assert!(input_offset < positioned_input);
    assert!(output_offset < positioned_output);
    assert!(positioned_input < first_read);
    assert!(positioned_output < first_read);
    assert!(input_offset < first_read);
    assert!(output_offset < first_read);
    assert!(first_read < transactional_copyout);
    assert!(transactional_copyout < destination_write);
    assert!(transactional_copyout < rollback);
    assert!(!copy[destination_write..].contains("linux_write_user_u64("));

    let positioned_start = syscall
        .find("fn linux_positioned_fxfs_file(")
        .expect("positioned file helper");
    let positioned = braced_body(&syscall[positioned_start..]);
    assert!(positioned.contains(".linux_fxfs_file(record.handle)"));
    assert!(positioned.contains(".cloned()"));
    assert!(positioned.contains("ok_or(SysError::ESPIPE)"));
    assert!(positioned.contains("fxfs::position_cursor(&mut file.cursor"));
    assert!(!positioned.contains("fxfs::seek_cursor(&mut file.cursor"));

    let read_start = syscall
        .find("fn linux_copy_file_read_bytes(")
        .expect("copy range read helper");
    let read = braced_body(&syscall[read_start..]);
    assert!(read.contains("positioned.as_mut()"));
    assert!(read.contains("fxfs::cursor_read(&mut file.cursor"));
    assert!(read.contains("linux_fd_read_bytes(fd, out)"));

    let write_start = syscall
        .find("fn linux_copy_file_write_bytes(")
        .expect("copy range write helper");
    let write = braced_body(&syscall[write_start..]);
    assert!(write.contains("positioned.as_mut()"));
    assert!(write.contains("fxfs::cursor_write(&mut file.cursor"));
    assert!(write.contains("linux_fd_write_bytes(fd, bytes)"));

    let rollback_start = syscall
        .find("fn linux_copy_file_rollback_read(")
        .expect("copy range rollback helper");
    let rollback = braced_body(&syscall[rollback_start..]);
    assert!(rollback.contains("if positioned.is_some()"));
    assert!(rollback.contains("linux_fd_rollback_read_bytes(fd, bytes)"));

    let offset_start = syscall
        .find("fn linux_read_user_copy_offset(")
        .expect("copy offset helper");
    let offset = braced_body(&syscall[offset_start..]);
    assert!(offset.contains("linux_user_buffer_writable("));
    assert!(offset.contains("linux_read_u64_user("));

    let commit_start = syscall
        .find("fn linux_commit_copy_offsets(")
        .expect("transactional copy offset helper");
    let commit = braced_body(&syscall[commit_start..]);
    assert!(commit.contains("linux_write_user_u64("));
    assert!(commit.contains("linux_rollback_copy_offsets("));

    assert!(copy.contains("if written < read"));
    assert!(copy.contains("&buffer[written..read]"));
    assert!(copy.contains("let next_written = total.saturating_add(written)"));
    assert!(copy.contains("linux_rollback_copy_offsets("));

    assert!(compat.contains("pub fn restore_read_bytes("));
    assert!(compat.contains("object.queue.push_front(*byte)"));
}

#[test]
fn fxfs_positioned_io_can_read_past_eof_and_extend_within_the_file_limit() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fxfs = std::fs::read_to_string(repository.join("src/user_level/services/fxfs.rs"))
        .expect("read FxFS implementation");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let copy_start = fxfs
        .find("fn fxfs_copy_from_data(")
        .expect("FxFS positioned read helper");
    let copy = braced_body(&fxfs[copy_start..]);
    assert!(copy.contains("if offset >= data.len()"));
    assert!(copy.contains("return Ok(0)"));

    let position_start = fxfs
        .find("fn position_cursor(&self")
        .expect("FxFS positioned cursor implementation");
    let position = braced_body(&fxfs[position_start..]);
    assert!(position.contains("user_logic::fxfs_file_size_valid(offset)"));
    assert!(position.contains("cursor.offset = offset"));

    assert!(fxfs.contains("pub fn position_cursor(cursor: &mut FxfsCursor, offset: usize)"));

    let pread_start = syscall.find("pub fn sys_pread(").expect("pread path");
    let pread = braced_body(&syscall[pread_start..]);
    assert!(pread.contains("fxfs::position_cursor(&mut file.cursor, offset)"));
    assert!(!pread.contains("fxfs::seek_cursor(&mut file.cursor, offset)"));

    let pwrite_start = syscall.find("pub fn sys_pwrite(").expect("pwrite path");
    let pwrite = braced_body(&syscall[pwrite_start..]);
    assert!(pwrite.contains("fxfs::position_cursor(&mut file.cursor, offset)"));
    assert!(!pwrite.contains("fxfs::seek_cursor(&mut file.cursor, offset)"));
}

#[test]
fn linux_append_mode_positions_fxfs_writes_at_current_end_of_file() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let write_bytes_start = syscall
        .find("pub(crate) fn linux_fd_write_bytes(")
        .expect("Linux fd write helper");
    let write_bytes = braced_body(&syscall[write_bytes_start..]);
    assert!(write_bytes.contains("record.status_flags & LINUX_O_APPEND != 0"));
    assert!(write_bytes.contains("fxfs::cursor_attrs(file.cursor)"));
    assert!(write_bytes.contains("fxfs::position_cursor(&mut file.cursor, append_offset)"));

    let pwrite_start = syscall.find("pub fn sys_pwrite(").expect("pwrite path");
    let pwrite = braced_body(&syscall[pwrite_start..]);
    assert!(pwrite.contains("let append = record.status_flags & LINUX_O_APPEND != 0"));
    assert!(pwrite.contains("fxfs::cursor_attrs(file.cursor)"));
    assert!(pwrite.contains("fxfs::position_cursor(&mut file.cursor, append_offset)"));
}

#[test]
fn linux_fd_syscalls_report_bad_file_descriptor_for_bad_fds() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    for start_marker in [
        "pub fn sys_pread(",
        "pub fn sys_pwrite(",
        "pub fn sys_fcntl(",
        "pub fn sys_lseek(",
    ] {
        let start = syscall.find(start_marker).expect("fd syscall");
        let body = braced_body(&syscall[start..]);
        assert!(
            body.contains("ok_or(SysError::EBADF)") || body.contains("ok_or(SysError::EBADF)?"),
            "{start_marker} should map absent file descriptors to EBADF"
        );
    }

    let write_bytes_start = syscall
        .find("pub(crate) fn linux_fd_write_bytes(")
        .expect("Linux fd write helper");
    let write_bytes = braced_body(&syscall[write_bytes_start..]);
    assert!(write_bytes.contains("ok_or(SysError::EBADF)"));

    let read_bytes_start = syscall
        .find("pub(crate) fn linux_fd_read_bytes(")
        .expect("Linux fd read helper");
    let read_bytes = braced_body(&syscall[read_bytes_start..]);
    assert!(read_bytes.contains("ok_or(SysError::EBADF)"));
}

#[test]
fn linux_vectored_io_preflights_every_entry_before_io() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let decode_start = syscall
        .find("fn linux_read_user_iovecs(")
        .expect("complete iovec decode and preflight helper");
    let decode = braced_body(&syscall[decode_start..]);
    assert!(decode.contains("linux_read_user_iovec("));
    assert!(decode.contains("linux_user_buffer_readable("));
    assert!(decode.contains("linux_user_buffer_writable("));
    assert!(decode.contains("try_reserve_exact(iov_count)"));

    for (start_marker, end_marker, vector_call, io_call) in [
        (
            "pub fn sys_readv(",
            "pub fn sys_writev(",
            "linux_read_user_iovecs(iov_ptr, iov_count, true)",
            "sys_read(",
        ),
        (
            "pub fn sys_writev(",
            "pub fn sys_sendfile(",
            "linux_read_user_iovecs(iov_ptr, iov_count, false)",
            "sys_write(",
        ),
    ] {
        let start = syscall.find(start_marker).expect("vectored I/O path");
        let end = syscall[start..]
            .find(end_marker)
            .expect("vectored I/O path end")
            + start;
        let body = &syscall[start..end];
        let vector = body
            .find(vector_call)
            .expect("complete vector preflight before I/O");
        let io = body.find(io_call).expect("vectored I/O operation");

        assert!(vector < io);
        assert!(!body.contains("linux_read_user_iovec("));
    }
}

#[test]
fn native_zircon_stream_iov_is_bounded_and_independent_of_linux_process_memory() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    assert!(syscall.contains("const ZIRCON_MAX_IOV: usize"));
    assert!(syscall.contains("const ZIRCON_IO_STAGING_BYTES: usize"));
    assert!(syscall.contains("struct ZirconIovec"));

    let snapshot_start = syscall
        .find("fn zircon_read_iovecs(")
        .expect("bounded native Zircon iovec snapshot");
    let snapshot = braced_body(&syscall[snapshot_start..]);
    assert!(snapshot.contains("ZIRCON_MAX_IOV"));
    assert!(snapshot.contains("try_reserve_exact(vector_size)"));
    assert!(snapshot.contains("zircon_read_iovec(vector, index)?"));
    assert!(snapshot.contains("iov.base.checked_add(iov.len)"));

    for (start_marker, end_marker, io_call, copy_call) in [
        (
            "fn zircon_iov_write_compat(",
            "fn zircon_iov_read_compat(",
            "compat::table().write_bytes(",
            "core::ptr::copy_nonoverlapping(",
        ),
        (
            "fn zircon_iov_read_compat(",
            "// Zircon VMO Syscalls",
            "compat::table().read_bytes(",
            "core::ptr::copy_nonoverlapping(",
        ),
    ] {
        let start = syscall
            .find(start_marker)
            .expect("native Zircon iovec path");
        let end = syscall[start..]
            .find(end_marker)
            .expect("native Zircon iovec path end")
            + start;
        let body = &syscall[start..end];

        let snapshot = body
            .find("zircon_read_iovecs(vector, vector_size)?")
            .expect("complete native iovec snapshot before I/O");
        let io = body.find(io_call).expect("native backend I/O");
        assert!(snapshot < io);
        assert!(body.contains("ZIRCON_IO_STAGING_BYTES"));
        assert!(body.contains(io_call));
        assert!(body.contains(copy_call));
        assert!(!body.contains("zircon_read_iovec(vector, index)"));
        assert!(!body.contains("linux_read_user_iovec"));
        assert!(!body.contains("linux_copy_"));
        assert!(!body.contains("linux_kernel_buffer"));
    }

    let native_write_start = syscall
        .find("fn zircon_iov_write_compat(")
        .expect("native Zircon write path");
    let native_write = braced_body(&syscall[native_write_start..]);
    assert!(native_write.contains("Err(_) if total != 0 => return Ok(total)"));

    let writev = braced_body(
        &syscall[syscall
            .find("pub fn sys_stream_writev(")
            .expect("native stream writev")..],
    );
    let readv = braced_body(
        &syscall[syscall
            .find("pub fn sys_stream_readv(")
            .expect("native stream readv")..],
    );
    assert!(writev.contains("zircon_iov_write_compat("));
    assert!(readv.contains("zircon_iov_read_compat("));
}

#[test]
fn linux_core_io_uses_bounded_staging_for_full_validated_ranges() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    for (start_marker, staging_name, preflight, io_call, copy_call) in [
        (
            "pub fn sys_write(",
            "staging",
            "linux_user_buffer_readable(buf_ptr, len)",
            "linux_fd_write_bytes(fd, &staging[..chunk])",
            "linux_copy_from_user(",
        ),
        (
            "pub fn sys_read(",
            "staging",
            "linux_user_buffer_writable(buf_ptr, len)",
            "linux_fd_read_bytes(fd, &mut staging[..chunk])",
            "linux_copy_to_user(",
        ),
        (
            "pub fn sys_pread(",
            "staging",
            "linux_user_buffer_writable(buf, len)",
            "fxfs::cursor_read(&mut file.cursor, &mut staging[..chunk])",
            "linux_copy_to_user(",
        ),
        (
            "pub fn sys_pwrite(",
            "staging",
            "linux_user_buffer_readable(buf, len)",
            "fxfs::cursor_write(&mut file.cursor, &staging[..chunk])",
            "linux_copy_from_user(",
        ),
    ] {
        let start = syscall.find(start_marker).expect("bounded Linux I/O path");
        let body = braced_body(&syscall[start..]);
        let validation = body.find(preflight).expect("whole-range preflight");
        let io = body.find(io_call).expect("bounded descriptor I/O");

        assert!(validation < io);
        assert!(body.contains(&format!(
            "let mut {staging_name} = [0u8; LINUX_IO_STAGING_BYTES]"
        )));
        assert!(body.contains("while total < len"));
        assert!(body.contains("core::cmp::min(staging.len(), len - total)"));
        assert!(body.contains(copy_call));
        assert!(body.contains("if total == 0"));
        assert!(!body.contains("linux_kernel_buffer(len)"));
    }
}

#[test]
fn linux_datagram_io_preserves_message_boundaries_with_bounded_single_operations() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let buffer_start = syscall
        .find("fn linux_datagram_buffer(")
        .expect("bounded datagram allocation helper");
    let buffer = braced_body(&syscall[buffer_start..]);
    assert!(buffer.contains("if len > socket::SOCKET_SIZE"));
    assert!(buffer.contains("try_reserve_exact(len)"));
    assert!(buffer.contains("staging.resize(len, 0)"));
    assert!(!buffer.contains("linux_kernel_buffer("));

    let write_helper_start = syscall
        .find("fn linux_datagram_write(")
        .expect("single-operation datagram write helper");
    let write_helper = braced_body(&syscall[write_helper_start..]);
    assert!(write_helper.contains("linux_datagram_buffer(len)?"));
    assert!(write_helper.contains("linux_copy_from_user("));
    assert_eq!(write_helper.matches("linux_fd_write_bytes(").count(), 1);
    assert!(!write_helper.contains("while "));

    let read_helper_start = syscall
        .find("fn linux_datagram_read(")
        .expect("single-operation datagram read helper");
    let read_helper = braced_body(&syscall[read_helper_start..]);
    assert!(read_helper.contains("info.rx_buf_available"));
    assert!(read_helper.contains("socket::SOCKET_SIZE"));
    assert!(read_helper.contains("linux_datagram_buffer(read_len)?"));
    assert!(read_helper.contains("linux_copy_to_user("));
    assert_eq!(read_helper.matches("linux_fd_read_bytes(").count(), 1);
    assert!(!read_helper.contains("while "));

    for (start_marker, end_marker, preflight, route) in [
        (
            "pub fn sys_write(",
            "pub fn sys_read(",
            "linux_user_buffer_readable(buf_ptr, len)",
            "linux_datagram_write(fd, buf_ptr, len)",
        ),
        (
            "pub fn sys_read(",
            "pub fn sys_close(",
            "linux_user_buffer_writable(buf_ptr, len)",
            "linux_datagram_read(fd, buf_ptr, len, info)",
        ),
    ] {
        let start = syscall.find(start_marker).expect("Linux core I/O path");
        let end = syscall[start..]
            .find(end_marker)
            .expect("Linux core I/O path end")
            + start;
        let body = &syscall[start..end];
        let validation = body.find(preflight).expect("whole-range preflight");
        let datagram = body
            .find("linux_datagram_socket_info(fd)")
            .expect("real datagram detection");
        let operation = body.find(route).expect("datagram-specific I/O route");

        assert!(validation < datagram);
        assert!(datagram < operation);
        assert!(body.contains("LINUX_IO_STAGING_BYTES"));
        assert!(body.contains("while total < len"));
    }

    let sendto_start = syscall.find("pub fn sys_sendto(").expect("sendto path");
    let sendto_end = syscall[sendto_start..]
        .find("fn linux_recvfrom_source_length(")
        .expect("sendto path end")
        + sendto_start;
    assert!(syscall[sendto_start..sendto_end].contains("sys_write(sockfd, buf, len)"));

    let recvfrom_start = syscall.find("pub fn sys_recvfrom(").expect("recvfrom path");
    let recvfrom_end = syscall[recvfrom_start..]
        .find("pub fn sys_recvmsg(")
        .expect("recvfrom path end")
        + recvfrom_start;
    assert!(syscall[recvfrom_start..recvfrom_end].contains("sys_read(sockfd, buf, len)"));

    let writev_helper_start = syscall
        .find("fn linux_datagram_writev(")
        .expect("single-operation vectored datagram write helper");
    let writev_helper = braced_body(&syscall[writev_helper_start..]);
    assert!(writev_helper.contains("linux_datagram_iov_len(iovecs)?"));
    assert!(writev_helper.contains("linux_copy_from_user("));
    assert_eq!(writev_helper.matches("linux_fd_write_bytes(").count(), 1);

    let readv_helper_start = syscall
        .find("fn linux_datagram_readv(")
        .expect("single-operation vectored datagram read helper");
    let readv_helper = braced_body(&syscall[readv_helper_start..]);
    assert!(readv_helper.contains("linux_datagram_iov_len(iovecs)?"));
    assert!(readv_helper.contains("linux_copy_to_user("));
    assert!(readv_helper.contains("read.checked_sub(offset)"));
    assert_eq!(readv_helper.matches("linux_fd_read_bytes(").count(), 1);

    for (start_marker, end_marker, route) in [
        (
            "pub fn sys_readv(",
            "pub fn sys_writev(",
            "linux_datagram_readv(fd, &iovecs, info)",
        ),
        (
            "pub fn sys_writev(",
            "pub fn sys_sendfile(",
            "linux_datagram_writev(fd, &iovecs)",
        ),
    ] {
        let start = syscall.find(start_marker).expect("Linux vectored I/O path");
        let end = syscall[start..]
            .find(end_marker)
            .expect("Linux vectored I/O path end")
            + start;
        let body = &syscall[start..end];
        let datagram = body
            .find("linux_datagram_socket_info(fd)")
            .expect("vectored datagram detection");
        let operation = body
            .find(route)
            .expect("single vectored datagram operation");
        assert!(datagram < operation);
    }
}

#[test]
fn linux_recvfrom_preflights_source_address_before_receive() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");

    let preflight_start = syscall
        .find("fn linux_recvfrom_source_length(")
        .expect("recvfrom source-address preflight helper");
    let preflight = braced_body(&syscall[preflight_start..]);
    assert!(preflight.contains("linux_user_buffer_writable(addrlen"));
    assert!(preflight.contains("linux_read_user_u32(addrlen)"));
    assert!(preflight.contains("linux_user_buffer_writable(src_addr, length)"));
    assert!(preflight.contains("if length != 0 && !linux_user_buffer_writable(src_addr, length)"));

    let recv_start = syscall
        .find("pub fn sys_recvfrom(")
        .expect("recvfrom implementation");
    let recv_end = syscall[recv_start..]
        .find("pub fn sys_recvmsg(")
        .expect("recvfrom implementation end")
        + recv_start;
    let recv = &syscall[recv_start..recv_end];
    let validation = recv
        .find("linux_recvfrom_source_length(src_addr, addrlen)?")
        .expect("source-address validation");
    let read = recv.find("sys_read(").expect("socket receive");

    assert!(validation < read);
    assert!(!recv[read..].contains("linux_read_user_u32(addrlen)"));
}

#[test]
fn linux_resource_clone_inherits_open_descriptions_and_shared_pages() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read process runtime");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read process memory runtime");
    let resource_logic = std::fs::read_to_string(
        repository.join("src/syscall/linux_process_memory_logic_shared.rs"),
    )
    .expect("read resource ownership logic");

    for declaration in [
        "pub(crate) struct LinuxDescriptorEntry",
        "pub fd: usize",
        "pub description_id: u32",
        "pub close_on_exec: bool",
        "pub(crate) struct LinuxOpenDescription",
        "pub object_type: ObjectType",
        "pub status_flags: usize",
        "pub offset: usize",
        "pub references: usize",
    ] {
        assert!(
            syscall.contains(declaration)
                || process.contains(declaration)
                || resource_logic.contains(declaration)
        );
    }
    assert!(syscall.contains("linux_open_descriptions: Vec<LinuxOpenDescription>"));
    assert!(syscall.contains("linux_process_resources: Vec<LinuxProcessResources>"));
    assert!(!syscall.contains("linux_fds: Vec<LinuxFdRecord>"));

    let clone_start = process
        .find("pub(crate) struct LinuxResourceClone")
        .expect("unpublished resource clone reservation");
    let clone_impl = &process[clone_start..];
    assert!(clone_impl.contains("descriptors: Vec<LinuxDescriptorEntry>"));
    assert!(clone_impl.contains("shared_attachments"));
    assert!(clone_impl.contains("impl Drop for LinuxResourceClone"));
    assert!(clone_impl.contains("release_linux_resource_clone"));

    let fork = braced_body(
        &syscall[syscall
            .find("pub fn sys_fork()")
            .expect("synthetic fork remains for Task 9")..],
    );
    assert!(!fork.contains("reserve_resource_clone"));
    assert!(!fork.contains("LinuxResourceClone"));

    let close = braced_body(
        &syscall[syscall
            .find("pub fn sys_close(fd: usize)")
            .expect("descriptor close")..],
    );
    assert!(close.contains("close_linux_fd_for_current_process(fd)"));
    let close_helper = braced_body(
        &syscall[syscall
            .find("fn close_linux_fd_for_current_process(fd: usize)")
            .expect("central descriptor close helper")..],
    );
    assert!(close_helper.contains("release_open_description"));
    assert!(close_helper.contains("final_reference"));

    let shared_record = braced_body(
        &syscall[syscall
            .find("struct LinuxSharedMemoryRecord")
            .expect("shared-memory object record")..],
    );
    assert!(shared_record.contains("named: bool"));
    assert!(shared_record.contains("references: usize"));
    assert!(!shared_record.contains("attachments:"));
    assert!(resource_logic.contains("struct LinuxSharedPageRecord"));
    assert!(resource_logic.contains("references: usize"));
    assert!(memory.contains("reserve_shared_attachments"));
    assert!(memory.contains("LinuxMappingSource::SharedMemory"));
    assert!(memory.contains("shared_attachments: Vec<LinuxSharedAttachmentRecord>"));
    assert!(memory.contains("pub attachment_len: usize"));
    assert!(memory.contains("attachment_len: attachment.len"));

    let mark_shared_start = memory
        .find("pub(crate) fn mark_shared(")
        .expect("shared mapping transaction");
    let mark_shared = braced_body(&memory[mark_shared_start..]);
    assert_eq!(mark_shared.matches("with_current(").count(), 1);
    assert!(mark_shared.contains("acquire_or_register_shared_page"));

    let reserve_start = syscall
        .find("fn reserve_process_resources(")
        .expect("process resource reservation");
    let reserve = braced_body(&syscall[reserve_start..]);
    assert!(reserve.contains("clone_linux_process_state"));
    assert!(reserve.contains("try_reserve_exact"));

    let release_start = syscall
        .find("pub(crate) fn release_linux_resource_clone(")
        .expect("resource clone rollback");
    let release = braced_body(&syscall[release_start..]);
    assert!(release.contains("release_reserved_fork_resources"));
    assert!(!release.contains("sys_handle_close"));

    let reserved_release_start = syscall
        .find("fn release_reserved_fork_resources(")
        .expect("allocation-free reserved resource rollback");
    let reserved_release = braced_body(&syscall[reserved_release_start..]);
    assert!(reserved_release.contains("assert!(parent_ownership_preserved)"));
    assert!(!reserved_release.contains("debug_assert!"));

    let process_release_start = syscall
        .find("pub(crate) fn release_linux_process_resources(")
        .expect("process resource teardown");
    let process_release = braced_body(&syscall[process_release_start..]);
    assert!(process_release.contains("sys_handle_close"));

    let munmap = braced_body(
        &syscall[syscall
            .find("pub fn sys_munmap(")
            .expect("process-local unmap")..],
    );
    assert!(munmap.contains("release_shared_memory_attachment"));

    let rmid = braced_body(
        &syscall[syscall
            .find("pub fn sys_shmctl(")
            .expect("shared-memory removal")..],
    );
    assert!(rmid.contains("remove_shared_memory_name"));
    assert!(!rmid.contains("compat::close_handle(HandleValue(record.handle))"));
}

#[test]
fn linux_fork_publishes_only_a_complete_child() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read process runtime");
    let fork_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_fork_logic_shared.rs"))
            .expect("read shared fork transaction logic");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read process memory runtime");
    let host = std::fs::read_to_string(repository.join("tests/host/src/lib.rs"))
        .expect("read host fork adapter");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let thread = std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/thread.rs"))
        .expect("read AArch64 thread runtime");
    let address_space =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/user_address_space.rs"))
            .expect("read AArch64 address-space owner");
    let address_space_core =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/aarch64_vm_logic_shared.rs"))
            .expect("read AArch64 address-space core");
    let context_switch =
        std::fs::read_to_string(repository.join("src/kernel_lowlevel/ARM64/context_switch.S"))
            .expect("read AArch64 child restore assembly");

    for declaration in [
        "pub(crate) struct Aarch64ProcessStart",
        "pub frame: Aarch64ExceptionFrame",
        "pub return_pc: u64",
        "pub pstate: u64",
        "pub root_paddr: u64",
        "struct Aarch64LinuxForkOps",
        "type LinuxForkReservation = LinuxForkOwnershipCore<Aarch64LinuxForkOps>",
        "impl LinuxForkOwnershipOps for Aarch64LinuxForkOps",
    ] {
        assert!(process.contains(declaration), "missing `{declaration}`");
    }
    assert!(process.contains("include!(\"linux_fork_logic_shared.rs\")"));
    assert!(fork_logic.contains("pub(crate) trait LinuxForkTransactionBackend"));
    assert!(fork_logic.contains("pub(crate) trait LinuxForkOwnershipOps"));
    assert!(fork_logic.contains("pub(crate) struct LinuxForkOwnershipCore"));
    assert!(fork_logic.contains(
        "impl<O: LinuxForkOwnershipOps> LinuxForkTransactionBackend for LinuxForkOwnershipCore<O>"
    ));
    assert!(fork_logic.contains("pub(crate) fn run_linux_fork_transaction"));
    assert!(!process.contains("impl LinuxForkTransactionBackend for LinuxForkReservation"));

    let fork = braced_body(
        &syscall[syscall
            .find("pub fn sys_fork()")
            .expect("fork implementation")..],
    );
    assert!(fork.contains("sys_fork_with_namespace_flags(0, LINUX_SIGCHLD)"));
    assert!(!fork.contains("LINUX_NEXT_SYNTHETIC_PID"));
    assert!(!syscall.contains("static LINUX_NEXT_SYNTHETIC_PID"));
    let fork_path = braced_body(
        &syscall[syscall
            .find("fn sys_fork_with_child_tid(")
            .expect("shared eager fork path")..],
    );
    assert!(fork_path.contains("linux_syscall_context::current()"));
    assert!(fork_path.contains("linux_process::run_fork_transaction("));
    let clone = braced_body(
        &syscall[syscall
            .find("pub fn sys_clone(")
            .expect("clone implementation")..],
    );
    assert!(clone.contains("linux_signal_valid(flags & 0xff, LINUX_MAX_SIGNAL)"));
    assert!(clone.contains("flags & 0xff"));
    assert!(fork_path.contains("child_exit_signal"));

    let transaction = &fork_logic[fork_logic
        .find("pub(crate) fn run_linux_fork_transaction")
        .expect("shared fork transaction")..];
    for operation in [
        "LinuxForkAcquisition::SchedulerThread",
        "LinuxForkAcquisition::Task",
        "LinuxForkAcquisition::Process",
        "LinuxForkAcquisition::Resources",
        "LinuxForkAcquisition::Memory",
        "LinuxForkAcquisition::Configured",
        "transaction.backend.install_resources()?",
        "transaction.backend.publish_process()?",
        "transaction.backend.publish_task()?",
        "transaction.backend.publish_scheduler_thread()?",
        "transaction.backend.complete_publication()?",
    ] {
        assert!(
            transaction.contains(operation),
            "missing transaction `{operation}`"
        );
    }
    for point in [
        "LinuxForkFailurePoint::SchedulerThread",
        "LinuxForkFailurePoint::Task",
        "LinuxForkFailurePoint::Process",
        "LinuxForkFailurePoint::Memory",
        "LinuxForkFailurePoint::Configured",
    ] {
        assert!(
            transaction.contains(point),
            "missing transaction failpoint `{point}`"
        );
    }

    let transaction_drop = &fork_logic[fork_logic
        .find("impl<B: LinuxForkTransactionBackend> Drop for LinuxForkTransaction<B>")
        .expect("shared fork rollback owner")..];
    assert!(transaction_drop.contains("ledger.rollback_into"));
    assert!(transaction_drop.contains("self.backend.rollback(acquisition)"));

    let ownership_core = &fork_logic[fork_logic
        .find(
            "impl<O: LinuxForkOwnershipOps> LinuxForkTransactionBackend for LinuxForkOwnershipCore<O>",
        )
        .expect("shared production ownership core")..];
    for rollback in [
        "self.ops.rollback_configured(configured)",
        "self.ops.rollback_memory(memory)",
        "self.ops.rollback_reserved_resources(resources)",
        "self.ops.rollback_installed_resources(process)",
        "self.ops.rollback_process(process)",
        "self.ops.rollback_task(task)",
        "self.ops.rollback_scheduler_thread(scheduler_thread)",
        "self.ops.restore_publication(publication)",
    ] {
        assert!(
            ownership_core.contains(rollback),
            "missing shared ownership rollback `{rollback}`"
        );
    }

    let backend = &process[process
        .find("impl LinuxForkOwnershipOps for Aarch64LinuxForkOps")
        .expect("production fork ownership operations")..];
    for acquisition in [
        "create_suspended_thread_on_cpu",
        "reserve_fork_task",
        "reserve_child_with_pid",
        "reserve_resource_clone",
        "clone_for_fork",
        "frame.regs[0] = 0",
        "read_user_stack_pointer()",
        "read_user_tls()",
    ] {
        assert!(
            backend.contains(acquisition),
            "missing acquisition `{acquisition}`"
        );
    }
    for release in [
        "unregister(memory.pid)",
        "drop(resources)",
        "release_resources(process.pid)",
        "runtime.processes.rollback_fork(process)",
        "linux_task::rollback_fork_task(task)",
        "terminate_thread(scheduler_thread)",
    ] {
        assert!(backend.contains(release), "missing rollback `{release}`");
    }

    let publication_mask = backend
        .find("Ok(crate::kernel_lowlevel::cpu::mask_interrupts())")
        .expect("interrupt-masked fork publication");
    let publication_restore = backend
        .find("restore_interrupts(publication)")
        .expect("fork publication interrupt restore");
    assert!(publication_mask < publication_restore);

    assert!(host.contains("LinuxForkOwnershipCore<HostForkOps<'a>>"));
    assert!(host.contains("impl LinuxForkOwnershipOps for HostForkOps<'_>"));
    assert!(!host.contains("struct HostForkBackend"));
    assert!(!host.contains("fn rollback(&mut self, acquisition: LinuxForkAcquisition)"));

    assert!(memory.contains("pub(crate) fn clone_for_fork("));
    assert!(memory.contains("PageFrameAllocator::alloc()"));
    assert!(memory.contains("core::ptr::copy_nonoverlapping"));
    assert!(memory.contains("acquire_shared_page("));
    assert!(memory.contains("allocate_shared_mmap_pages("));
    assert!(memory.contains("flags & LINUX_MAP_SHARED != 0"));
    assert!(memory.contains("linux_mmap_backing_is_shared(self.mappings[index].flags)"));
    assert!(memory.contains("crate::kernel_lowlevel::cpu::sync_instruction_cache()"));
    assert!(memory.contains("try_reserve_exact(parent.mappings.len())"));
    assert!(memory.contains("try_reserve(1)"));
    assert!(memory.contains("Aarch64AddressSpace::new_for_fork(fork_table_allocation_failure)"));
    assert!(memory.contains("LinuxForkFailurePoint::ChildRoot"));
    assert!(memory.contains("LinuxForkFailurePoint::TablePage"));
    assert!(memory.contains("LinuxForkFailurePoint::SharedReference"));
    assert!(fork_logic.contains("pub(crate) trait LinuxForkPageOps"));
    assert!(fork_logic.contains("pub(crate) fn clone_linux_fork_pages"));
    assert!(fork_logic.contains("pub(crate) fn map_linux_fork_pages"));
    assert!(fork_logic.contains("pub(crate) fn clone_and_map_linux_fork_pages"));
    assert!(
        memory.contains("impl super::linux_process::LinuxForkPageOps for LinuxProcessForkPageOps")
    );
    assert!(memory.contains("super::linux_process::clone_linux_fork_pages("));
    assert!(memory.contains("super::linux_process::map_linux_fork_pages("));
    assert!(host.contains("impl LinuxForkPageOps for HostForkPageOps<'_>"));
    assert!(host.contains("clone_and_map_linux_fork_pages("));
    for point in [
        "LinuxForkFailurePoint::PrivatePageAllocation",
        "LinuxForkFailurePoint::PrivatePageCopy",
        "LinuxForkFailurePoint::PrivatePageMap",
        "LinuxForkFailurePoint::SharedPageMap",
    ] {
        assert!(
            fork_logic.contains(point),
            "missing granular page failpoint `{point}`"
        );
    }
    assert!(syscall.contains("LinuxForkFailurePoint::DescriptorReference"));
    assert!(syscall.contains("self.linux_process_resources.try_reserve(1)"));
    assert!(syscall
        .contains("self.linux_process_resources.len() == self.linux_process_resources.capacity()"));
    assert!(syscall.contains("rollback_fork_resource_clone"));
    assert!(!syscall.contains("debug_assert!(self.rollback_fork_resource_clone"));
    assert!(address_space.contains("failure_hook: Option<TableAllocationFailureHook>"));
    assert!(address_space.contains("self.failure_hook.is_some_and(|hook| hook(allocation))"));
    assert!(address_space_core.contains(".try_reserve(1)"));
    let clone_pages = &fork_logic[fork_logic
        .find("pub(crate) fn clone_linux_fork_pages")
        .expect("shared fork page clone")..];
    assert!(clone_pages.contains("ops.allocate_private(parent)"));
    assert!(clone_pages.contains("ops.release_page(child)"));
    assert!(clone_pages.contains("release_linux_fork_pages(ops, &child_pages)"));
    assert!(task.contains("pub(crate) fn reserve_fork_task("));
    assert!(task.contains("task.tgid = reservation.tid"));
    assert!(task.contains("pub(crate) fn publish_fork_task("));
    assert!(task.contains("pub(crate) fn rollback_fork_task("));
    for point in [
        "LinuxForkFailurePoint::ProcessPublication",
        "LinuxForkFailurePoint::TaskPublication",
        "LinuxForkFailurePoint::SchedulerPublication",
    ] {
        assert!(
            transaction.contains(point),
            "missing publication failpoint `{point}`"
        );
    }

    for process_owned_state in [
        "container: LinuxProcessContainerState",
        "fn clone_linux_process_state(",
    ] {
        assert!(
            syscall.contains(process_owned_state),
            "missing process-owned fork state `{process_owned_state}`"
        );
    }
    for signal_state in [
        "type LinuxProcessSignalState = LinuxProcessSignalStateCore<",
        "fork_child(LinuxPendingSignals::new())",
        "pub(crate) fn clone_signal_state_for_fork(",
    ] {
        assert!(
            process.contains(signal_state),
            "missing process-runtime signal state `{signal_state}`"
        );
    }
    for removed_global in [
        "static LINUX_SIGNAL_HANDLERS",
        "static LINUX_SIGNAL_FLAGS",
        "static LINUX_SIGNAL_RESTORERS",
        "static LINUX_SIGNAL_ACTION_MASKS",
        "static mut LINUX_PROCESS_PENDING",
        "linux_mounts: Vec<LinuxMountRecord>",
        "linux_cap_effective: u64",
        "linux_seccomp_mode: usize",
        "linux_chrooted: bool",
    ] {
        assert!(
            !syscall.contains(removed_global),
            "fork-shared process state remains global `{removed_global}`"
        );
    }

    for timer_ownership in [
        "timer_handles: Vec<u32>",
        "real_timer_deadline_tick: u64",
        "fn linux_timer_owned(&self, pid: usize, timer_id: u32) -> bool",
        "fn linux_real_timer_deadline(&self, pid: usize) -> u64",
        "fn set_linux_real_timer_deadline(&mut self, pid: usize, deadline: u64)",
    ] {
        assert!(
            syscall.contains(timer_ownership),
            "missing process timer ownership `{timer_ownership}`"
        );
    }
    assert!(!syscall.contains("static LINUX_REAL_TIMER_DEADLINE_TICK"));
    let installed_resources = &syscall[syscall
        .find("fn install_process_resources(")
        .expect("child resource installation")..];
    assert!(installed_resources.contains("timer_handles: Vec::new()"));
    assert!(installed_resources.contains("real_timer_deadline_tick: LINUX_TIMER_DISABLED"));
    let process_reset = braced_body(
        &syscall[syscall
            .find("fn reset_linux_process_state(&mut self) -> Vec<u32>")
            .expect("process resource reset")..],
    );
    assert!(process_reset.contains("resources.timer_handles"));
    let timer_create = braced_body(
        &syscall[syscall
            .find("pub fn sys_linux_timer_create(")
            .expect("POSIX timer create")..],
    );
    assert!(timer_create.contains("register_linux_timer(pid, handle.0, timer_id, clock, signal)"));
    let timer_registration = &syscall[syscall
        .find("fn register_linux_timer(")
        .expect("process timer registration")..];
    assert!(timer_registration.contains("process_resources_mut(pid)"));
    for operation in [
        "pub fn sys_linux_timer_settime(",
        "pub fn sys_linux_timer_gettime(",
        "pub fn sys_linux_timer_getoverrun(",
        "pub fn sys_linux_timer_delete(",
    ] {
        let body = braced_body(&syscall[syscall.find(operation).expect("POSIX timer operation")..]);
        assert!(
            body.contains("linux_timer_owned("),
            "unscoped `{operation}`"
        );
    }

    assert!(thread.contains("start_linux_process_child"));
    assert!(context_switch.contains("start_linux_process_child"));
    assert!(context_switch.contains("msr     ttbr0_el1"));
    assert!(context_switch.contains("ldp     q30, q31"));
    assert!(context_switch.contains("eret"));

    let process_child = &context_switch[context_switch
        .find("start_linux_process_child:")
        .expect("process child assembly entry")..];
    let ttbr = process_child
        .find("msr     ttbr0_el1")
        .expect("child TTBR0");
    let first_dsb = process_child.find("dsb     ish").expect("pre-TLBI barrier");
    let tlbi = process_child
        .find("tlbi    vmalle1is")
        .expect("child TLB flush");
    let second_dsb = tlbi
        + process_child[tlbi..]
            .find("dsb     ish")
            .expect("post-TLBI barrier");
    let isb = process_child
        .find("isb")
        .expect("child instruction barrier");
    assert!(ttbr < first_dsb && first_dsb < tlbi && tlbi < second_dsb && second_dsb < isb);
}

#[test]
fn linux_wait_reaps_one_real_child_status() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let process_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_logic_shared.rs"))
            .expect("read Linux process logic");
    let process_memory =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
            .expect("read Linux process memory runtime");
    let process_memory_logic = std::fs::read_to_string(
        repository.join("src/syscall/linux_process_memory_logic_shared.rs"),
    )
    .expect("read shared Linux process memory logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");

    for required in [
        "pub(crate) enum LinuxWaitOutcome",
        "Ready { pid: usize, status: i32 }",
        "WouldBlock",
        "NoChildren",
        "pub(crate) fn wait_current(",
        "pub(crate) fn complete_wait_current(",
        "pub(crate) fn exit_current_process(",
        "pub(crate) const LINUX_WAIT_WNOHANG: u32 = 1;",
        "pub(crate) const LINUX_WAIT_WUNTRACED: u32 = 2;",
        "pub(crate) const fn linux_wait_options_valid(options: u32) -> bool",
        "options & !(LINUX_WAIT_WNOHANG | LINUX_WAIT_WUNTRACED) == 0",
    ] {
        assert!(
            process.contains(required) || process_logic.contains(required),
            "missing wait/process lifecycle API `{required}`"
        );
    }

    let wait_start = syscall.find("pub fn sys_wait4(").expect("wait4 syscall");
    let wait = braced_body(&syscall[wait_start..]);
    let wait_runtime_start = process
        .find("pub(crate) fn wait_current(")
        .expect("wait runtime helper");
    let wait_runtime = braced_body(&process[wait_runtime_start..]);
    assert!(wait.contains("linux_wait_selector(pid"));
    assert!(wait.contains("!linux_process::linux_wait_options_valid(options)"));
    assert!(wait.contains("options & linux_process::LINUX_WAIT_WNOHANG"));
    assert!(wait.contains("LinuxWaitOutcome::Ready"));
    assert!(wait.contains("complete_wait_current"));
    assert!(wait.contains("LinuxWaitOutcome::WouldBlock"));
    assert!(wait_runtime.contains("LinuxBlockReason::ChildWait"));
    assert!(wait_runtime.contains("scheduler::schedule()"));
    assert!(wait.contains("SysError::ECHILD"));

    let completion_start = process
        .find("pub(crate) fn complete_wait_current(")
        .expect("process-locked wait completion helper");
    let completion = braced_body(&process[completion_start..]);
    assert!(completion.contains("with_runtime(|runtime|"));
    assert!(completion.contains("complete_linux_wait("));
    assert!(completion.contains("linux_process_memory::copy_to_process("));
    assert!(completion.contains("parent.pid"));
    let transaction_start = process_logic
        .find("pub(crate) fn complete_linux_wait")
        .expect("shared wait transaction");
    let transaction = braced_body(&process_logic[transaction_start..]);
    let revalidate = transaction
        .find("wait_outcome(parent_pid, selector)")
        .expect("ready child revalidation");
    let copy = transaction
        .find("copy_status(status)")
        .expect("status copyout");
    let commit = transaction
        .find("reap(parent_pid, pid)")
        .expect("one-time reap");
    assert!(revalidate < copy && copy < commit);

    let exit_start = syscall
        .find("pub fn sys_exit(exit_code: i32)")
        .expect("exit");
    let exit = braced_body(&syscall[exit_start..]);
    assert!(exit.contains("exit_current_linux_process(exit_code, false)"));
    let exit_runtime_start = syscall
        .find("fn exit_current_linux_process(")
        .expect("shared Linux exit helper");
    let exit_runtime = braced_body(&syscall[exit_runtime_start..]);
    assert!(exit_runtime.contains("exit_current_process"));
    assert!(task.contains("retire_process_tasks"));
    assert!(process.contains("linux_task::wake_process_waiters(parent_pid)"));

    assert!(process_logic.contains("pub notification_signal: Option<usize>"));
    assert!(process_logic.contains("pub(crate) const fn linux_child_exit_notification("));
    assert!(process_logic.contains("pub(crate) const fn linux_visible_parent_pid("));
    let current_parent = braced_body(
        &process[process
            .find("pub(crate) fn current_parent_pid()")
            .expect("visible parent identity helper")..],
    );
    assert!(current_parent.contains("linux_visible_parent_pid(process.pid, process.parent_pid)"));
    assert!(process.contains("LinuxPendingSignal::standard(notification_signal)"));

    assert!(process_memory_logic.contains("pub(crate) enum LinuxCopyAddressErrorClass"));
    assert!(process_memory_logic.contains("pub(crate) const fn linux_copy_address_error_class("));
    assert_eq!(
        process_memory
            .matches(".map_err(map_copy_address_error)")
            .count(),
        2,
        "only checked AArch64 copy directions use the EFAULT mapper",
    );
    assert!(process_memory.contains("fn map_address_error("));
    assert!(process_memory.contains("fn map_copy_address_error("));

    let group_start = syscall
        .find("pub fn sys_exit_group(exit_code: i32)")
        .expect("exit_group");
    let group = braced_body(&syscall[group_start..]);
    assert!(!group.contains("linux_task::reset()"));
    assert!(group.contains("exit_current_linux_process(exit_code, true)"));
}

#[test]
fn posix_resources_include_linux_process_page_lifecycle() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read Linux process memory");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall runtime");
    let guest = std::fs::read_to_string(repository.join("src/user_level/services/posix_test.rs"))
        .expect("read POSIX guest runner");

    for field in [
        "linux_processes",
        "linux_zombies",
        "private_pages",
        "shared_pages",
        "page_table_pages",
    ] {
        assert!(syscall.contains(&format!("pub {field}: usize")));
        assert!(guest.contains(&format!("\"{field}\"")));
    }
    assert!(process.contains("pub(crate) struct LinuxProcessResourceCounts"));
    assert!(process.contains("pub(crate) fn resource_counts("));
    assert!(process.contains("runtime.processes.running_pids_match("));
    assert!(memory.contains("pub(crate) fn resource_counts("));
    assert!(syscall.contains("let linux_process_counts = linux_process::resource_counts()"));
    assert!(!syscall.contains("let linux_memory_counts = linux_process_memory::resource_counts()"));
    for field in ["private_pages", "shared_pages", "page_table_pages"] {
        assert!(process.contains(&format!("{field}: linux_memory_counts.{field}")));
    }
    assert!(syscall.contains("processes: crate::kernel_lowlevel::memory::process_manager()"));
}

#[test]
fn linux_wait_and_exit_runtime_own_blocking_reaping_and_safe_teardown() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let process_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_logic_shared.rs"))
            .expect("read Linux process logic");
    let memory = std::fs::read_to_string(repository.join("src/syscall/linux_process_memory.rs"))
        .expect("read Linux process memory");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall runtime");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");

    let wait_start = process
        .find("pub(crate) fn wait_current(")
        .expect("wait runtime helper");
    let wait = braced_body(&process[wait_start..]);
    assert!(wait.contains("nohang"));
    assert!(!wait.contains("_nohang"));
    assert!(wait.contains("linux_task::block_current(LinuxBlockReason::ChildWait)"));
    assert!(wait.contains("scheduler::schedule()"));

    let syscall_wait_start = syscall.find("pub fn sys_wait4(").expect("wait4 syscall");
    let syscall_wait = braced_body(&syscall[syscall_wait_start..]);
    assert!(!syscall_wait.contains("block_current"));
    assert!(!syscall_wait.contains("scheduler::schedule()"));

    assert!(process_logic.contains("pub(crate) const LINUX_LAUNCH_REAPER_PID"));
    assert!(process_logic.contains("fn reparent_children_to_launch_reaper("));
    assert!(process_logic.contains("fn adopt_launch_descendants("));
    assert!(process_logic.contains("fn reap_launch_descendants("));

    let exit_start = process
        .find("pub(crate) fn exit_current_process(")
        .expect("process exit helper");
    let exit = braced_body(&process[exit_start..]);
    let deactivate = exit
        .find("deactivate_current_address_space()")
        .expect("active address space is deactivated");
    let retire_tasks = exit
        .find("retire_process_tasks(")
        .expect("current process tasks are retired");
    let unregister = exit
        .find("linux_process_memory::unregister(process.pid)")
        .expect("exiting memory is unregistered");
    assert!(deactivate < retire_tasks && retire_tasks < unregister);
    assert!(memory.contains("mmu::activate_bootstrap_on_current_cpu()"));

    assert!(task.contains("fn retire_tasks("));
    let retire_process = braced_body(
        &task[task
            .find("pub(crate) fn retire_process_tasks(")
            .expect("process task retirement")..],
    );
    let retire_descendants = braced_body(
        &task[task
            .find("pub(crate) fn retire_launch_descendants(")
            .expect("launch descendant retirement")..],
    );
    assert!(retire_process.contains("retire_tasks("));
    assert!(retire_descendants.contains("retire_tasks("));
    assert!(task.contains("copy_to_process("));
    assert!(task.contains("wake_address("));
}

#[test]
fn linux_fork_inherits_pid_owned_identity_paths_and_clears_transient_state() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read task runtime");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");

    for inherited in [
        "credentials: LinuxCredentialsCore",
        "cwd: String",
        "root: String",
        "credentials: parent.credentials.fork_child()",
        "try_clone_linux_fork_path(&parent.cwd)",
        "try_clone_linux_fork_path(&parent.root)",
        "container: parent.container.try_fork(namespace_flags)?",
    ] {
        assert!(
            syscall.contains(inherited),
            "missing fork inheritance `{inherited}`"
        );
    }

    let install = braced_body(
        &syscall[syscall
            .find("fn install_process_resources(")
            .expect("child process resource install")..],
    );
    for empty_child_state in [
        "timer_handles: Vec::new()",
        "real_timer_deadline_tick: LINUX_TIMER_DISABLED",
    ] {
        assert!(
            install.contains(empty_child_state),
            "child inherited transient state `{empty_child_state}`"
        );
    }
    let clone_signal = braced_body(
        &process[process
            .find("pub(crate) fn clone_signal_state_for_fork(")
            .expect("fork signal-state clone")..],
    );
    assert!(clone_signal.contains("fork_child(LinuxPendingSignals::new())"));
    assert!(syscall.contains("fn linux_aio_request_count() -> usize"));
    assert!(syscall.contains("active requests are invariantly zero"));

    let rollback = braced_body(
        &syscall[syscall
            .find("fn rollback_fork_process_resources(")
            .expect("fork process-resource rollback")..],
    );
    assert!(rollback.contains("let transient_state_empty ="));
    assert!(rollback.contains("let parent_ownership_preserved ="));
    assert!(rollback.contains("self.rollback_fork_resource_clone("));
    assert!(rollback.contains("transient_state_empty && parent_ownership_preserved"));

    let chroot = braced_body(
        &syscall[syscall
            .find("pub fn sys_chroot(")
            .expect("process root update")..],
    );
    assert!(chroot.contains("resources.root = root"));
    assert!(!chroot.contains("resources.cwd ="));

    let reserve_task = braced_body(
        &task[task
            .find("pub(crate) fn reserve_fork_task(")
            .expect("fork child task reservation")..],
    );
    assert!(reserve_task.contains("reserve_child(0, scheduler_id.0)"));
    assert!(reserve_task.contains("prepare_linux_fork_task_signal_state("));
    assert!(reserve_task.contains("signal_state.reset_in_place()"));
    assert!(reserve_task.contains("signal_state.mask = mask"));
}

#[test]
fn linux_signal_termination_reports_wait_status_and_sigchld() {
    smros_host_tests::linux_signal_lifecycle_behavior_contract();

    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let process = std::fs::read_to_string(repository.join("src/syscall/linux_process.rs"))
        .expect("read Linux process runtime");
    let process_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_process_logic_shared.rs"))
            .expect("read shared Linux process logic");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read syscall implementation");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let launcher = std::fs::read_to_string(repository.join("src/user_level/services/run_elf.rs"))
        .expect("read ELF launcher runtime");

    assert!(process_logic.contains("pub(crate) enum LinuxSigchldExitPolicy"));
    assert!(process_logic.contains("RetainZombieAndNotify"));
    assert!(process_logic.contains("ReapAndNotify"));
    assert!(process_logic.contains("ReapWithoutNotify"));
    assert!(process_logic.contains("pub(crate) fn terminate_child("));
    assert!(process_logic.contains("pub(crate) fn linux_wait_status_signal("));
    assert!(process_logic.contains("pub(crate) struct LinuxProcessSignalStateCore"));
    assert!(process_logic.contains("pub signal_actions: [Action; N]"));
    assert!(process_logic.contains("pub process_pending: Pending"));

    for process_owned in [
        "type LinuxProcessSignalState = LinuxProcessSignalStateCore<",
        "pub(crate) fn with_signal_state<R>(",
        "pub(crate) fn clone_signal_state_for_fork(",
    ] {
        assert!(
            process.contains(process_owned),
            "missing Linux-process signal ownership `{process_owned}`"
        );
    }
    assert!(!syscall.contains("struct LinuxKernelSigaction"));
    let resources = braced_body(
        &syscall[syscall
            .find("struct LinuxProcessResources")
            .expect("Linux process resources")..],
    );
    assert!(!resources.contains("signal_actions"));
    assert!(!resources.contains("process_pending"));

    let store_action = braced_body(
        &syscall[syscall
            .find("fn store_linux_signal_action(")
            .expect("signal action storage")..],
    );
    assert!(store_action.contains("let tgid = linux_resource_pid()"));
    assert!(store_action.contains("linux_task::discard_signal(tgid, signum)"));
    let discard_task = braced_body(
        &task[task
            .find("pub(crate) fn discard_signal(tgid: usize, signum: usize)")
            .expect("TGID-scoped task pending discard")..],
    );
    assert!(discard_task.contains("task.tgid == tgid"));

    let target = braced_body(
        &task[task
            .find("pub(crate) fn process_signal_target(")
            .expect("process-directed target selection")..],
    );
    assert!(target.contains("tgid"));
    assert!(target.contains("process_signal_target_for(tgid, signum)"));
    assert!(task.contains("select_linux_process_signal_target("));

    let terminate = braced_body(
        &process[process
            .find("pub(crate) fn terminate_by_signal(")
            .expect("signal termination runtime")..],
    );
    assert!(terminate.contains("linux_wait_status_signal(signum, false)"));
    assert!(terminate.contains("linux_task::terminate_process_tasks(tgid)"));
    assert!(terminate.contains("finish_terminal_process("));
    assert!(!terminate.contains("by_pid(tgid)"));
    assert!(!terminate.contains("linux_task::finish_current_without_el0_return()"));

    let finish_signal = braced_body(
        &syscall[syscall
            .find("fn terminate_linux_process_by_signal(")
            .expect("signal termination completion")..],
    );
    assert!(finish_signal.contains("linux_process::terminate_by_signal(tgid, signum)"));
    assert!(finish_signal.contains("LinuxProcessExitOutcome::LaunchRoot"));
    assert!(finish_signal.contains("prepare_run_elf_return"));
    assert!(finish_signal.contains("linux_task::finish_current_without_el0_return()"));

    let delivery = braced_body(
        &syscall[syscall
            .find("fn deliver_next_linux_signal(")
            .expect("default signal delivery")..],
    );
    assert!(delivery.contains("linux_process::linux_signal_delivery_route("));
    assert!(delivery.contains("terminate_linux_process_by_signal(current.tgid, signum)"));
    assert!(delivery.contains("regs[0] = launch_id"));
    assert!(!delivery.contains("process_manager().terminate_process(current.tgid)"));

    let kill = braced_body(
        &syscall[syscall
            .find("pub fn sys_kill(")
            .expect("process-directed kill")..],
    );
    assert!(kill.contains("linux_process::by_pid(target_pid)"));
    assert!(kill.contains("if signum == LINUX_SIGKILL"));
    assert!(kill.contains("terminate_linux_process_by_signal(target_pid, signum)"));
    assert!(kill.contains("queue_process_linux_signal_and_wake(target_pid"));

    let sigqueue = braced_body(
        &syscall[syscall
            .find("pub fn sys_rt_sigqueueinfo(")
            .expect("process queued signal syscall")..],
    );
    assert!(sigqueue.contains("if sig == LINUX_SIGKILL"));
    assert!(sigqueue.contains("terminate_linux_process_by_signal(pid, sig)"));
    assert!(!sigqueue.contains("linux_process::terminate_by_signal(pid, sig)"));

    let process_queue = braced_body(
        &syscall[syscall
            .find("fn queue_process_linux_signal_and_wake(")
            .expect("process signal queue")..],
    );
    assert!(
        process_queue
            .matches("with_linux_process_signal_state_for(tgid")
            .count()
            >= 2,
        "failed signal-wait completion must roll back in the target process"
    );
    assert!(process_queue.contains("rollback_reservation(reservation, record)"));

    let fork_process = braced_body(
        &process[process
            .find("pub(crate) fn clone_signal_state_for_fork(")
            .expect("fork process signal cloning")..],
    );
    assert!(fork_process.contains("fork_child(LinuxPendingSignals::new())"));
    let fork_task = braced_body(
        &task[task
            .find("pub(crate) fn reserve_fork_task(")
            .expect("fork child task reservation")..],
    );
    assert!(fork_task.contains("prepare_linux_fork_task_signal_state("));
    assert!(fork_task.contains("signal_state.reset_in_place()"));
    assert!(fork_task.contains("signal_state.mask = mask"));
    assert!(fork_task.contains("reserve_child(0, scheduler_id.0)"));

    let terminal_start = process
        .find("fn finish_terminal_process(")
        .expect("terminal child handling");
    let terminal_source = &process[terminal_start..];
    let terminal_signature = &terminal_source[..terminal_source
        .find('{')
        .expect("terminal child handling body")];
    assert!(terminal_signature.contains("pid: usize"));
    assert!(!terminal_signature.contains("LinuxProcessCore"));
    let terminal = braced_body(terminal_source);
    assert!(terminal.contains("LINUX_SIGCHLD"));
    assert!(terminal.contains("LINUX_SA_NOCLDWAIT"));
    assert!(terminal.contains("LINUX_LAUNCH_REAPER_PID"));
    assert!(terminal.contains("linux_sigchld_exit_policy("));
    assert!(terminal.contains("super::queue_process_linux_signal_and_wake("));
    assert!(terminal.contains("linux_task::wake_process_waiters(parent_pid)"));
    let descendant_transition = braced_body(
        &terminal[terminal
            .find("let transition = with_runtime(|runtime|")
            .expect("atomic descendant terminal transition")..],
    );
    assert!(descendant_transition.contains("let process = runtime"));
    assert!(descendant_transition.contains(".by_pid(pid)"));
    assert!(descendant_transition.contains("process.parent_pid"));
    assert!(descendant_transition.contains(".terminate_child(pid, wait_status, policy)"));
    assert!(descendant_transition.contains("reparent_children_to_launch_reaper(pid)"));
    assert!(terminal.contains("let _ = apply_linux_terminal_child_transition("));
    let queue = terminal
        .find("super::queue_process_linux_signal_and_wake(")
        .expect("SIGCHLD notification");
    let wake = terminal
        .find("linux_task::wake_process_waiters(parent_pid)")
        .expect("matching waiter wake");
    assert!(wake > queue || terminal.contains("notify_parent"));

    let root_transition = braced_body(
        &terminal[terminal
            .find("let descendant_count = with_runtime(|runtime|")
            .expect("atomic launch-root terminal transition")..],
    );
    assert!(root_transition.contains("runtime.processes.exit(pid, wait_status)"));
    assert!(root_transition.contains("runtime.signal_states[slot]"));
    assert!(root_transition.contains("adopt_launch_descendants(LINUX_ROOT_PID)"));

    let prepare_return = braced_body(
        &launcher[launcher
            .find("pub fn prepare_run_elf_return(")
            .expect("launcher kernel return preparation")..],
    );
    assert!(prepare_return.contains("set_kernel_resume("));
    assert!(prepare_return.contains("run_elf_launcher_resume as *const () as u64"));
}

#[test]
fn posix_process_runtime_results_separate_campaign_and_merge_head_evidence() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn sha256sum(bytes: &[u8]) -> String {
        let mut child = Command::new("sha256sum")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start sha256sum");
        child
            .stdin
            .take()
            .expect("sha256sum stdin")
            .write_all(bytes)
            .expect("hash evidence bytes");
        let output = child.wait_with_output().expect("finish sha256sum");
        assert!(
            output.status.success(),
            "sha256sum failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("sha256sum output is UTF-8")
            .split_whitespace()
            .next()
            .expect("sha256sum digest")
            .to_owned()
    }

    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let results = std::fs::read_to_string(
        repository.join("docs/posix/2026-08-06-aarch64-fork-process-runtime-results.md"),
    )
    .expect("read AArch64 fork process runtime results");

    let (historical, merge_head) = results
        .split_once("## Task 14 merge-head verification")
        .expect("Task 14 merge-head verification addendum");

    let historical_document = historical
        .strip_suffix('\n')
        .expect("one separator newline before Task 14 evidence");
    assert_eq!(historical_document.len(), 19_076);
    assert_eq!(
        sha256sum(historical_document.as_bytes()),
        "b2b56c1663dce1ee49324fdc383c0d9793069c9687fc8a1109a39fe3d5557a76"
    );

    assert_eq!(merge_head.len(), 8_721);
    assert_eq!(
        sha256sum(merge_head.as_bytes()),
        "ed96831991d93b2475e4948da31574d8afdff4a1213c7a9b7c79b72c2f99bad1"
    );

    for required in [
        "| Merge candidate | `599c4925dd708a21da1b8d9458fed0fa3232a63b` |",
        "| Merge-candidate host test counts | `make test` passed 253 unit tests, 80 integration contracts, 473 POSIX-tool tests, 4 launcher tests, and 8 linker-layout tests; the coverage run also passed 1 socket behavior test |",
        "| Final post-documentation host verification | `make test` passed 253 unit tests, 81 integration contracts, 473 POSIX-tool tests, 4 launcher tests, and 8 linker-layout tests |",
        "| Proof counts | Verus coverage audit passed; syscall 278, kernel objects 266, kernel low level 132, user level 172, and services 140: 988 verified, 0 errors |",
        "| AArch64 build and layout | passed; entry `0x40200000`, `.text [0x40200000,0x4027a000)`, `.rodata [0x4027a000,0x4a9c7000)`, `.data [0x4a9c7000,0x4bb40000)`, `.bss [0x4bb40000,0x4fb59000)`, `.stack [0x4fb59000,0x4fbd9000)` |",
        "| QEMU smoke | passed with SMP=4 and 512 MiB on private disk `target/posix/aarch64/smros-fxfs-task14-smoke-599c4925dd70.img` |",
        "| POSIX stage | 1,979 C sources; 1,941 compile pass, 38 compile fail; 1,680 link pass, 2 link fail; 169 shell tests unported; 119,397,116 staged bytes |",
        "| Host coverage | failed as required below 100%: 8,690/8,768 lines, 99.11%, 78 uncovered; `make coverage-host` exited 2 |",
        "| Coverity | unavailable: `cov-build`, `cov-analyze`, and `cov-format-errors` were all missing; no findings or analysis coverage is claimed |",
        "`9466093c93ba29f29e7a025ac98f13a79f2178c1d99274a837641aa98bf70cb8`",
        "`ef58bb15baf69fc731bdb64810bec7a64ab8559a31e7c4152be4852858042e7a`",
        "`2354fdb550290652373cd831c7489300bdd20344aa74fe151d8b3cfe0d009724`",
        "`fde6be8fee45bc42dcc8fcd6442fff2156f301f2560d64094f562b9efa430bfb`",
        "`cb4251d05bd5f23661c85b9d015a707b`",
        "| pass | 949 |",
        "| fail | 173 |",
        "| unresolved | 109 |",
        "| unsupported | 20 |",
        "| untested | 15 |",
        "| timeout | 332 |",
        "build coverage `1598/1637`, execution coverage\n`1598/1598`, runtime pass coverage `949/1598`, and program completion\n`1197/2054`",
        "The current quality record contains ten checks: eight passed, host Rust\ncoverage failed, and Coverity was unavailable. Its overall status is\n`failed`",
    ] {
        assert!(
            merge_head.contains(required),
            "merge-head evidence is missing `{required}`"
        );
    }
    assert!(merge_head.contains(
        "The earlier full campaign remains immutably bound to \
         `c0a513e75f7762b90e1e6de6ef27051e1add801d` as a historical baseline."
    ));
}
