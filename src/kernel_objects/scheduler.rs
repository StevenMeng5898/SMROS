#![allow(dead_code)]

//! Preemptive scheduler
//!
//! This module implements Round-Robin, EDF, and credit scheduling for SMROS.
//! It manages multiple threads and performs context switching on timer ticks.

use crate::kernel_lowlevel::thread::{
    self, SendPtr, ThreadControlBlock, ThreadId, ThreadStack, ThreadState, DEFAULT_STACK_SIZE,
    MAX_THREADS,
};
use crate::kernel_objects::object_logic;
use core::cell::UnsafeCell;
use core::ptr;

include!("scheduler_logic_shared.rs");

/// A Sync wrapper around UnsafeCell that is safe to use as a static.
/// SAFETY: This is safe because the scheduler ensures only one thread accesses
/// the idle stack at a time (during init).
struct SyncUnsafeCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncUnsafeCell<T> {}
impl<T> SyncUnsafeCell<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// Maximum number of CPUs for thread binding.
pub const MAX_CPUS: usize = crate::kernel_lowlevel::smp::MAX_CPUS;
const DEFAULT_TIME_SLICE_TICKS: u32 = 10;
const DEFAULT_EDF_PERIOD_TICKS: u32 = 50;
const DEFAULT_CREDIT: i32 = 100;
const MAX_CREDIT_WEIGHT: u32 = (i32::MAX as u32) / (DEFAULT_CREDIT as u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulePolicy {
    RoundRobin,
    Edf,
    Credit,
}

impl SchedulePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            SchedulePolicy::RoundRobin => "round-robin",
            SchedulePolicy::Edf => "edf",
            SchedulePolicy::Credit => "credit",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        smros_sched_policy_from_match_flags_body!(
            value.eq_ignore_ascii_case("rr"),
            value.eq_ignore_ascii_case("round-robin"),
            value.eq_ignore_ascii_case("edf"),
            value.eq_ignore_ascii_case("credit"),
            SchedulePolicy::RoundRobin,
            SchedulePolicy::Edf,
            SchedulePolicy::Credit
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThreadScheduleInfo {
    pub deadline_tick: u64,
    pub period_ticks: u32,
    pub credit: i32,
    pub credit_cap: i32,
    pub weight: u32,
}

impl ThreadScheduleInfo {
    pub const fn empty() -> Self {
        Self {
            deadline_tick: u64::MAX,
            period_ticks: DEFAULT_EDF_PERIOD_TICKS,
            credit: 0,
            credit_cap: DEFAULT_CREDIT,
            weight: 1,
        }
    }

    pub const fn idle() -> Self {
        Self {
            deadline_tick: u64::MAX,
            period_ticks: DEFAULT_EDF_PERIOD_TICKS,
            credit: 0,
            credit_cap: 0,
            weight: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct ScheduleTestTask {
    id: usize,
    ready: bool,
    deadline_tick: u64,
    credit: i32,
    cpu_affinity: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerPolicyTestResult {
    pub round_robin: usize,
    pub edf: usize,
    pub credit: usize,
    pub cpu_filtered: usize,
}

/// Scheduler structure
pub struct Scheduler {
    /// Thread control blocks
    threads: [ThreadControlBlock; MAX_THREADS],

    /// Current running thread ID
    current_thread: ThreadId,

    /// Next thread to run (for round-robin)
    next_thread: usize,

    /// Number of active threads
    active_threads: usize,

    /// Scheduler tick count
    tick_count: u64,

    /// Time slice per thread (in ticks)
    time_slice_ticks: u32,

    /// Active scheduling policy
    policy: SchedulePolicy,

    /// Per-thread policy metadata
    schedule_info: [ThreadScheduleInfo; MAX_THREADS],

    /// Static stack for idle thread
    idle_stack: SendPtr,
}

// SAFETY: The scheduler is only accessed from one thread at a time.
// Cooperative scheduling and interrupt disabling during context switches
// ensure no concurrent mutable access occurs.
unsafe impl Send for Scheduler {}
// SAFETY: Sync is safe because all mutable state is either atomic or
// protected by the scheduler's cooperative scheduling model.
unsafe impl Sync for Scheduler {}

/// Global scheduler instance wrapped in UnsafeCell for interior mutability.
struct SchedulerCell(UnsafeCell<Scheduler>);

// SAFETY: SchedulerCell provides interior mutability for the global scheduler.
// Access is serialized by the scheduler's design - only one thread runs at a time
// and interrupts are disabled during context switches.
unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Scheduler::new()));

/// Get a mutable reference to the global scheduler.
// SAFETY: This is safe because we only access the scheduler from one thread at a time
// and we ensure no references are held across context switches.
pub fn scheduler() -> &'static mut Scheduler {
    unsafe { &mut *SCHEDULER.0.get() }
}

fn task_allowed_on_cpu(task: ScheduleTestTask, cpu_id: Option<usize>) -> bool {
    match cpu_id {
        Some(cpu) => smros_sched_task_allowed_on_cpu_body!(
            task.cpu_affinity.is_some(),
            task.cpu_affinity.unwrap_or(0),
            true,
            cpu
        ),
        None => true,
    }
}

fn pick_round_robin_from_tasks(
    tasks: &[ScheduleTestTask],
    start_id: usize,
    cpu_id: Option<usize>,
) -> Option<usize> {
    if tasks.is_empty() {
        return None;
    }
    for offset in 0..tasks.len() {
        let wanted_id = start_id.saturating_add(offset);
        for task in tasks {
            if task.id == wanted_id && task.ready && task_allowed_on_cpu(*task, cpu_id) {
                return Some(task.id);
            }
        }
    }
    for task in tasks {
        if task.ready && task_allowed_on_cpu(*task, cpu_id) {
            return Some(task.id);
        }
    }
    None
}

fn pick_edf_from_tasks(tasks: &[ScheduleTestTask], cpu_id: Option<usize>) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_deadline = u64::MAX;
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && smros_sched_edf_better_body!(task.deadline_tick, best.is_some(), best_deadline)
        {
            best = Some(task.id);
            best_deadline = task.deadline_tick;
        }
    }
    best
}

fn pick_credit_from_tasks(tasks: &[ScheduleTestTask], cpu_id: Option<usize>) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_credit = i32::MIN;
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && smros_sched_credit_better_body!(task.credit, best.is_some(), best_credit)
        {
            best = Some(task.id);
            best_credit = task.credit;
        }
    }
    best
}

