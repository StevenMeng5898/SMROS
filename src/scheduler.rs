//! Preemptive Round-Robin Scheduler
//!
//! This module implements a preemptive Round-Robin scheduler for SMROS.
//! It manages multiple threads and performs context switching on timer ticks.

use crate::thread::{
    ThreadControlBlock, ThreadId, ThreadState, MAX_THREADS, DEFAULT_STACK_SIZE, ThreadStack,
};
use core::ptr;

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
    idle_stack: *mut u8,
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
            idle_stack: ptr::null_mut(),
        }
    }

    /// Initialize the scheduler
    pub fn init(&mut self) {
        // Initialize all TCBs as empty
        for i in 0..MAX_THREADS {
            self.threads[i].id = ThreadId(i);
            self.threads[i].state = ThreadState::Empty;
        }

        // Allocate idle thread stack
        static mut IDLE_STACK: [u8; DEFAULT_STACK_SIZE] = [0; DEFAULT_STACK_SIZE];
        unsafe {
            self.idle_stack = IDLE_STACK.as_mut_ptr();
        }

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
        tcb.init_idle(
            idle_thread_entry,
            self.idle_stack,
            DEFAULT_STACK_SIZE,
        );
    }

    /// Create a new thread
    pub fn create_thread(
        &mut self,
        entry: extern "C" fn() -> !,
        name: &'static str,
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
    pub fn schedule(&mut self) -> Option<ThreadId> {
        if self.active_threads <= 1 {
            return Some(ThreadId::IDLE);
        }

        let start = self.next_thread;
        let mut attempts = 0;
        let current = self.current_thread.0;

        while attempts < MAX_THREADS {
            let idx = (start + attempts) % MAX_THREADS;

            // Skip the current thread and idle thread (unless it's the only option)
            if idx != current
                && idx != 0
                && self.threads[idx].state == ThreadState::Ready
            {
                self.next_thread = (idx + 1) % MAX_THREADS;
                return Some(ThreadId(idx));
            }

            attempts += 1;
        }

        // No ready worker thread found, run idle
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
            tcb.time_slice == 0 && self.active_threads > 1
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
            (tcb.stack, tcb.stack_size, tcb.id.0)
        } else {
            (ptr::null_mut(), 0, 0)
        };

        self.active_threads -= 1;

        // Free stack (only for non-idle threads)
        if !stack_info.0.is_null() && stack_info.2 != 0 {
            let layout = unsafe {
                alloc::alloc::Layout::from_size_align_unchecked(stack_info.1, 16)
            };
            unsafe {
                alloc::alloc::dealloc(stack_info.0, layout);
            }
        }
    }

    /// Get tick count
    pub fn get_tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Print scheduler status
    pub fn print_status(&self, serial: &mut crate::serial::Serial) {
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
        serial.write_str("ID  State      Name        TimeSlice  TotalTicks\n");

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
        cortex_a::asm::wfi();
    }
}

/// Global scheduler instance
static mut SCHEDULER: Scheduler = Scheduler::new();

/// Get a reference to the global scheduler
pub fn get_scheduler() -> &'static mut Scheduler {
    unsafe { &mut SCHEDULER }
}

/// Perform a context switch to the next thread
pub fn schedule() {
    unsafe {
        let scheduler = &mut SCHEDULER;

        // Find next thread to run
        if let Some(next_id) = scheduler.schedule() {
            let current_id = scheduler.current_thread;

            if next_id == current_id {
                // No need to switch
                return;
            }

            // Update states
            if let Some(current_tcb) = scheduler.get_thread_mut(current_id) {
                if current_tcb.state == ThreadState::Running {
                    current_tcb.state = ThreadState::Ready;
                }
            }

            let time_slice = scheduler.time_slice_ticks;
            if let Some(next_tcb) = scheduler.get_thread_mut(next_id) {
                next_tcb.state = ThreadState::Running;
                next_tcb.time_slice = time_slice;
            }

            scheduler.current_thread = next_id;

            // Perform context switch
            let current_ctx =
                scheduler.get_thread_mut(current_id).unwrap() as *mut ThreadControlBlock;
            let next_ctx =
                scheduler.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;

            context_switch(current_ctx, next_ctx);
        }
    }
}

/// Start the first user thread (called from kernel_main)
/// This function never returns - it jumps to the first thread
pub fn start_first_thread() -> ! {
    unsafe {
        let scheduler = &mut SCHEDULER;

        // Find first ready thread
        let mut found_thread: Option<usize> = None;
        for i in 1..MAX_THREADS {
            if scheduler.threads[i].state == ThreadState::Ready {
                found_thread = Some(i);
                break;
            }
        }

        if let Some(i) = found_thread {
            let next_id = ThreadId(i);

            // Update states
            let current_id = scheduler.current_thread;
            if let Some(current_tcb) = scheduler.get_thread_mut(current_id) {
                current_tcb.state = ThreadState::Ready;
            }

            let time_slice = scheduler.time_slice_ticks;
            if let Some(next_tcb) = scheduler.get_thread_mut(next_id) {
                next_tcb.state = ThreadState::Running;
                next_tcb.time_slice = time_slice;
            }

            scheduler.current_thread = next_id;

            // Jump to the first thread (don't save current context)
            let next_ctx =
                scheduler.get_thread_mut(next_id).unwrap() as *mut ThreadControlBlock;
            context_switch_start(next_ctx);
        }

        // No ready thread found, just halt
        loop {
            cortex_a::asm::wfi();
        }
    }
}

// External assembly function for context switching (defined in main.rs)
extern "C" {
    fn context_switch(current: *mut ThreadControlBlock, next: *mut ThreadControlBlock);
    fn context_switch_start(next: *mut ThreadControlBlock) -> !;
}

/// Helper function to print a number
fn print_number(serial: &mut crate::serial::Serial, mut num: u32) {
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
    unsafe {
        // Just reset the time slice to force a context switch
        let scheduler = &mut SCHEDULER;
        if let Some(tcb) = scheduler.get_thread_mut(scheduler.current_thread) {
            tcb.time_slice = 0; // Force preemption
        }
        schedule();
    }
}

/// Sleep for a number of ticks
pub fn sleep_ticks(_ticks: u32) {
    unsafe {
        let scheduler = &mut SCHEDULER;
        if let Some(tcb) = scheduler.get_thread_mut(scheduler.current_thread) {
            tcb.state = ThreadState::Blocked;
        }
        schedule();
    }
}
