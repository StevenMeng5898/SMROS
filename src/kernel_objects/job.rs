//! Zircon-style job kernel object state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobRecord {
    pub handle: u32,
    pub parent: Option<u32>,
    pub child_count: usize,
    pub policy_count: usize,
    pub critical_process: Option<u32>,
}

impl JobRecord {
    pub const fn new(handle: u32, parent: Option<u32>) -> Self {
        Self {
            handle,
            parent,
            child_count: 0,
            policy_count: 0,
            critical_process: None,
        }
    }

    pub fn add_child(&mut self) {
        self.child_count = self.child_count.saturating_add(1);
    }

    pub fn remove_child(&mut self) {
        self.child_count = self.child_count.saturating_sub(1);
    }

    pub fn set_policy_count(&mut self, policy_count: usize) {
        self.policy_count = policy_count;
    }

    pub fn set_critical_process(&mut self, process: u32) {
        self.critical_process = Some(process);
    }
}