impl Scheduler {
    /// Create a new scheduler instance
    pub const fn new() -> Self {
        Scheduler {
            threads: [const { ThreadControlBlock::new() }; MAX_THREADS],
            current_thread: ThreadId::INVALID,
            next_thread: 0,
            active_threads: 0,
            tick_count: 0,
            time_slice_ticks: DEFAULT_TIME_SLICE_TICKS,
            policy: SchedulePolicy::RoundRobin,
            schedule_info: [const { ThreadScheduleInfo::empty() }; MAX_THREADS],
            idle_stack: SendPtr(ptr::null_mut()),
        }
    }

    /// Initialize the scheduler
    pub fn init(&mut self) {
        // Initialize all TCBs as empty
        for i in 0..MAX_THREADS {
            self.threads[i].id = ThreadId(i);
            self.threads[i].state = ThreadState::Empty;
            self.schedule_info[i] = ThreadScheduleInfo::empty();
        }

        // Allocate idle thread stack using a Sync wrapper around UnsafeCell
        static IDLE_STACK: SyncUnsafeCell<[u8; DEFAULT_STACK_SIZE]> =
            SyncUnsafeCell::new([0; DEFAULT_STACK_SIZE]);
        // SAFETY: We're single-threaded during init, so no aliasing mutable
        // references exist. The SyncUnsafeCell provides interior mutability safely.
        self.idle_stack = SendPtr(unsafe { (*IDLE_STACK.get()).as_mut_ptr() });

        // Create idle thread (thread 0)
        self.create_idle_thread();

        self.current_thread = ThreadId::IDLE;
        self.next_thread = 1;
        self.active_threads = 1;
        self.tick_count = 0;
        self.policy = SchedulePolicy::RoundRobin;
    }

    /// Create the idle thread
    fn create_idle_thread(&mut self) {
        let tcb = &mut self.threads[0];
        tcb.init_idle(idle_thread_entry, self.idle_stack.0, DEFAULT_STACK_SIZE);
        self.schedule_info[0] = ThreadScheduleInfo::idle();
    }

    /// Create a new thread
    pub fn create_thread(
        &mut self,
        entry: extern "C" fn() -> !,
        name: &'static str,
    ) -> Option<ThreadId> {
        self.create_thread_on_cpu(entry, name, None)
    }

