//! Thread Management Module
//!
//! This module provides thread abstraction for the SMROS scheduler.
//! It defines Thread objects, Thread IDs, and CPU context structures.

use core::ptr;

/// Maximum number of concurrent threads
pub const MAX_THREADS: usize = 16;

/// Default thread stack size (8KB)
pub const DEFAULT_STACK_SIZE: usize = 0x2000;

/// Thread states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThreadState {
    Empty = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Terminated = 4,
}

impl ThreadState {
    /// Get string representation of thread state
    pub fn as_str(&self) -> &'static str {
        match self {
            ThreadState::Empty => "Empty     ",
            ThreadState::Ready => "Ready     ",
            ThreadState::Running => "Running   ",
            ThreadState::Blocked => "Blocked   ",
            ThreadState::Terminated => "Terminated",
        }
    }
}

/// Thread ID type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct ThreadId(pub usize);

impl ThreadId {
    pub const INVALID: ThreadId = ThreadId(usize::MAX);
    pub const IDLE: ThreadId = ThreadId(0);

    /// Get the numeric value of the thread ID
    pub fn as_usize(&self) -> usize {
        self.0
    }
}

/// ARM64 CPU context (registers saved during context switch)
/// This structure must match the layout expected by context_switch assembly
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    // General purpose registers X0-X28
    pub x0: u64,
    pub x1: u64,
    pub x2: u64,
    pub x3: u64,
    pub x4: u64,
    pub x5: u64,
    pub x6: u64,
    pub x7: u64,
    pub x8: u64,
    pub x9: u64,
    pub x10: u64,
    pub x11: u64,
    pub x12: u64,
    pub x13: u64,
    pub x14: u64,
    pub x15: u64,
    pub x16: u64,
    pub x17: u64,
    pub x18: u64,
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,

    // Frame Pointer (FP)
    pub fp: u64,

    // Link Register (return address)
    pub lr: u64,

    // Stack Pointer
    pub sp: u64,

    // Program Counter (entry point for new threads)
    pub pc: u64,

    // Processor State (PSTATE)
    pub pstate: u64,
}

impl CpuContext {
    /// Create a new CPU context for a thread
    pub fn new(entry: extern "C" fn() -> !, stack_top: u64) -> Self {
        CpuContext {
            x0: 0, x1: 0, x2: 0, x3: 0, x4: 0, x5: 0, x6: 0, x7: 0,
            x8: 0, x9: 0, x10: 0, x11: 0, x12: 0, x13: 0, x14: 0, x15: 0,
            x16: 0, x17: 0, x18: 0, x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0,
            lr: thread_exit_wrapper as u64,
            sp: stack_top,
            pc: entry as u64,
            pstate: 0x3C5, // EL1, interrupts enabled
        }
    }

    /// Create a default CPU context (for idle thread)
    pub const fn default_context() -> Self {
        CpuContext {
            x0: 0, x1: 0, x2: 0, x3: 0, x4: 0, x5: 0, x6: 0, x7: 0,
            x8: 0, x9: 0, x10: 0, x11: 0, x12: 0, x13: 0, x14: 0, x15: 0,
            x16: 0, x17: 0, x18: 0, x19: 0, x20: 0, x21: 0, x22: 0, x23: 0,
            x24: 0, x25: 0, x26: 0, x27: 0, x28: 0,
            fp: 0,
            lr: 0,
            sp: 0,
            pc: 0,
            pstate: 0,
        }
    }
}

/// Wrapper for raw pointers to implement Send/Sync
/// SAFETY: We ensure these pointers are only used in single-threaded contexts
/// or protected by proper synchronization.
pub struct SendPtr(pub *mut u8);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// Thread Control Block (TCB) - represents a thread object
#[repr(C)]
pub struct ThreadControlBlock {
    /// Thread ID
    pub id: ThreadId,

    /// Thread state
    pub state: ThreadState,

    /// CPU context for context switching
    pub context: CpuContext,

    /// Thread stack pointer
    pub stack: SendPtr,

    /// Stack size
    pub stack_size: usize,

    /// Thread entry point
    pub entry: Option<extern "C" fn() -> !>,

    /// Time slice remaining (in ticks)
    pub time_slice: u32,

    /// Total ticks run
    pub total_ticks: u32,

    /// Thread name (for debugging)
    pub name: &'static str,

    /// CPU affinity (which CPU this thread should run on)
    /// None = any CPU, Some(n) = specific CPU
    pub cpu_affinity: Option<usize>,

