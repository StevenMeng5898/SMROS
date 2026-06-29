#![allow(dead_code)]

//! Preemptive scheduler
//!
//! This module implements round-robin, EDF, credit, and fair scheduling for SMROS.
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
const DEFAULT_THREAD_PRIORITY: u8 = 16;
const DEFAULT_EDF_PERIOD_TICKS: u32 = 50;
const DEFAULT_CREDIT: i32 = 100;
const MAX_CREDIT_WEIGHT: u32 = (i32::MAX as u32) / (DEFAULT_CREDIT as u32);
pub const SCHED_TRACE_CAPACITY: usize = 128;
pub const SCHED_SAMPLE_MAX_WORKERS: usize = MAX_THREADS - 2;
const SCHED_SAMPLE_WORK_UNITS: u32 = 960;
const SCHED_SAMPLE_SPIN_UNITS: u32 = 80_000;
const SCHED_SAMPLE_TRACE_SNAPSHOT_ROUNDS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulePolicy {
    RoundRobin,
    Edf,
    Credit,
    Fair,
}

impl SchedulePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            SchedulePolicy::RoundRobin => "round-robin",
            SchedulePolicy::Edf => "edf",
            SchedulePolicy::Credit => "credit",
            SchedulePolicy::Fair => "fair",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        smros_sched_policy_from_match_flags_body!(
            value.eq_ignore_ascii_case("rr"),
            value.eq_ignore_ascii_case("round-robin"),
            value.eq_ignore_ascii_case("edf"),
            value.eq_ignore_ascii_case("credit"),
            value.eq_ignore_ascii_case("fair"),
            SchedulePolicy::RoundRobin,
            SchedulePolicy::Edf,
            SchedulePolicy::Credit,
            SchedulePolicy::Fair
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
    pub time_slice_ticks: u32,
    pub priority: u8,
    pub process_id: usize,
}

impl ThreadScheduleInfo {
    pub const fn empty() -> Self {
        Self {
            deadline_tick: u64::MAX,
            period_ticks: DEFAULT_EDF_PERIOD_TICKS,
            credit: 0,
            credit_cap: DEFAULT_CREDIT,
            weight: 1,
            time_slice_ticks: DEFAULT_TIME_SLICE_TICKS,
            priority: DEFAULT_THREAD_PRIORITY,
            process_id: 1,
        }
    }

