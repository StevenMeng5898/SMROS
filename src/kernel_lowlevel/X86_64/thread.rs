#![allow(dead_code)]

use core::ptr;

use super::lowlevel_logic;

pub const MAX_THREADS: usize = 128;
pub const DEFAULT_STACK_SIZE: usize = 0x8000;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct ThreadId(pub usize);

impl ThreadId {
    pub const INVALID: ThreadId = ThreadId(usize::MAX);
    pub const IDLE: ThreadId = ThreadId(0);

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl CpuContext {
    pub fn new(entry: extern "C" fn() -> !, stack_top: u64) -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            rsp: stack_top & !0xf,
            rip: entry as *const () as u64,
            rflags: 0x202,
        }
    }

    pub fn set_entry_stack(&mut self, entry: u64, stack_top: u64) {
        self.rip = entry;
        self.rsp = stack_top & !0xf;
    }

    pub fn set_user_state(&mut self, state: u64) {
        self.rflags = state;
    }

    pub const fn default_context() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            rsp: 0,
            rip: 0,
            rflags: 0,
        }
    }
}

#[repr(transparent)]
pub struct SendPtr(pub *mut u8);

unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

#[repr(C)]
pub struct ThreadControlBlock {
    pub id: ThreadId,
    pub state: ThreadState,
    pub context: CpuContext,
    pub stack: SendPtr,
    pub stack_size: usize,
    pub entry: Option<extern "C" fn() -> !>,
    pub time_slice: u32,
    pub total_ticks: u32,
    pub name: &'static str,
    pub cpu_affinity: Option<usize>,
    pub current_cpu: Option<usize>,
}

impl ThreadControlBlock {
    pub const fn new() -> Self {
        Self {
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
        self.current_cpu = cpu_affinity;
        self.context = CpuContext::new(entry, (stack as u64) + (stack_size as u64));
    }

    pub fn init_idle(
        &mut self,
        idle_entry: extern "C" fn() -> !,
        stack: *mut u8,
        stack_size: usize,
    ) {
        self.id = ThreadId::IDLE;
        self.state = ThreadState::Ready;
        self.entry = Some(idle_entry);
        self.name = "idle";
        self.stack = SendPtr(stack);
        self.stack_size = stack_size;
        self.time_slice = 10;
        self.total_ticks = 0;
        self.cpu_affinity = None;
        self.context = CpuContext::new(idle_entry, (stack as u64) + (stack_size as u64));
    }

    pub fn is_runnable(&self) -> bool {
        thread_state_is_runnable(self.state)
    }

    pub fn is_idle(&self) -> bool {
        thread_id_is_idle(self.id)
    }

    pub fn print_info(&self, serial: &mut crate::kernel_lowlevel::serial::Serial) {
        print_number(serial, self.id.0 as u32);
        serial.write_str("   ");
        serial.write_str(self.state.as_str());
        serial.write_str("  ");
        serial.write_str(self.name);
        for _ in 0..(12usize.saturating_sub(self.name.len())) {
            serial.write_byte(b' ');
        }
        match self.current_cpu {
            Some(cpu) => print_number(serial, cpu as u32),
            None => serial.write_str("*"),
        }
        serial.write_str("    ");
        print_number(serial, self.time_slice);
        serial.write_str("         ");
        print_number(serial, self.total_ticks);
        serial.write_str("\n");
    }
}

pub fn thread_state_is_runnable(state: ThreadState) -> bool {
    lowlevel_logic::thread_state_runnable(state, ThreadState::Ready, ThreadState::Running)
}

pub fn thread_id_is_idle(id: ThreadId) -> bool {
    lowlevel_logic::thread_id_idle(id.0, ThreadId::IDLE.0)
}

#[inline(always)]
pub fn wait_for_interrupt() {
    crate::kernel_lowlevel::cpu::wait_for_interrupt();
}

#[allow(improper_ctypes)]
extern "C" {
    fn context_switch(current: *mut ThreadControlBlock, next: *mut ThreadControlBlock);
    fn context_switch_start(next: *mut ThreadControlBlock) -> !;
}

pub unsafe fn switch_context(current: *mut ThreadControlBlock, next: *mut ThreadControlBlock) {
    context_switch(current, next);
}

pub unsafe fn start_context(next: *mut ThreadControlBlock) -> ! {
    context_switch_start(next)
}

fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut len = 0;
    while num > 0 && len < 10 {
        buf[len] = b'0' + (num % 10) as u8;
        num /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        serial.write_byte(buf[len]);
    }
}

pub struct ThreadStack {
    ptr: *mut u8,
    size: usize,
}

impl ThreadStack {
    pub fn alloc(size: usize) -> Option<Self> {
        let layout = alloc::alloc::Layout::from_size_align(size, 16).ok()?;
        let ptr = unsafe { alloc::alloc::alloc(layout) };
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr, size })
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for ThreadStack {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            if let Ok(layout) = alloc::alloc::Layout::from_size_align(self.size, 16) {
                unsafe {
                    alloc::alloc::dealloc(self.ptr, layout);
                }
            }
        }
    }
}
