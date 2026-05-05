#![allow(dead_code)]

//! Preemptive Round-Robin Scheduler
//!
//! This module implements a preemptive Round-Robin scheduler for SMROS.
//! It manages multiple threads and performs context switching on timer ticks.

use crate::kernel_lowlevel::thread::{
    self, SendPtr, ThreadControlBlock, ThreadId, ThreadStack, ThreadState, DEFAULT_STACK_SIZE,
    MAX_THREADS,
};
use crate::kernel_objects::object_logic;
use core::cell::UnsafeCell;
use core::ptr;

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

/// Maximum number of CPUs for thread binding
pub const MAX_CPUS: usize = 4;

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

impl Scheduler {
    /// Create a new scheduler instance
    pub const fn new() -> Self {
        Scheduler {
            threads: [const { ThreadControlBlock::new() }; MAX_THREADS],
            current_thread: ThreadId::INVALID,
            next_thread: 0,
            active_threads: 0,
            tick_count: 0,
            time_slice_ticks: 10, // 10 ticks per time slice (100ms at 100Hz)
            idle_stack: SendPtr(ptr::null_mut()),
        }
    }

    /// Initialize the scheduler
    pub fn init(&mut self) {
        // Initialize all TCBs as empty
        for i in 0..MAX_THREADS {
            self.threads[i].id = ThreadId(i);
            self.threads[i].state = ThreadState::Empty;
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
    }

    /// Create the idle thread
    fn create_idle_thread(&mut self) {
        let tcb = &mut self.threads[0];
        tcb.init_idle(idle_thread_entry, self.idle_stack.0, DEFAULT_STACK_SIZE);
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

                // Leak the stack (it will be freed when thread terminates)
                core::mem::forget(stack);

                self.active_threads += 1;

                return Some(ThreadId(i));
            }
        }

        None // No available slots
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

    /// Schedule the next thread (Round-Robin)
    pub fn schedule_next(&mut self) -> Option<ThreadId> {
        if self.active_threads <= 1 {
            return Some(ThreadId::IDLE);
        }

        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;

        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);

            // Skip the current thread and idle thread (unless it's the only option)
            if object_logic::scheduler_can_run(
                idx,
                current,
                self.threads[idx].state == ThreadState::Ready,
            ) {
                self.next_thread = (idx + 1) % MAX_THREADS;
                return Some(ThreadId(idx));
            }

            attempts += 1;
        }

        // No ready worker thread found, run idle
        Some(ThreadId::IDLE)
    }

    /// Schedule the next thread for a specific CPU (CPU-aware scheduling)
    pub fn schedule_next_for_cpu(&mut self, cpu_id: usize) -> Option<ThreadId> {
        if self.active_threads <= 1 {
            return Some(ThreadId::IDLE);
        }

        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;

        // Find a thread that is bound to this CPU (or unbound)
        while attempts < MAX_THREADS {
            let idx = object_logic::scheduler_candidate_index(start, attempts, MAX_THREADS);

            if object_logic::scheduler_can_run(
                idx,
                current,
                self.threads[idx].state == ThreadState::Ready,
            ) {
                let thread_cpu = self.threads[idx].cpu_affinity;
                // Only schedule if thread is bound to this CPU or unbound
                if object_logic::scheduler_cpu_allowed(
                    thread_cpu.is_some(),
                    thread_cpu.unwrap_or(0),
                    cpu_id,
                ) {
                    self.next_thread = (idx + 1) % MAX_THREADS;
                    return Some(ThreadId(idx));
                }
            }

            attempts += 1;
        }

        // No ready thread for this CPU found, run idle
        Some(ThreadId::IDLE)
    }

    /// Handle timer tick (called from interrupt handler)
    pub fn on_timer_tick(&mut self) {
        self.tick_count += 1;

        // Decrement current thread's time slice
        if let Some(tcb) = self.get_thread_mut(self.current_thread) {
            if tcb.time_slice > 0 {
                tcb.time_slice -= 1;
            }

            tcb.total_ticks += 1;
        }
    }

    /// Check if preemption is needed
    pub fn should_preempt(&self) -> bool {
        if let Some(tcb) = self.get_thread(self.current_thread) {
            object_logic::scheduler_should_preempt(tcb.time_slice, self.active_threads)
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
        serial.write_str("\nThread Table:\n");
        serial.write_str("ID  State      Name        CPU  TimeSlice  TotalTicks\n");

        for i in 0..MAX_THREADS {
            let tcb = &self.threads[i];
            if tcb.state != ThreadState::Empty {
                tcb.print_info(serial);
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