    /// Create a new thread bound to a specific CPU
    pub fn create_thread_on_cpu(
        &mut self,
        entry: extern "C" fn() -> !,
        name: &'static str,
        cpu_affinity: Option<usize>,
    ) -> Option<ThreadId> {
        // Find an empty slot
        for i in 1..MAX_THREADS {
            if self.threads[i].state == ThreadState::Empty {
                // Allocate stack
                let stack = ThreadStack::alloc(DEFAULT_STACK_SIZE)?;

                let tcb = &mut self.threads[i];
                tcb.init(
                    ThreadId(i),
                    entry,
                    name,
                    stack.as_ptr(),
                    DEFAULT_STACK_SIZE,
                    self.time_slice_ticks,
                    cpu_affinity,
                );
                self.init_thread_schedule_info(i);

                // Leak the stack (it will be freed when thread terminates)
                core::mem::forget(stack);

                self.active_threads += 1;

                return Some(ThreadId(i));
            }
        }

        None // No available slots
    }

    fn init_thread_schedule_info(&mut self, index: usize) {
        let phase = (index as u64).saturating_mul(5);
        let period = DEFAULT_EDF_PERIOD_TICKS;
        self.schedule_info[index] = ThreadScheduleInfo {
            deadline_tick: self
                .tick_count
                .saturating_add(period as u64)
                .saturating_add(phase),
            period_ticks: period,
            credit: DEFAULT_CREDIT,
            credit_cap: DEFAULT_CREDIT,
            weight: 1,
        };
    }

    /// Get the active scheduling policy.
    pub fn policy(&self) -> SchedulePolicy {
        self.policy
    }

    /// Set the active scheduling policy.
    pub fn set_policy(&mut self, policy: SchedulePolicy) {
        self.policy = policy;
        if policy == SchedulePolicy::Credit {
            self.refill_credits();
        }
        crate::kobj_info!("scheduler", "policy set to {}", policy.as_str());
    }