    /// Which CPU this thread is currently executing on
    pub current_cpu: Option<usize>,
}

impl ThreadControlBlock {
    /// Create a new TCB with default values
    pub const fn new() -> Self {
        ThreadControlBlock {
            id: ThreadId::INVALID,
            state: ThreadState::Empty,
            context: CpuContext::default_context(),
            stack: SendPtr(ptr::null_mut()),
            stack_size: 0,
            entry: None,
            time_slice: 0,
            total_ticks: 0,
            name: "",
            cpu_affinity: None,
            current_cpu: None,
        }
    }

    /// Initialize a thread with entry point and name
    pub fn init(
        &mut self,
        id: ThreadId,
        entry: extern "C" fn() -> !,
        name: &'static str,
        stack: *mut u8,
        stack_size: usize,
        time_slice: u32,
        cpu_affinity: Option<usize>,
    ) {
        self.id = id;
        self.state = ThreadState::Ready;
        self.entry = Some(entry);
        self.name = name;
        self.stack = SendPtr(stack);
        self.stack_size = stack_size;
        self.time_slice = time_slice;
        self.total_ticks = 0;
        self.cpu_affinity = cpu_affinity;
        self.current_cpu = cpu_affinity; // Initially scheduled on affinity CPU

        // Set up initial context
        let stack_top = (stack as u64) + (stack_size as u64);
        self.context = CpuContext::new(entry, stack_top);
    }

    /// Initialize the idle thread
    pub fn init_idle(&mut self, idle_entry: extern "C" fn() -> !, stack: *mut u8, stack_size: usize) {
        self.id = ThreadId::IDLE;
        self.state = ThreadState::Ready;
        self.entry = Some(idle_entry);
        self.name = "idle";
        self.stack = SendPtr(stack);
        self.stack_size = stack_size;
        self.time_slice = 10;
        self.total_ticks = 0;
        self.cpu_affinity = None;

        let stack_top = (stack as u64) + (stack_size as u64);
        self.context = CpuContext::new(idle_entry, stack_top);
    }

    /// Check if thread is in a runnable state
    pub fn is_runnable(&self) -> bool {
        self.state == ThreadState::Ready || self.state == ThreadState::Running
    }

    /// Check if thread is the idle thread
    pub fn is_idle(&self) -> bool {
        self.id == ThreadId::IDLE
    }

    /// Print thread information to serial
    pub fn print_info(&self, serial: &mut crate::serial::Serial) {
        print_number(serial, self.id.0 as u32);
        serial.write_str("   ");
        serial.write_str(self.state.as_str());
        serial.write_str("  ");
        serial.write_str(self.name);
        // Pad name to 12 characters
        for _ in 0..(12usize.saturating_sub(self.name.len())) {
            serial.write_byte(b' ');
        }
        
        // Print current CPU (where it's actually running)
        match self.current_cpu {
            Some(cpu) => {
                print_number(serial, cpu as u32);
            }
            None => {
                serial.write_str("*");
            }
        }
        serial.write_str("    ");
        
        print_number(serial, self.time_slice);
        serial.write_str("         ");
        print_number(serial, self.total_ticks);
        serial.write_str("\n");
    }
}

/// Thread exit wrapper - called when a thread returns
extern "C" fn thread_exit_wrapper() -> ! {
    loop {
        cortex_a::asm::wfi();
    }
}

/// Print a number to serial (helper function)
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

/// Thread stack allocator - safe wrapper around raw allocation
pub struct ThreadStack {
    ptr: *mut u8,
    size: usize,
}

impl ThreadStack {
    /// Allocate a new thread stack
    pub fn alloc(size: usize) -> Option<Self> {
        // SAFETY: size is DEFAULT_STACK_SIZE (8KB) which is valid and 16-byte aligned
        let layout = alloc::alloc::Layout::from_size_align(size, 16).ok()?;

        let ptr = unsafe { alloc::alloc::alloc(layout) };

        if ptr.is_null() {
            return None;
        }

        Some(ThreadStack { ptr, size })
    }

    /// Get the stack pointer
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Get the stack size
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for ThreadStack {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr was allocated with the same layout in alloc()
            let layout = alloc::alloc::Layout::from_size_align(self.size, 16)
                .expect("Invalid layout");
            unsafe {
                alloc::alloc::dealloc(self.ptr, layout);
            }
        }
    }
}
