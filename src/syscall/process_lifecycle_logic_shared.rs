/// Legal process and task lifecycle edges.
///
/// Callers pass the state tokens they use (`LinuxProcessState` / `LinuxTaskState`
/// in the kernel, the same tokens as `u8` codes in the proof harness). A `None`
/// result means the edge is refused and the slot must stay unchanged.
///
/// Process edges: Empty -> Reserved -> Publishing -> Running, Reserved -> Running,
/// Reserved|Publishing -> Empty, Running <-> Exiting, Running|Exiting -> Zombie,
/// Zombie -> Empty. Wait reaps a zombie once; the emptied slot is not a zombie.
///
/// Task edges: Empty -> Starting -> Runnable, Runnable <-> Blocked,
/// Runnable|Blocked -> Exited -> Empty. Retire accepts only Exited.

#[allow(unused_macros)]
macro_rules! smros_process_reserve_body {
    (
        $parent_running:expr,
        $slot_empty:expr,
        $pid_in_range:expr,
        $duplicate:expr,
        $reserved:expr
    ) => {{
        if $parent_running && $slot_empty && $pid_in_range && !$duplicate {
            Some($reserved)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_publish_body {
    (
        $state:expr,
        $pid:expr,
        $parent:expr,
        $want_pid:expr,
        $want_parent:expr,
        $reserved:expr,
        $running:expr
    ) => {{
        if $state == $reserved && $pid == $want_pid && $parent == $want_parent {
            Some($running)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_publish_fork_body {
    (
        $state:expr,
        $pid:expr,
        $parent:expr,
        $want_pid:expr,
        $want_parent:expr,
        $reserved:expr,
        $publishing:expr
    ) => {{
        if $state == $reserved && $pid == $want_pid && $parent == $want_parent {
            Some($publishing)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_complete_fork_body {
    (
        $state:expr,
        $pid:expr,
        $parent:expr,
        $want_pid:expr,
        $want_parent:expr,
        $publishing:expr,
        $running:expr
    ) => {{
        if $state == $publishing && $pid == $want_pid && $parent == $want_parent {
            Some($running)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_rollback_body {
    (
        $state:expr,
        $pid:expr,
        $parent:expr,
        $want_pid:expr,
        $want_parent:expr,
        $reserved:expr,
        $empty:expr
    ) => {{
        if $state == $reserved && $pid == $want_pid && $parent == $want_parent {
            Some($empty)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_rollback_fork_body {
    (
        $state:expr,
        $pid:expr,
        $parent:expr,
        $want_pid:expr,
        $want_parent:expr,
        $reserved:expr,
        $publishing:expr,
        $empty:expr
    ) => {{
        if ($state == $reserved || $state == $publishing)
            && $pid == $want_pid
            && $parent == $want_parent
        {
            Some($empty)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_begin_exit_body {
    ($state:expr, $running:expr, $exiting:expr) => {{
        if $state == $running {
            Some($exiting)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_abort_exit_body {
    ($state:expr, $exiting:expr, $running:expr) => {{
        if $state == $exiting {
            Some($running)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_process_exit_body {
    ($state:expr, $running:expr, $exiting:expr, $zombie:expr) => {{
        if $state == $running || $state == $exiting {
            Some($zombie)
        } else {
            None
        }
    }};
}

/// Reap only a zombie whose parent and pid match. The successor token is empty,
/// so a second reap of that slot is refused.
#[allow(unused_macros)]
macro_rules! smros_process_reap_body {
    (
        $state:expr,
        $parent:expr,
        $pid:expr,
        $want_parent:expr,
        $want_pid:expr,
        $zombie:expr,
        $empty:expr
    ) => {{
        if $state == $zombie && $parent == $want_parent && $pid == $want_pid {
            Some($empty)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_reserve_body {
    ($state:expr, $empty:expr, $starting:expr) => {{
        if $state == $empty {
            Some($starting)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_publish_body {
    (
        $state:expr,
        $tid:expr,
        $scheduler_thread:expr,
        $want_tid:expr,
        $want_scheduler_thread:expr,
        $starting:expr,
        $runnable:expr
    ) => {{
        if $state == $starting && $tid == $want_tid && $scheduler_thread == $want_scheduler_thread {
            Some($runnable)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_block_body {
    ($state:expr, $runnable:expr, $blocked:expr, $reason_is_none:expr) => {{
        if !$reason_is_none && $state == $runnable {
            Some($blocked)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_wake_body {
    ($state:expr, $blocked:expr, $runnable:expr) => {{
        if $state == $blocked {
            Some($runnable)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_exit_body {
    ($state:expr, $runnable:expr, $blocked:expr, $exited:expr) => {{
        if $state == $runnable || $state == $blocked {
            Some($exited)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_task_retire_body {
    ($state:expr, $exited:expr, $empty:expr) => {{
        if $state == $exited {
            Some($empty)
        } else {
            None
        }
    }};
}

/// Empty slot becomes Running. Used by root registration and the kernel
/// dispatch identity. Not the Reserved path.
#[allow(unused_macros)]
macro_rules! smros_process_install_running_body {
    ($state:expr, $empty:expr, $running:expr) => {{
        if $state == $empty {
            Some($running)
        } else {
            None
        }
    }};
}

/// Occupied slot becomes Empty. Used when the dispatch identity is abandoned.
#[allow(unused_macros)]
macro_rules! smros_process_clear_occupied_body {
    ($state:expr, $empty:expr) => {{
        if $state != $empty {
            Some($empty)
        } else {
            None
        }
    }};
}

/// A visible launch descendant (Running, Exiting, or Zombie) becomes Empty.
/// `stopped` is not a state in this model.
#[allow(unused_macros)]
macro_rules! smros_process_clear_visible_body {
    ($state:expr, $running:expr, $exiting:expr, $zombie:expr, $empty:expr) => {{
        if $state == $running || $state == $exiting || $state == $zombie {
            Some($empty)
        } else {
            None
        }
    }};
}

/// Empty slot becomes Runnable. Root and dispatch task installation.
#[allow(unused_macros)]
macro_rules! smros_task_install_runnable_body {
    ($state:expr, $empty:expr, $runnable:expr) => {{
        if $state == $empty {
            Some($runnable)
        } else {
            None
        }
    }};
}

/// Starting or Runnable reservation becomes Empty.
#[allow(unused_macros)]
macro_rules! smros_task_rollback_body {
    ($state:expr, $starting:expr, $runnable:expr, $empty:expr) => {{
        if $state == $starting || $state == $runnable {
            Some($empty)
        } else {
            None
        }
    }};
}