    /// Set EDF timing metadata for a thread.
    pub fn set_thread_deadline(
        &mut self,
        id: ThreadId,
        deadline_tick: u64,
        period_ticks: u32,
    ) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS || period_ticks == 0 {
            return false;
        }
        self.schedule_info[id.0].deadline_tick = deadline_tick;
        self.schedule_info[id.0].period_ticks = period_ticks;
        true
    }

    /// Set credit scheduler metadata for a thread.
    pub fn set_thread_credit(&mut self, id: ThreadId, credit: i32, cap: i32, weight: u32) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS || cap < 0 || credit < 0 || credit > cap || weight == 0
        {
            return false;
        }
        self.schedule_info[id.0].credit = credit;
        self.schedule_info[id.0].credit_cap = cap;
        self.schedule_info[id.0].weight = weight;
        true
    }

    pub fn thread_schedule_info(&self, id: ThreadId) -> Option<ThreadScheduleInfo> {
        if id.0 < MAX_THREADS {
            Some(self.schedule_info[id.0])
        } else {
            None
        }
    }

    pub fn time_slice_ticks(&self) -> u32 {
        self.time_slice_ticks
    }

    pub fn active_threads(&self) -> usize {
        self.active_threads
    }

    /// Get the current thread ID
    pub fn current(&self) -> ThreadId {
        self.current_thread
    }

    /// Get a reference to a thread's TCB
    pub fn get_thread(&self, id: ThreadId) -> Option<&ThreadControlBlock> {
        if id.0 < MAX_THREADS {
            Some(&self.threads[id.0])
        } else {
            None
        }
    }

    /// Get a mutable reference to a thread's TCB
    pub fn get_thread_mut(&mut self, id: ThreadId) -> Option<&mut ThreadControlBlock> {
        if id.0 < MAX_THREADS {
            Some(&mut self.threads[id.0])
        } else {
            None
        }
    }

    /// Schedule the next thread using the active policy.
    pub fn schedule_next(&mut self) -> Option<ThreadId> {
        self.schedule_next_filtered(None)
    }

    /// Schedule the next thread for a specific CPU using the active policy.
    pub fn schedule_next_for_cpu(&mut self, cpu_id: usize) -> Option<ThreadId> {
        self.schedule_next_filtered(Some(cpu_id))
    }

    fn schedule_next_filtered(&mut self, cpu_id: Option<usize>) -> Option<ThreadId> {
        if self.active_threads <= 1 {
            return Some(ThreadId::IDLE);
        }

        let selected = match self.policy {
            SchedulePolicy::RoundRobin => self.pick_round_robin(cpu_id),
            SchedulePolicy::Edf => self.pick_edf(cpu_id),
            SchedulePolicy::Credit => self.pick_credit(cpu_id),
        };

        selected.map(ThreadId).or(Some(ThreadId::IDLE))
    }

    fn thread_allowed_on_cpu(&self, idx: usize, cpu_id: Option<usize>) -> bool {
        match cpu_id {
            Some(cpu) => {
                let thread_cpu = self.threads[idx].cpu_affinity;
                smros_sched_task_allowed_on_cpu_body!(
                    thread_cpu.is_some(),
                    thread_cpu.unwrap_or(0),
                    true,
                    cpu
                )
            }
            None => true,
        }
    }

    fn candidate_can_run(&self, idx: usize, current: usize, cpu_id: Option<usize>) -> bool {
        object_logic::scheduler_can_run(idx, current, self.threads[idx].state == ThreadState::Ready)
            && self.thread_allowed_on_cpu(idx, cpu_id)
    }

    fn pick_round_robin(&mut self, cpu_id: Option<usize>) -> Option<usize> {
        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);

            // Skip the current thread and idle thread (unless it's the only option)
            if self.candidate_can_run(idx, current, cpu_id) {
                self.next_thread = (idx + 1) % MAX_THREADS;
                return Some(idx);
            }

            attempts += 1;
        }

        None
    }

    fn pick_edf(&mut self, cpu_id: Option<usize>) -> Option<usize> {
        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;
        let mut best: Option<usize> = None;
        let mut best_deadline = u64::MAX;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);
            if self.candidate_can_run(idx, current, cpu_id) {
                let deadline = self.schedule_info[idx].deadline_tick;
                if smros_sched_edf_better_body!(deadline, best.is_some(), best_deadline) {
                    best = Some(idx);
                    best_deadline = deadline;
                }
            }
            attempts += 1;
        }

        if let Some(idx) = best {
            self.next_thread = (idx + 1) % MAX_THREADS;
        }
        best
    }

    fn pick_credit(&mut self, cpu_id: Option<usize>) -> Option<usize> {
        if !self.any_ready_credit(cpu_id) {
            self.refill_credits();
        }

        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;
        let mut best: Option<usize> = None;
        let mut best_credit = i32::MIN;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);
            if self.candidate_can_run(idx, current, cpu_id) {
                let credit = self.schedule_info[idx].credit;
                if smros_sched_credit_better_body!(credit, best.is_some(), best_credit) {
                    best = Some(idx);
                    best_credit = credit;
                }
            }
            attempts += 1;
        }

        if let Some(idx) = best {
            self.next_thread = (idx + 1) % MAX_THREADS;
        }
        best
    }

    fn any_ready_credit(&self, cpu_id: Option<usize>) -> bool {
        let current = self.current_thread.0;
        for idx in 1..MAX_THREADS {
            if self.candidate_can_run(idx, current, cpu_id) && self.schedule_info[idx].credit > 0 {
                return true;
            }
        }
        false
    }

    fn refill_credits(&mut self) {
        for idx in 1..MAX_THREADS {
            if self.threads[idx].state == ThreadState::Ready
                || self.threads[idx].state == ThreadState::Running
            {
                let info = &mut self.schedule_info[idx];
                info.credit = smros_sched_refill_credit_body!(
                    info.credit_cap,
                    info.weight,
                    DEFAULT_CREDIT,
                    MAX_CREDIT_WEIGHT
                );
            }
        }
    }

    fn advance_deadline(&mut self, idx: usize) {
        if idx == 0 || idx >= MAX_THREADS {
            return;
        }
        let info = &mut self.schedule_info[idx];
        info.deadline_tick = smros_sched_advance_deadline_body!(
            info.deadline_tick,
            self.tick_count,
            info.period_ticks
        );
    }

    pub fn run_policy_self_test(&self) -> SchedulerPolicyTestResult {
        let tasks = [
            ScheduleTestTask {
                id: 1,
                ready: true,
                deadline_tick: 90,
                credit: 30,
                cpu_affinity: None,
            },
            ScheduleTestTask {
                id: 2,
                ready: true,
                deadline_tick: 40,
                credit: 10,
                cpu_affinity: Some(1),
            },
            ScheduleTestTask {
                id: 3,
                ready: true,
                deadline_tick: 70,
                credit: 80,
                cpu_affinity: None,
            },
            ScheduleTestTask {
                id: 4,
                ready: false,
                deadline_tick: 10,
                credit: 200,
                cpu_affinity: None,
            },
        ];

        SchedulerPolicyTestResult {
            round_robin: pick_round_robin_from_tasks(&tasks, 2, None).unwrap_or(0),
            edf: pick_edf_from_tasks(&tasks, None).unwrap_or(0),
            credit: pick_credit_from_tasks(&tasks, None).unwrap_or(0),
            cpu_filtered: pick_edf_from_tasks(&tasks, Some(0)).unwrap_or(0),
        }
    }

    /// Handle timer tick (called from interrupt handler)
    pub fn on_timer_tick(&mut self) {
        self.tick_count += 1;

        // Decrement current thread's time slice
        let current = self.current_thread;
        let policy = self.policy;
        let tick_count = self.tick_count;
        let mut advance_deadline = false;
        let mut force_preempt = false;
        if let Some(tcb) = self.get_thread_mut(current) {
            if tcb.time_slice > 0 {
                tcb.time_slice = smros_sched_time_slice_after_tick_body!(tcb.time_slice);
            }

            tcb.total_ticks += 1;
            if current.0 != 0 && policy == SchedulePolicy::Edf && tcb.time_slice == 0 {
                force_preempt = true;
            }
        }

        if current.0 != 0 {
            match policy {
                SchedulePolicy::Edf => {
                    if force_preempt || tick_count >= self.schedule_info[current.0].deadline_tick {
                        if let Some(tcb) = self.get_thread_mut(current) {
                            tcb.time_slice = 0;
                        }
                        advance_deadline = true;
                    }
                }
                SchedulePolicy::Credit => {
                    let exhausted = {
                        let info = &mut self.schedule_info[current.0];
                        info.credit = smros_sched_credit_after_tick_body!(info.credit);
                        info.credit <= 0
                    };
                    if exhausted {
                        if let Some(tcb) = self.get_thread_mut(current) {
                            tcb.time_slice = 0;
                        }
                    }
                }
                SchedulePolicy::RoundRobin => {}
            }
        }

        if advance_deadline {
            self.advance_deadline(current.0);
        }
    }

    /// Check if preemption is needed
    pub fn should_preempt(&self) -> bool {
        if let Some(tcb) = self.get_thread(self.current_thread) {
            if self.active_threads <= 1 {
                return false;
            }
            let info = self.schedule_info[self.current_thread.0];
            smros_sched_should_preempt_body!(
                self.policy,
                SchedulePolicy::RoundRobin,
                SchedulePolicy::Edf,
                SchedulePolicy::Credit,
                tcb.time_slice,
                self.active_threads,
                info.deadline_tick,
                self.tick_count,
                info.credit
            )
        } else {
            false
        }
    }

    /// Reset time slice for a thread
    pub fn reset_time_slice(&mut self, id: ThreadId) {
        let time_slice = self.time_slice_ticks;
        if let Some(tcb) = self.get_thread_mut(id) {
            tcb.time_slice = time_slice;
        }
    }

    /// Block the current thread
    pub fn block_current(&mut self) {
        if let Some(tcb) = self.get_thread_mut(self.current_thread) {
            tcb.state = ThreadState::Blocked;
            tcb.time_slice = 0;
        }
    }

    /// Terminate the current thread
    pub fn terminate_current(&mut self) {
        let current_id = self.current_thread;
        let stack_info = if let Some(tcb) = self.get_thread_mut(current_id) {
            tcb.state = ThreadState::Terminated;
            tcb.time_slice = 0;
            (tcb.stack.0, tcb.stack_size, tcb.id.0)
        } else {
            (ptr::null_mut(), 0, 0)
        };

        self.active_threads -= 1;

        // Free stack (only for non-idle threads)
        if !stack_info.0.is_null() && stack_info.2 != 0 {
            // SAFETY: stack was allocated with Layout::from_size_align(DEFAULT_STACK_SIZE, 16)
            if let Ok(layout) = alloc::alloc::Layout::from_size_align(stack_info.1, 16) {
                unsafe {
                    alloc::alloc::dealloc(stack_info.0, layout);
                }
            }
        }
    }

    /// Mark the current thread terminated without freeing its stack.
    ///
    /// This is used by EL0 launcher return paths that are still executing on
    /// the launcher stack while selecting the next runnable thread.
    pub fn finish_current_without_stack_free(&mut self) {
        let current_id = self.current_thread;
        if let Some(tcb) = self.get_thread_mut(current_id) {
            if tcb.state != ThreadState::Terminated {
                tcb.state = ThreadState::Terminated;
                tcb.time_slice = 0;
                if self.active_threads > 0 {
                    self.active_threads -= 1;
                }
            }
        }
    }

    /// Get tick count
    pub fn get_tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Print scheduler status
    pub fn print_status(&self, serial: &mut crate::kernel_lowlevel::serial::Serial) {
        serial.write_str("\n=== Scheduler Status ===\n");
        serial.write_str("Active threads: ");
        print_number(serial, self.active_threads as u32);
        serial.write_str("\n");
        serial.write_str("Current thread: ");
        print_number(serial, self.current_thread.0 as u32);
        serial.write_str("\n");
        serial.write_str("Tick count: ");
        print_number(serial, self.tick_count as u32);
        serial.write_str("\n");
        serial.write_str("Policy: ");
        serial.write_str(self.policy.as_str());
        serial.write_str("\n");
        serial.write_str("\nThread Table:\n");
        serial
            .write_str("ID  State      Name        CPU  TimeSlice  TotalTicks  Deadline  Credit\n");

        for i in 0..MAX_THREADS {
            let tcb = &self.threads[i];
            if tcb.state != ThreadState::Empty {
                tcb.print_info(serial);
                serial.write_str("    sched deadline=");
                print_number_u64(serial, self.schedule_info[i].deadline_tick);
                serial.write_str(" period=");
                print_number(serial, self.schedule_info[i].period_ticks);
                serial.write_str(" credit=");
                print_i32(serial, self.schedule_info[i].credit);
                serial.write_str("/");
                print_i32(serial, self.schedule_info[i].credit_cap);
                serial.write_str(" weight=");
                print_number(serial, self.schedule_info[i].weight);
                serial.write_str("\n");
            }
        }

        serial.write_str("=========================\n");
    }
}