    pub const fn idle() -> Self {
        Self {
            deadline_tick: u64::MAX,
            period_ticks: DEFAULT_EDF_PERIOD_TICKS,
            credit: 0,
            credit_cap: 0,
            weight: 0,
            time_slice_ticks: DEFAULT_TIME_SLICE_TICKS,
            priority: 0,
            process_id: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct ScheduleTestTask {
    id: usize,
    ready: bool,
    deadline_tick: u64,
    credit: i32,
    total_ticks: u32,
    weight: u32,
    priority: u8,
    cpu_affinity: Option<usize>,
}

#[derive(Clone, Copy)]
struct SchedulerSampleTraceWorker {
    cpu_id: usize,
    thread_id: usize,
    total_ticks: u32,
    schedule_info: ThreadScheduleInfo,
}

impl SchedulerSampleTraceWorker {
    const fn empty() -> Self {
        Self {
            cpu_id: 0,
            thread_id: 0,
            total_ticks: 0,
            schedule_info: ThreadScheduleInfo::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerPolicyTestResult {
    pub round_robin: usize,
    pub edf: usize,
    pub credit: usize,
    pub fair: usize,
    pub cpu_filtered: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerTraceEntry {
    pub tick: u64,
    pub cpu_id: usize,
    pub thread_id: usize,
}

impl SchedulerTraceEntry {
    pub const fn empty() -> Self {
        Self {
            tick: 0,
            cpu_id: 0,
            thread_id: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerSampleResult {
    pub requested: usize,
    pub created: usize,
    pub failed: usize,
    pub online_cpus: usize,
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

    /// Recent timer samples for shell-visible CPU time-slice tracing.
    trace_entries: [SchedulerTraceEntry; SCHED_TRACE_CAPACITY],

    /// Next trace entry slot.
    trace_next: usize,

    /// Number of valid trace entries.
    trace_len: usize,

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
    let best_priority = highest_ready_priority_from_tasks(tasks, cpu_id);
    for offset in 0..tasks.len() {
        let wanted_id = start_id.saturating_add(offset);
        for task in tasks {
            if task.id == wanted_id
                && task.ready
                && task.priority == best_priority
                && task_allowed_on_cpu(*task, cpu_id)
            {
                return Some(task.id);
            }
        }
    }
    for task in tasks {
        if task.ready && task.priority == best_priority && task_allowed_on_cpu(*task, cpu_id) {
            return Some(task.id);
        }
    }
    None
}

fn highest_ready_priority_from_tasks(tasks: &[ScheduleTestTask], cpu_id: Option<usize>) -> u8 {
    let mut best_priority = 0u8;
    let mut found = false;
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && smros_sched_priority_better_body!(task.priority, found, best_priority)
        {
            best_priority = task.priority;
            found = true;
        }
    }
    best_priority
}

fn pick_edf_from_tasks(tasks: &[ScheduleTestTask], cpu_id: Option<usize>) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_deadline = u64::MAX;
    let best_priority = highest_ready_priority_from_tasks(tasks, cpu_id);
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && task.priority == best_priority
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
    let best_priority = highest_ready_priority_from_tasks(tasks, cpu_id);
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && task.priority == best_priority
            && smros_sched_credit_better_body!(task.credit, best.is_some(), best_credit)
        {
            best = Some(task.id);
            best_credit = task.credit;
        }
    }
    best
}

fn pick_fair_from_tasks(tasks: &[ScheduleTestTask], cpu_id: Option<usize>) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_ticks = 0u32;
    let mut best_weight = 1u32;
    let best_priority = highest_ready_priority_from_tasks(tasks, cpu_id);
    for task in tasks {
        if task.ready
            && task_allowed_on_cpu(*task, cpu_id)
            && task.priority == best_priority
            && smros_sched_fair_better_body!(
                task.total_ticks,
                task.weight,
                best.is_some(),
                best_ticks,
                best_weight
            )
        {
            best = Some(task.id);
            best_ticks = task.total_ticks;
            best_weight = task.weight;
        }
    }
    best
}

extern "C" fn sched_sample_worker() -> ! {
    let slot = current_sample_slot();
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.write_str("[sched-sample] worker ");
    print_number(&mut serial, slot as u32);
    serial.write_str(" start\n");

    let mut burst = 0u32;
    while burst < SCHED_SAMPLE_WORK_UNITS {
        let mut spin = 0u32;
        while spin < SCHED_SAMPLE_SPIN_UNITS {
            core::hint::spin_loop();
            spin += 1;
        }
        let online_cpus =
            core::cmp::max(crate::kernel_lowlevel::smp::online_cpu_count() as usize, 1);
        let next_cpu = (slot + 1) % online_cpus;
        yield_now_on_cpu(next_cpu);
        burst += 1;
    }

    serial.write_str("[sched-sample] worker ");
    print_number(&mut serial, slot as u32);
    serial.write_str(" done\n");
    scheduler().finish_current_without_stack_free();
    schedule();
    loop {
        thread::wait_for_interrupt();
    }
}

fn current_sample_slot() -> usize {
    let s = scheduler();
    let current = s.current_thread.0;
    let mut slot = 0usize;
    for idx in 1..MAX_THREADS {
        if s.threads[idx].name == "sched_sample" && s.threads[idx].state != ThreadState::Empty {
            if idx == current {
                return slot;
            }
            slot += 1;
        }
    }
    0
}

fn priority_from_weight(weight: u32) -> u8 {
    if weight == 0 {
        1
    } else if weight > u8::MAX as u32 {
        u8::MAX
    } else {
        weight as u8
    }
}

fn sample_priority_from_weight(weight: u32) -> u8 {
    DEFAULT_THREAD_PRIORITY.saturating_add(priority_from_weight(weight))
}

fn contains_usize(values: &[usize], value: usize) -> bool {
    for item in values {
        if *item == value {
            return true;
        }
    }
    false
}

fn sort_usize_prefix(values: &mut [usize], len: usize) {
    let capped_len = core::cmp::min(len, values.len());
    let mut index = 1usize;
    while index < capped_len {
        let value = values[index];
        let mut insert = index;
        while insert > 0 && values[insert - 1] > value {
            values[insert] = values[insert - 1];
            insert -= 1;
        }
        values[insert] = value;
        index += 1;
    }
}

fn count_workers_on_cpu(workers: &[SchedulerSampleTraceWorker], cpu_id: usize) -> usize {
    let mut count = 0usize;
    for worker in workers {
        if worker.cpu_id == cpu_id {
            count += 1;
        }
    }
    count
}

fn sample_trace_worker_after(
    lhs: SchedulerSampleTraceWorker,
    rhs: SchedulerSampleTraceWorker,
) -> bool {
    if lhs.cpu_id != rhs.cpu_id {
        return lhs.cpu_id > rhs.cpu_id;
    }
    if lhs.schedule_info.priority != rhs.schedule_info.priority {
        return lhs.schedule_info.priority < rhs.schedule_info.priority;
    }
    lhs.thread_id > rhs.thread_id
}

fn sort_sample_trace_workers(workers: &mut [SchedulerSampleTraceWorker; MAX_THREADS], len: usize) {
    let capped_len = core::cmp::min(len, workers.len());
    let mut index = 1usize;
    while index < capped_len {
        let value = workers[index];
        let mut insert = index;
        while insert > 0 && sample_trace_worker_after(workers[insert - 1], value) {
            workers[insert] = workers[insert - 1];
            insert -= 1;
        }
        workers[insert] = value;
        index += 1;
    }
}

fn sample_trace_worker_slice_ticks(worker: SchedulerSampleTraceWorker) -> u64 {
    u64::from(core::cmp::max(worker.schedule_info.time_slice_ticks, 1))
}

fn highest_sample_trace_priority(
    workers: &[SchedulerSampleTraceWorker],
    cpu_id: usize,
    excluded: &[bool; MAX_THREADS],
) -> u8 {
    let mut best_priority = 0u8;
    let mut found = false;
    for (index, worker) in workers.iter().enumerate() {
        if !excluded[index]
            && worker.cpu_id == cpu_id
            && smros_sched_priority_better_body!(
                worker.schedule_info.priority,
                found,
                best_priority
            )
        {
            best_priority = worker.schedule_info.priority;
            found = true;
        }
    }
    best_priority
}

fn sample_trace_snapshot_rounds(worker_count: usize) -> usize {
    let worker_count = core::cmp::max(worker_count, 1);
    let rounds_that_fit = SCHED_TRACE_CAPACITY
        .saturating_div(worker_count)
        .saturating_sub(1);
    core::cmp::max(
        1,
        core::cmp::min(SCHED_SAMPLE_TRACE_SNAPSHOT_ROUNDS, rounds_that_fit),
    )
}

fn pick_sample_trace_worker(
    workers: &mut [SchedulerSampleTraceWorker],
    policy: SchedulePolicy,
    cpu_id: usize,
    rr_cursor_by_cpu: &mut [usize; MAX_CPUS],
    excluded: &[bool; MAX_THREADS],
) -> Option<usize> {
    let selected = match policy {
        SchedulePolicy::RoundRobin => {
            pick_sample_trace_round_robin(workers, cpu_id, rr_cursor_by_cpu, excluded)
        }
        SchedulePolicy::Edf => pick_sample_trace_edf(workers, cpu_id, excluded),
        SchedulePolicy::Credit => pick_sample_trace_credit(workers, cpu_id, excluded),
        SchedulePolicy::Fair => pick_sample_trace_fair(workers, cpu_id, excluded),
    };

    if let Some(index) = selected {
        let run_ticks = sample_trace_worker_slice_ticks(workers[index]);
        update_sample_trace_worker_after_pick(&mut workers[index], policy, run_ticks);
    }
    selected
}

fn pick_sample_trace_round_robin(
    workers: &[SchedulerSampleTraceWorker],
    cpu_id: usize,
    rr_cursor_by_cpu: &mut [usize; MAX_CPUS],
    excluded: &[bool; MAX_THREADS],
) -> Option<usize> {
    if workers.is_empty() {
        return None;
    }
    let cursor_slot = core::cmp::min(cpu_id, MAX_CPUS.saturating_sub(1));
    let start = rr_cursor_by_cpu[cursor_slot] % workers.len();
    let best_priority = highest_sample_trace_priority(workers, cpu_id, excluded);
    let mut offset = 0usize;
    while offset < workers.len() {
        let index = (start + offset) % workers.len();
        if !excluded[index]
            && workers[index].cpu_id == cpu_id
            && workers[index].schedule_info.priority == best_priority
        {
            rr_cursor_by_cpu[cursor_slot] = (index + 1) % workers.len();
            return Some(index);
        }
        offset += 1;
    }
    None
}

fn pick_sample_trace_edf(
    workers: &[SchedulerSampleTraceWorker],
    cpu_id: usize,
    excluded: &[bool; MAX_THREADS],
) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_deadline = u64::MAX;
    let best_priority = highest_sample_trace_priority(workers, cpu_id, excluded);
    let mut index = 0usize;
    while index < workers.len() {
        let worker = workers[index];
        if !excluded[index]
            && worker.cpu_id == cpu_id
            && worker.schedule_info.priority == best_priority
            && smros_sched_edf_better_body!(
                worker.schedule_info.deadline_tick,
                best.is_some(),
                best_deadline
            )
        {
            best = Some(index);
            best_deadline = worker.schedule_info.deadline_tick;
        }
        index += 1;
    }
    best
}

fn pick_sample_trace_credit(
    workers: &mut [SchedulerSampleTraceWorker],
    cpu_id: usize,
    excluded: &[bool; MAX_THREADS],
) -> Option<usize> {
    if !sample_trace_has_credit(workers, cpu_id) {
        refill_sample_trace_credits(workers, cpu_id);
    }

    let mut best: Option<usize> = None;
    let mut best_credit = i32::MIN;
    let best_priority = highest_sample_trace_priority(workers, cpu_id, excluded);
    let mut index = 0usize;
    while index < workers.len() {
        let worker = workers[index];
        if !excluded[index]
            && worker.cpu_id == cpu_id
            && worker.schedule_info.priority == best_priority
            && smros_sched_credit_better_body!(
                worker.schedule_info.credit,
                best.is_some(),
                best_credit
            )
        {
            best = Some(index);
            best_credit = worker.schedule_info.credit;
        }
        index += 1;
    }
    best
}

fn pick_sample_trace_fair(
    workers: &[SchedulerSampleTraceWorker],
    cpu_id: usize,
    excluded: &[bool; MAX_THREADS],
) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_ticks = 0u32;
    let mut best_weight = 1u32;
    let best_priority = highest_sample_trace_priority(workers, cpu_id, excluded);
    let mut index = 0usize;
    while index < workers.len() {
        let worker = workers[index];
        if !excluded[index]
            && worker.cpu_id == cpu_id
            && worker.schedule_info.priority == best_priority
            && smros_sched_fair_better_body!(
                worker.total_ticks,
                worker.schedule_info.weight,
                best.is_some(),
                best_ticks,
                best_weight
            )
        {
            best = Some(index);
            best_ticks = worker.total_ticks;
            best_weight = worker.schedule_info.weight;
        }
        index += 1;
    }
    best
}

fn sample_trace_has_credit(workers: &[SchedulerSampleTraceWorker], cpu_id: usize) -> bool {
    for worker in workers {
        if worker.cpu_id == cpu_id && worker.schedule_info.credit > 0 {
            return true;
        }
    }
    false
}

fn refill_sample_trace_credits(workers: &mut [SchedulerSampleTraceWorker], cpu_id: usize) {
    for worker in workers {
        if worker.cpu_id == cpu_id {
            worker.schedule_info.credit = smros_sched_refill_credit_body!(
                worker.schedule_info.credit_cap,
                worker.schedule_info.weight,
                DEFAULT_CREDIT,
                MAX_CREDIT_WEIGHT
            );
        }
    }
}

fn update_sample_trace_worker_after_pick(
    worker: &mut SchedulerSampleTraceWorker,
    policy: SchedulePolicy,
    run_ticks: u64,
) {
    let charged_ticks = if run_ticks > u32::MAX as u64 {
        u32::MAX
    } else {
        run_ticks as u32
    };
    worker.total_ticks = worker.total_ticks.saturating_add(charged_ticks);
    match policy {
        SchedulePolicy::Edf => {
            worker.schedule_info.deadline_tick = smros_sched_advance_deadline_body!(
                worker.schedule_info.deadline_tick,
                worker.schedule_info.deadline_tick,
                worker.schedule_info.period_ticks
            );
        }
        SchedulePolicy::Credit => {
            let mut ticks = charged_ticks;
            while ticks > 0 {
                worker.schedule_info.credit =
                    smros_sched_credit_after_tick_body!(worker.schedule_info.credit);
                ticks -= 1;
            }
        }
        SchedulePolicy::RoundRobin | SchedulePolicy::Fair => {}
    }
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
            trace_entries: [SchedulerTraceEntry::empty(); SCHED_TRACE_CAPACITY],
            trace_next: 0,
            trace_len: 0,
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
        self.trace_entries = [SchedulerTraceEntry::empty(); SCHED_TRACE_CAPACITY];
        self.trace_next = 0;
        self.trace_len = 0;
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
            time_slice_ticks: self.time_slice_ticks,
            priority: DEFAULT_THREAD_PRIORITY,
            process_id: 1,
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

    pub fn set_thread_credit_value(&mut self, id: ThreadId, credit: i32) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS || credit < 0 {
            return false;
        }
        if self.threads[id.0].state == ThreadState::Empty {
            return false;
        }
        self.schedule_info[id.0].credit = credit;
        self.schedule_info[id.0].credit_cap = credit;
        true
    }

    pub fn thread_schedule_info(&self, id: ThreadId) -> Option<ThreadScheduleInfo> {
        if id.0 < MAX_THREADS {
            Some(self.schedule_info[id.0])
        } else {
            None
        }
    }

    pub fn set_thread_priority(&mut self, id: ThreadId, priority: u8) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS || priority == 0 {
            return false;
        }
        if self.threads[id.0].state == ThreadState::Empty {
            return false;
        }
        self.schedule_info[id.0].priority = priority;
        if self.threads[id.0].state == ThreadState::Running {
            self.threads[id.0].time_slice = 0;
        }
        true
    }

    pub fn set_thread_cpu_affinity(&mut self, id: ThreadId, cpu_id: Option<usize>) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS {
            return false;
        }
        if self.threads[id.0].state == ThreadState::Empty {
            return false;
        }
        if let Some(cpu) = cpu_id {
            let online_cpus =
                core::cmp::max(crate::kernel_lowlevel::smp::online_cpu_count() as usize, 1);
            if cpu >= online_cpus || cpu >= MAX_CPUS {
                return false;
            }
        }

        let thread = &mut self.threads[id.0];
        thread.cpu_affinity = cpu_id;
        if thread.state != ThreadState::Running {
            thread.current_cpu = cpu_id;
        } else if let Some(cpu) = cpu_id {
            if thread.current_cpu != Some(cpu) {
                thread.time_slice = 0;
            }
        }
        true
    }

    pub fn bind_thread_process(&mut self, id: ThreadId, process_id: usize) -> bool {
        if id.0 >= MAX_THREADS || self.threads[id.0].state == ThreadState::Empty {
            return false;
        }
        self.schedule_info[id.0].process_id = process_id;
        true
    }

    pub fn live_thread_count_for_process(&self, process_id: usize) -> usize {
        let mut count = 0usize;
        for idx in 1..MAX_THREADS {
            let thread = &self.threads[idx];
            if thread.state != ThreadState::Empty
                && thread.state != ThreadState::Terminated
                && self.schedule_info[idx].process_id == process_id
            {
                count += 1;
            }
        }
        count
    }

    pub fn set_thread_time_slice(&mut self, id: ThreadId, ticks: u32) -> bool {
        if id.0 == 0 || id.0 >= MAX_THREADS || ticks == 0 {
            return false;
        }
        if self.threads[id.0].state == ThreadState::Empty {
            return false;
        }
        self.schedule_info[id.0].time_slice_ticks = ticks;
        if id == self.current_thread {
            if let Some(tcb) = self.get_thread_mut(id) {
                tcb.time_slice = core::cmp::min(tcb.time_slice, ticks);
                if tcb.time_slice == 0 {
                    tcb.time_slice = ticks;
                }
            }
        }
        true
    }

    pub fn thread_time_slice_ticks(&self, id: ThreadId) -> Option<u32> {
        if id.0 < MAX_THREADS && self.threads[id.0].state != ThreadState::Empty {
            Some(self.schedule_info[id.0].time_slice_ticks)
        } else {
            None
        }
    }

    pub fn start_sample_workers(&mut self, requested: usize) -> SchedulerSampleResult {
        self.reap_terminated_threads();
        self.clear_trace();
        let online_cpus =
            core::cmp::max(crate::kernel_lowlevel::smp::online_cpu_count() as usize, 1);
        let open_slots = self.available_thread_slots();
        let requested_workers = requested.clamp(1, SCHED_SAMPLE_MAX_WORKERS);
        let wanted = requested_workers.min(open_slots);
        let mut created = 0usize;

        for slot in 0..wanted {
            let cpu = slot % online_cpus;
            if let Some(id) =
                self.create_thread_on_cpu(sched_sample_worker, "sched_sample", Some(cpu))
            {
                let weight = 1 + ((slot / online_cpus) as u32 % 4);
                let priority = sample_priority_from_weight(weight);
                let cap = DEFAULT_CREDIT.saturating_mul(weight as i32);
                let _ = self.set_thread_credit(id, cap, cap, weight);
                let _ = self.set_thread_priority(id, priority);
                let deadline = self
                    .tick_count
                    .saturating_add(DEFAULT_EDF_PERIOD_TICKS as u64)
                    .saturating_sub(u64::from(priority))
                    .saturating_add(slot as u64);
                let _ = self.set_thread_deadline(id, deadline, DEFAULT_EDF_PERIOD_TICKS);
                self.push_trace_entry(cpu, id.0);
                created += 1;
            }
        }

        SchedulerSampleResult {
            requested: requested_workers,
            created,
            failed: requested_workers.saturating_sub(created),
            online_cpus,
        }
    }

    fn available_thread_slots(&self) -> usize {
        let mut slots = 0usize;
        for idx in 1..MAX_THREADS {
            if self.threads[idx].state == ThreadState::Empty
                || self.threads[idx].state == ThreadState::Terminated
            {
                slots += 1;
            }
        }
        slots
    }

    fn reap_terminated_threads(&mut self) {
        for idx in 1..MAX_THREADS {
            if self.threads[idx].state == ThreadState::Terminated {
                self.threads[idx] = ThreadControlBlock::new();
                self.threads[idx].id = ThreadId(idx);
                self.schedule_info[idx] = ThreadScheduleInfo::empty();
            }
        }
    }

    fn clear_trace(&mut self) {
        self.trace_entries = [SchedulerTraceEntry::empty(); SCHED_TRACE_CAPACITY];
        self.trace_next = 0;
        self.trace_len = 0;
    }

    fn push_trace_entry(&mut self, cpu_id: usize, thread_id: usize) {
        self.push_trace_entry_at_tick(self.tick_count, cpu_id, thread_id);
    }

    fn push_trace_entry_at_tick(&mut self, tick: u64, cpu_id: usize, thread_id: usize) {
        if SCHED_TRACE_CAPACITY == 0 {
            return;
        }
        self.trace_entries[self.trace_next] = SchedulerTraceEntry {
            tick,
            cpu_id,
            thread_id,
        };
        self.trace_next = (self.trace_next + 1) % SCHED_TRACE_CAPACITY;
        if self.trace_len < SCHED_TRACE_CAPACITY {
            self.trace_len += 1;
        }
    }

    pub fn record_sample_worker_snapshot(&mut self) -> usize {
        let mut workers = [SchedulerSampleTraceWorker::empty(); MAX_THREADS];
        let mut worker_count = 0usize;
        for idx in 1..MAX_THREADS {
            let worker = {
                let thread = &self.threads[idx];
                if thread.name == "sched_sample" && thread.state != ThreadState::Empty {
                    Some(SchedulerSampleTraceWorker {
                        cpu_id: thread.current_cpu.or(thread.cpu_affinity).unwrap_or(0),
                        thread_id: idx,
                        total_ticks: thread.total_ticks,
                        schedule_info: self.schedule_info[idx],
                    })
                } else {
                    None
                }
            };
            if let Some(worker) = worker {
                workers[worker_count] = worker;
                worker_count += 1;
            }
        }
        if worker_count == 0 {
            return 0;
        }
        sort_sample_trace_workers(&mut workers, worker_count);

        let mut cpu_rows = [usize::MAX; MAX_CPUS];
        let mut cpu_count = 0usize;
        for worker in workers[..worker_count].iter().copied() {
            if !contains_usize(&cpu_rows[..cpu_count], worker.cpu_id) && cpu_count < cpu_rows.len()
            {
                cpu_rows[cpu_count] = worker.cpu_id;
                cpu_count += 1;
            }
        }
        sort_usize_prefix(&mut cpu_rows, cpu_count);

        let mut preview_workers = workers;
        let mut preview_offsets = [0u64; MAX_CPUS];
        let mut preview_rr_cursor_by_cpu = [0usize; MAX_CPUS];
        let snapshot_rounds = sample_trace_snapshot_rounds(worker_count);
        for worker in preview_workers[..worker_count].iter().copied() {
            let cpu_slot = core::cmp::min(worker.cpu_id, MAX_CPUS.saturating_sub(1));
            preview_offsets[cpu_slot] =
                preview_offsets[cpu_slot].saturating_add(sample_trace_worker_slice_ticks(worker));
        }
        for _round in 0..snapshot_rounds {
            for cpu in cpu_rows[..cpu_count].iter().copied() {
                let cpu_slot = core::cmp::min(cpu, MAX_CPUS.saturating_sub(1));
                let workers_on_cpu = count_workers_on_cpu(&preview_workers[..worker_count], cpu);
                let mut picked_in_round = [false; MAX_THREADS];
                for _ in 0..workers_on_cpu {
                    if let Some(worker_index) = pick_sample_trace_worker(
                        &mut preview_workers[..worker_count],
                        self.policy,
                        cpu,
                        &mut preview_rr_cursor_by_cpu,
                        &picked_in_round,
                    ) {
                        picked_in_round[worker_index] = true;
                        let worker = preview_workers[worker_index];
                        preview_offsets[cpu_slot] = preview_offsets[cpu_slot]
                            .saturating_add(sample_trace_worker_slice_ticks(worker));
                    }
                }
            }
        }

        let mut total_ticks = 1u64;
        for offset in preview_offsets {
            total_ticks = core::cmp::max(total_ticks, offset);
        }
        let base_tick = self.tick_count.saturating_sub(total_ticks);
        self.clear_trace();

        let mut recorded = 0usize;
        let mut cpu_trace_offsets = [0u64; MAX_CPUS];
        for worker in workers[..worker_count].iter().copied() {
            let cpu_slot = core::cmp::min(worker.cpu_id, MAX_CPUS.saturating_sub(1));
            self.push_trace_entry_at_tick(
                base_tick.saturating_add(cpu_trace_offsets[cpu_slot]),
                worker.cpu_id,
                worker.thread_id,
            );
            cpu_trace_offsets[cpu_slot] =
                cpu_trace_offsets[cpu_slot].saturating_add(sample_trace_worker_slice_ticks(worker));
            recorded += 1;
        }

        let mut rr_cursor_by_cpu = [0usize; MAX_CPUS];
        for _round in 0..snapshot_rounds {
            for cpu in cpu_rows[..cpu_count].iter().copied() {
                let cpu_slot = core::cmp::min(cpu, MAX_CPUS.saturating_sub(1));
                let workers_on_cpu = count_workers_on_cpu(&workers[..worker_count], cpu);
                let mut picked_in_round = [false; MAX_THREADS];
                for _ in 0..workers_on_cpu {
                    if let Some(worker_index) = pick_sample_trace_worker(
                        &mut workers[..worker_count],
                        self.policy,
                        cpu,
                        &mut rr_cursor_by_cpu,
                        &picked_in_round,
                    ) {
                        picked_in_round[worker_index] = true;
                        let worker = workers[worker_index];
                        self.push_trace_entry_at_tick(
                            base_tick.saturating_add(cpu_trace_offsets[cpu_slot]),
                            cpu,
                            worker.thread_id,
                        );
                        cpu_trace_offsets[cpu_slot] = cpu_trace_offsets[cpu_slot]
                            .saturating_add(sample_trace_worker_slice_ticks(worker));
                        recorded += 1;
                    }
                }
            }
        }
        recorded
    }

    pub fn record_trace_sample(&mut self, cpu_id: usize) {
        if self.trace_len != 0 {
            let last_idx = if self.trace_next == 0 {
                SCHED_TRACE_CAPACITY - 1
            } else {
                self.trace_next - 1
            };
            let last = self.trace_entries[last_idx];
            let min_tick_delta = self
                .thread_time_slice_ticks(self.current_thread)
                .unwrap_or(self.time_slice_ticks)
                .max(1) as u64;
            if last.cpu_id == cpu_id
                && last.thread_id == self.current_thread.0
                && self.tick_count.saturating_sub(last.tick) < min_tick_delta
            {
                return;
            }
        }

        self.push_trace_entry(cpu_id, self.current_thread.0);
    }

    pub fn record_trace_switch(&mut self, cpu_id: usize) {
        self.push_trace_entry(cpu_id, self.current_thread.0);
    }

    pub fn trace_len(&self) -> usize {
        self.trace_len
    }

    pub fn trace_entry(&self, age_index: usize) -> Option<SchedulerTraceEntry> {
        if age_index >= self.trace_len || SCHED_TRACE_CAPACITY == 0 {
            return None;
        }
        let oldest = if self.trace_len == SCHED_TRACE_CAPACITY {
            self.trace_next
        } else {
            0
        };
        let idx = (oldest + age_index) % SCHED_TRACE_CAPACITY;
        Some(self.trace_entries[idx])
    }

    pub fn trace_entry_is_sample_worker(&self, entry: SchedulerTraceEntry) -> bool {
        if entry.thread_id >= MAX_THREADS {
            return false;
        }
        self.threads[entry.thread_id].name == "sched_sample"
    }

    pub fn trace_entry_is_task(&self, entry: SchedulerTraceEntry) -> bool {
        if entry.thread_id == ThreadId::IDLE.0 || entry.thread_id >= MAX_THREADS {
            return false;
        }
        let thread = &self.threads[entry.thread_id];
        thread.state != ThreadState::Empty && !thread.name.is_empty()
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
            SchedulePolicy::Fair => self.pick_fair(cpu_id),
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

    fn highest_ready_priority(&self, cpu_id: Option<usize>) -> Option<u8> {
        let current = self.current_thread.0;
        let mut best_priority = 0u8;
        let mut found = false;
        for idx in 1..MAX_THREADS {
            if self.candidate_can_run(idx, current, cpu_id)
                && smros_sched_priority_better_body!(
                    self.schedule_info[idx].priority,
                    found,
                    best_priority
                )
            {
                best_priority = self.schedule_info[idx].priority;
                found = true;
            }
        }
        if found {
            Some(best_priority)
        } else {
            None
        }
    }

    fn ready_higher_priority_exists(&self, cpu_id: Option<usize>) -> bool {
        if self.current_thread.0 == ThreadId::IDLE.0 {
            return false;
        }
        let Some(best_ready_priority) = self.highest_ready_priority(cpu_id) else {
            return false;
        };
        let current_priority = self.schedule_info[self.current_thread.0].priority;
        smros_sched_priority_should_preempt_body!(current_priority, true, best_ready_priority)
    }

    fn pick_round_robin(&mut self, cpu_id: Option<usize>) -> Option<usize> {
        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;
        let best_priority = self.highest_ready_priority(cpu_id)?;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);

            // Skip the current thread and idle thread (unless it's the only option)
            if self.candidate_can_run(idx, current, cpu_id)
                && self.schedule_info[idx].priority == best_priority
            {
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
        let best_priority = self.highest_ready_priority(cpu_id)?;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);
            if self.candidate_can_run(idx, current, cpu_id)
                && self.schedule_info[idx].priority == best_priority
            {
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
        let best_priority = self.highest_ready_priority(cpu_id)?;
        if !self.any_ready_credit(cpu_id, best_priority) {
            self.refill_credits();
        }

        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;
        let mut best: Option<usize> = None;
        let mut best_credit = i32::MIN;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);
            if self.candidate_can_run(idx, current, cpu_id)
                && self.schedule_info[idx].priority == best_priority
            {
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

    fn pick_fair(&mut self, cpu_id: Option<usize>) -> Option<usize> {
        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;
        let mut best: Option<usize> = None;
        let mut best_ticks = 0u32;
        let mut best_weight = 1u32;
        let best_priority = self.highest_ready_priority(cpu_id)?;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);
            if self.candidate_can_run(idx, current, cpu_id)
                && self.schedule_info[idx].priority == best_priority
            {
                let ticks = self.threads[idx].total_ticks;
                let weight = self.schedule_info[idx].weight;
                if smros_sched_fair_better_body!(
                    ticks,
                    weight,
                    best.is_some(),
                    best_ticks,
                    best_weight
                ) {
                    best = Some(idx);
                    best_ticks = ticks;
                    best_weight = weight;
                }
            }
            attempts += 1;
        }

        if let Some(idx) = best {
            self.next_thread = (idx + 1) % MAX_THREADS;
        }
        best
    }

    fn any_ready_credit(&self, cpu_id: Option<usize>, priority: u8) -> bool {
        let current = self.current_thread.0;
        for idx in 1..MAX_THREADS {
            if self.candidate_can_run(idx, current, cpu_id)
                && self.schedule_info[idx].priority == priority
                && self.schedule_info[idx].credit > 0
            {
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
                total_ticks: 18,
                weight: 1,
                priority: 1,
                cpu_affinity: None,
            },
            ScheduleTestTask {
                id: 2,
                ready: true,
                deadline_tick: 40,
                credit: 10,
                total_ticks: 20,
                weight: 5,
                priority: 2,
                cpu_affinity: Some(1),
            },
            ScheduleTestTask {
                id: 3,
                ready: true,
                deadline_tick: 70,
                credit: 80,
                total_ticks: 12,
                weight: 1,
                priority: 2,
                cpu_affinity: None,
            },
            ScheduleTestTask {
                id: 4,
                ready: false,
                deadline_tick: 10,
                credit: 200,
                total_ticks: 0,
                weight: 1,
                priority: 3,
                cpu_affinity: None,
            },
        ];

        SchedulerPolicyTestResult {
            round_robin: pick_round_robin_from_tasks(&tasks, 2, None).unwrap_or(0),
            edf: pick_edf_from_tasks(&tasks, None).unwrap_or(0),
            credit: pick_credit_from_tasks(&tasks, None).unwrap_or(0),
            fair: pick_fair_from_tasks(&tasks, None).unwrap_or(0),
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

            tcb.total_ticks = tcb.total_ticks.saturating_add(1);
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
                SchedulePolicy::Fair => {}
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
            if self.ready_higher_priority_exists(tcb.current_cpu) {
                return true;
            }
            if tcb.time_slice == 0
                && self.schedule_info[self.current_thread.0].priority
                    > self.highest_ready_priority(tcb.current_cpu).unwrap_or(0)
            {
                return false;
            }
            let info = self.schedule_info[self.current_thread.0];
            smros_sched_should_preempt_body!(
                self.policy,
                SchedulePolicy::RoundRobin,
                SchedulePolicy::Edf,
                SchedulePolicy::Credit,
                SchedulePolicy::Fair,
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
        let time_slice = self
            .thread_time_slice_ticks(id)
            .unwrap_or(self.time_slice_ticks);
        if let Some(tcb) = self.get_thread_mut(id) {
            tcb.time_slice = time_slice;
        }
    }

    fn charge_current_runtime(&mut self, units: u32) {
        if self.current_thread.0 == ThreadId::IDLE.0 {
            return;
        }
        if let Some(tcb) = self.get_thread_mut(self.current_thread) {
            tcb.total_ticks = tcb.total_ticks.saturating_add(units);
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
                serial.write_str(" slice=");
                print_number(serial, self.schedule_info[i].time_slice_ticks);
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
    let cpu_id = current_logical_cpu(s);

    // Find next thread to run
    if let Some(next_id) = s.schedule_next_for_cpu(cpu_id) {
        let current_id = s.current_thread;

        if next_id == current_id {
            // No need to switch
            return;
        }

        // Update states - get raw pointers first to avoid borrow issues
        let current_tcb_ptr = s.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
        let next_tcb_ptr = s.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

        let next_time_slice = s
            .thread_time_slice_ticks(next_id)
            .unwrap_or(s.time_slice_ticks);

        // Update states through raw pointers
        unsafe {
            if (*current_tcb_ptr).state == ThreadState::Running {
                (*current_tcb_ptr).state = ThreadState::Ready;
            }
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = next_time_slice;
            (*next_tcb_ptr).current_cpu = Some(cpu_id);
        }

        s.current_thread = next_id;
        s.record_trace_switch(cpu_id);

        // Perform context switch
        // SAFETY: These pointers are valid TCB references
        unsafe {
            thread::switch_context(current_tcb_ptr, next_tcb_ptr);
        }
    }
}

fn current_logical_cpu(s: &Scheduler) -> usize {
    let current_cpu = s
        .get_thread(s.current_thread)
        .and_then(|thread| thread.current_cpu)
        .unwrap_or_else(|| crate::kernel_lowlevel::smp::current_cpu_id() as usize);
    let online_cpus = core::cmp::max(crate::kernel_lowlevel::smp::online_cpu_count() as usize, 1);
    let max_cpu = core::cmp::min(online_cpus, MAX_CPUS).saturating_sub(1);
    core::cmp::min(current_cpu, max_cpu)
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

        let next_time_slice = s
            .thread_time_slice_ticks(next_id)
            .unwrap_or(s.time_slice_ticks);

        // Update states through raw pointers
        unsafe {
            (*current_tcb_ptr).state = ThreadState::Ready;
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = next_time_slice;
            (*next_tcb_ptr).current_cpu = Some(0);
        }

        s.current_thread = next_id;
        s.record_trace_switch(0);

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
    s.charge_current_runtime(1);
    if let Some(tcb) = s.get_thread_mut(s.current_thread) {
        tcb.time_slice = 0;
    }
    schedule();
}

/// Yield the current thread's time slice on a specific CPU
pub fn yield_now_on_cpu(cpu_id: usize) {
    let s = scheduler();
    s.charge_current_runtime(1);
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

        let next_time_slice = s
            .thread_time_slice_ticks(next_id)
            .unwrap_or(s.time_slice_ticks);

        // Update states through raw pointers
        unsafe {
            if (*current_tcb_ptr).state == ThreadState::Running {
                (*current_tcb_ptr).state = ThreadState::Ready;
            }
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = next_time_slice;
            // Mark which logical CPU this thread is running on
            (*next_tcb_ptr).current_cpu = Some(cpu_id);
        }

        s.current_thread = next_id;
        s.record_trace_switch(cpu_id);

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

        let next_time_slice = s
            .thread_time_slice_ticks(next_id)
            .unwrap_or(s.time_slice_ticks);

        // Update states through raw pointers
        unsafe {
            (*current_tcb_ptr).state = ThreadState::Ready;
            (*next_tcb_ptr).state = ThreadState::Running;
            (*next_tcb_ptr).time_slice = next_time_slice;
            (*next_tcb_ptr).current_cpu = Some(cpu_id);
        }

        s.current_thread = next_id;
        s.record_trace_switch(cpu_id);

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
