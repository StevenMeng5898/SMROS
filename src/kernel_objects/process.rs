//! Zircon-style process and thread kernel object state.

use super::right::{trusted_process_right_profile, ProcessRightProfile, UserProcessKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessRecord {
    pub handle: u32,
    pub job_handle: u32,
    pub pid: usize,
    pub root_vmar_handle: u32,
    pub exited: bool,
    pub exit_code: i32,
    pub right_profile: ProcessRightProfile,
}

impl ProcessRecord {
    pub const fn new(
        handle: u32,
        job_handle: u32,
        pid: usize,
        root_vmar_handle: u32,
        right_profile: ProcessRightProfile,
    ) -> Self {
        Self {
            handle,
            job_handle,
            pid,
            root_vmar_handle,
            exited: false,
            exit_code: 0,
            right_profile,
        }
    }

    pub fn mark_exited(&mut self, exit_code: i32) -> Option<usize> {
        let pid_to_terminate = if !self.exited && self.pid != 0 {
            Some(self.pid)
        } else {
            None
        };
        self.exited = true;
        self.exit_code = exit_code;
        pid_to_terminate
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadRecord {
    pub handle: u32,
    pub process_handle: u32,
    pub entry_point: usize,
    pub stack_top: usize,
    pub arg1: usize,
    pub arg2: usize,
    pub started: bool,
    pub exited: bool,
}

impl ThreadRecord {
    pub const fn new(handle: u32, process_handle: u32, entry_point: usize) -> Self {
        Self {
            handle,
            process_handle,
            entry_point,
            stack_top: 0,
            arg1: 0,
            arg2: 0,
            started: false,
            exited: false,
        }
    }

    pub fn start(
        &mut self,
        entry_point: usize,
        stack_top: usize,
        arg1: usize,
        arg2: usize,
    ) -> bool {
        if self.exited || self.started || entry_point == 0 {
            return false;
        }
        self.entry_point = entry_point;
        self.stack_top = stack_top;
        self.arg1 = arg1;
        self.arg2 = arg2;
        self.started = true;
        true
    }

    pub fn mark_exited(&mut self) {
        self.exited = true;
    }
}

pub const fn default_process_right_profile() -> ProcessRightProfile {
    trusted_process_right_profile(UserProcessKind::Test)
}