/// Idle thread entry point
extern "C" fn idle_thread_entry() -> ! {
    loop {
        // Immediately try to schedule another thread
        // If shell (or other threads) are ready, switch to them
        // This prevents deadlocks and ensures cooperative scheduling
        schedule();

        // If we returned here, no other threads were ready
        // Wait for interrupt (timer will trigger scheduler check)
        thread::wait_for_interrupt();
    }
}

/// Perform a context switch to the next thread
pub fn schedule() {
    let s = scheduler();

    // Find next thread to run
    if let Some(next_id) = s.schedule_next() {
        let current_id = s.current_thread;

        if next_id == current_id {
            // No need to switch
            return;
        }

        // Update states - get raw pointers first to avoid borrow issues
        let current_tcb_ptr = s.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
        let next_tcb_ptr = s.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

        // Update states through raw pointers
        unsafe {
            if (*current_tcb_ptr).state == ThreadState::Running {
                (*current_tcb_ptr).state = ThreadState::Ready;
            }
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = s.time_slice_ticks;
        }

        s.current_thread = next_id;

        // Perform context switch
        // SAFETY: These pointers are valid TCB references
        unsafe {
            thread::switch_context(current_tcb_ptr, next_tcb_ptr);
        }
    }
}

/// Start the first user thread (called from kernel_main)
/// This function never returns - it jumps to the first thread
pub fn start_first_thread() -> ! {
    let s = scheduler();

    // Find first ready thread
    let mut found_thread: Option<usize> = None;
    for i in 1..MAX_THREADS {
        if s.threads[i].state == ThreadState::Ready {
            found_thread = Some(i);
            break;
        }
    }

    if let Some(i) = found_thread {
        let next_id = ThreadId(i);

        // Update states - get raw pointers first
        let current_id = s.current_thread;
        let current_tcb_ptr = s.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
        let next_tcb_ptr = s.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

        // Update states through raw pointers
        unsafe {
            (*current_tcb_ptr).state = ThreadState::Ready;
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = s.time_slice_ticks;
        }

        s.current_thread = next_id;

        // Jump to the first thread (don't save current context)
        // SAFETY: This is safe - we're jumping to a valid thread entry point
        unsafe {
            thread::start_context(next_tcb_ptr);
        }
    }

    // No ready thread found, just halt
    loop {
        thread::wait_for_interrupt();
    }
}

/// Helper function to print a number
fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 10];
    let mut i = 0;

    while num > 0 && i < 10 {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    // Print in reverse order
    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_number_u64(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u64) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 20];
    let mut i = 0;

    while num > 0 && i < 20 {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_i32(serial: &mut crate::kernel_lowlevel::serial::Serial, value: i32) {
    if value < 0 {
        serial.write_byte(b'-');
        print_number(serial, value.saturating_abs() as u32);
    } else {
        print_number(serial, value as u32);
    }
}

/// Yield the current thread's time slice voluntarily
pub fn yield_now() {
    // Reset time slice to force preemption
    let s = scheduler();
    if let Some(tcb) = s.get_thread_mut(s.current_thread) {
        tcb.time_slice = 0;
    }
    schedule();
}

/// Yield the current thread's time slice on a specific CPU
pub fn yield_now_on_cpu(cpu_id: usize) {
    let s = scheduler();
    if let Some(tcb) = s.get_thread_mut(s.current_thread) {
        tcb.time_slice = 0;
    }
    schedule_on_cpu(cpu_id);
}

/// Perform a context switch to the next thread on a specific CPU
pub fn schedule_on_cpu(cpu_id: usize) {
    let s = scheduler();

    // Find next thread to run for this CPU
    if let Some(next_id) = s.schedule_next_for_cpu(cpu_id) {
        let current_id = s.current_thread;

        if next_id == current_id {
            // No need to switch
            return;
        }

        // Update states - get raw pointers first to avoid borrow issues
        let current_tcb_ptr = s.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
        let next_tcb_ptr = s.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

        // Update states through raw pointers
        unsafe {
            if (*current_tcb_ptr).state == ThreadState::Running {
                (*current_tcb_ptr).state = ThreadState::Ready;
            }
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = s.time_slice_ticks;
            // Mark which logical CPU this thread is running on
            (*next_tcb_ptr).current_cpu = Some(cpu_id);
        }

        s.current_thread = next_id;

        // Perform context switch
        // SAFETY: These pointers are valid TCB references
        unsafe {
            thread::switch_context(current_tcb_ptr, next_tcb_ptr);
        }
    }
}

/// Start the first user thread on a specific CPU (called from secondary CPU entry)
/// This function never returns - it jumps to the first thread for this CPU
pub fn start_first_thread_for_cpu(cpu_id: usize) -> ! {
    let s = scheduler();

    // Mark CPU as fully online before trying to start threads
    crate::kernel_lowlevel::smp::mark_cpu_online();

    // Find first ready thread bound to this CPU or unbound
    let mut found_thread: Option<usize> = None;
    for i in 1..MAX_THREADS {
        if s.threads[i].state == ThreadState::Ready {
            // Check if thread is bound to this CPU or unbound
            let thread_cpu = s.threads[i].cpu_affinity;
            if thread_cpu.is_none() || thread_cpu == Some(cpu_id) {
                found_thread = Some(i);
                break;
            }
        }
    }

    if let Some(i) = found_thread {
        let next_id = ThreadId(i);

        // Update states - get raw pointers first
        let current_id = s.current_thread;
        let current_tcb_ptr = s.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
        let next_tcb_ptr = s.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

        // Update states through raw pointers
        unsafe {
            (*current_tcb_ptr).state = ThreadState::Ready;
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = s.time_slice_ticks;
        }

        s.current_thread = next_id;

        // Jump to the first thread (don't save current context)
        // SAFETY: This is safe - we're jumping to a valid thread entry point
        unsafe {
            thread::start_context(next_tcb_ptr);
        }
    }

    // No ready thread found for this CPU, enter idle loop
    loop {
        thread::wait_for_interrupt();
    }
}

/// Sleep for a number of ticks
pub fn sleep_ticks(_ticks: u32) {
    let s = scheduler();
    if let Some(tcb) = s.get_thread_mut(s.current_thread) {
        tcb.state = ThreadState::Blocked;
    }
    schedule();
}
