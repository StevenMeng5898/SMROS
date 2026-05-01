#![allow(dead_code)]
#![allow(static_mut_refs)]
//! EL0 Process Management
//!
//! This module manages processes that run at EL0 (user mode).
//! It handles:
//! - Process creation with proper address space setup
//! - Privilege level transitions (EL1 → EL0)
//! - User-mode stack and memory setup
//! - Process context for exception handling

use crate::kernel_lowlevel::memory::{
    process_manager, PageFrameAllocator, ProcessControlBlock, PAGE_SIZE,
};
use crate::kernel_lowlevel::mmu::PageTableManager;
use crate::kernel_objects::thread::{ThreadControlBlock, ThreadId, DEFAULT_STACK_SIZE};
use crate::user_level::user_logic;

/// User process control block - extends PCB with EL0-specific data
#[repr(C)]
pub struct UserProcess {
    /// Base process control block
    pub pcb: ProcessControlBlock,
    /// Page table manager for this process
    pub page_table: Option<PageTableManager>,
    /// User stack virtual address
    pub user_stack_vaddr: usize,
    /// User stack size
    pub user_stack_size: usize,
    /// Entry point (user-mode code)
    pub entry_point: usize,
    /// Process handle (for Zircon compatibility)
    pub proc_handle: u32,
    /// VMAR handle (for Zircon compatibility)
    pub vmar_handle: u32,
    /// Whether this process is initialized
    pub initialized: bool,
}

impl UserProcess {
    /// Create a new user process
    pub fn new(
        pid: usize,
        parent_pid: usize,
        name: &'static str,
        entry_point: usize,
    ) -> Option<Self> {
        let mut pcb = ProcessControlBlock::new();
        if !pcb.init(pid, parent_pid, name) {
            return None;
        }

        // Create page table manager for this process
        let page_table = PageTableManager::new()?;

        Some(Self {
            pcb,
            page_table: Some(page_table),
            user_stack_vaddr: 0,
            user_stack_size: 0,
            entry_point,
            proc_handle: 0,
            vmar_handle: 0,
            initialized: false,
        })
    }

    /// Initialize user process with proper memory layout
    pub fn init_user_process(&mut self) -> bool {
        if self.initialized {
            return false;
        }

        // Setup user-space memory layout
        // Code segment at 0x0000_0000_0000_0000
        // Data segment at 0x0000_0000_0001_0000
        // Heap at 0x0000_0000_0002_0000
        // Stack at 0x0000_0000_FFFF_0000

        let pt = match &mut self.page_table {
            Some(pt) => pt,
            None => return false,
        };

        // Map code segment (read-execute, user-accessible)
        let code_pfn = match PageFrameAllocator::alloc() {
            Some(pfn) => pfn,
            None => return false,
        };
        let code_paddr = match user_logic::pfn_to_paddr(code_pfn, PAGE_SIZE) {
            Some(paddr) => paddr,
            None => return false,
        };
        let code_vaddr = user_logic::USER_CODE_VADDR;
        if !pt.map_user_region(
            code_vaddr, code_paddr, PAGE_SIZE, true,  // readable
            false, // not writable
            true,  // executable
        ) {
            return false;
        }

        // Map data segment (read-write, user-accessible)
        let data_pfn = match PageFrameAllocator::alloc() {
            Some(pfn) => pfn,
            None => return false,
        };
        let data_paddr = match user_logic::pfn_to_paddr(data_pfn, PAGE_SIZE) {
            Some(paddr) => paddr,
            None => return false,
        };
        let data_vaddr = user_logic::USER_DATA_VADDR;
        if !pt.map_user_region(
            data_vaddr, data_paddr, PAGE_SIZE, true,  // readable
            true,  // writable
            false, // not executable
        ) {
            return false;
        }

        // Map heap (read-write, user-accessible, multiple pages)
        let heap_vaddr = user_logic::USER_HEAP_VADDR;
        let heap_pages = user_logic::USER_HEAP_PAGES;
        for i in 0..heap_pages {
            let heap_pfn = match PageFrameAllocator::alloc() {
                Some(pfn) => pfn,
                None => return false,
            };
            let page_vaddr = match user_logic::page_offset_vaddr(heap_vaddr, i, PAGE_SIZE) {
                Some(vaddr) => vaddr,
                None => return false,
            };
            let heap_paddr = match user_logic::pfn_to_paddr(heap_pfn, PAGE_SIZE) {
                Some(paddr) => paddr,
                None => return false,
            };
            if !pt.map_user_region(
                page_vaddr, heap_paddr, PAGE_SIZE, true,  // readable
                true,  // writable
                false, // not executable
            ) {
                return false;
            }
        }

        // Map user stack (read-write, user-accessible, grows downward)
        let stack_vaddr = user_logic::USER_STACK_VADDR;
        let stack_pages = user_logic::USER_STACK_PAGES;
        for i in 0..stack_pages {
            let stack_pfn = match PageFrameAllocator::alloc() {
                Some(pfn) => pfn,
                None => return false,
            };
            let page_vaddr = match user_logic::page_offset_vaddr(stack_vaddr, i, PAGE_SIZE) {
                Some(vaddr) => vaddr,
                None => return false,
            };
            let stack_paddr = match user_logic::pfn_to_paddr(stack_pfn, PAGE_SIZE) {
                Some(paddr) => paddr,
                None => return false,
            };
            if !pt.map_user_region(
                page_vaddr,
                stack_paddr,
                PAGE_SIZE,
                true,  // readable
                true,  // writable
                false, // not executable
            ) {
                return false;
            }
        }

        self.user_stack_vaddr =
            match user_logic::page_offset_vaddr(stack_vaddr, stack_pages, PAGE_SIZE) {
                Some(vaddr) => vaddr,
                None => return false,
            };
        self.user_stack_size = match user_logic::page_offset_vaddr(0, stack_pages, PAGE_SIZE) {
            Some(size) => size,
            None => return false,
        };

        // Initialize PCB address space
        self.pcb.address_space.init(self.pcb.pid);

        self.initialized = true;
        true
    }

    /// Create a thread for this user process
    pub fn create_user_thread(&self, thread_id: ThreadId) -> Option<ThreadControlBlock> {
        let mut tcb = ThreadControlBlock::new();

        // Allocate stack for the thread
        let layout = alloc::alloc::Layout::from_size_align(DEFAULT_STACK_SIZE, 16).ok()?;

        let stack = unsafe { alloc::alloc::alloc(layout) };

        if stack.is_null() {
            return None;
        }

        let stack_top = user_logic::stack_top_u64(stack as u64, DEFAULT_STACK_SIZE)?;

        // Initialize thread context for EL0 execution
        tcb.init(
            thread_id,
            // Entry point is the user process entry
            unsafe { core::mem::transmute(self.entry_point as usize) },
            self.pcb.name,
            stack,
            DEFAULT_STACK_SIZE,
            user_logic::USER_THREAD_TIME_SLICE, // time slice
            None,                               // no CPU affinity
        );

        // Modify context for EL0 execution
        tcb.context.sp = stack_top;
        tcb.context.pc = self.entry_point as u64;

        // Set PSTATE for EL0 with interrupts enabled
        // M[1:0] = 0b00 (EL0t), D=1, A=1, I=1, F=1
        tcb.context.pstate = user_logic::el0_thread_pstate(); // EL0t, all interrupts masked

        Some(tcb)
    }
}

/// Global user process table
static mut USER_PROCESSES: [Option<UserProcess>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

/// Create a new user process
pub fn create_user_process(name: &'static str, entry_point: extern "C" fn() -> !) -> Option<usize> {
    let pm = process_manager();

    // Create PCB first
    let pid = pm.create_process(name)?;

    // Find empty slot in user process table
    unsafe {
        for i in 0..USER_PROCESSES.len() {
            if USER_PROCESSES[i].is_none() {
                let mut user_proc = UserProcess::new(
                    pid,
                    user_logic::USER_INIT_PARENT_PID, // parent is init
                    name,
                    entry_point as usize,
                )?;

                if user_proc.init_user_process() {
                    USER_PROCESSES[i] = Some(user_proc);
                    return Some(pid);
                }
            }
        }
    }

    None
}

/// Get user process by PID
pub fn get_user_process(pid: usize) -> Option<&'static UserProcess> {
    unsafe {
        for proc in USER_PROCESSES.iter() {
            if let Some(p) = proc {
                if p.pcb.pid == pid {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Get mutable user process by PID
pub fn get_user_process_mut(pid: usize) -> Option<&'static mut UserProcess> {
    unsafe {
        for proc in USER_PROCESSES.iter_mut() {
            if let Some(p) = proc {
                if p.pcb.pid == pid {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Switch to user mode (EL0)
///
/// This function prepares the CPU state for switching to EL0 and executes
/// the user process entry point.
///
/// # Safety
/// This function performs low-level CPU operations and should only be called
/// from kernel code with proper setup.
#[no_mangle]
pub unsafe extern "C" fn switch_to_el0(entry_point: u64, user_stack: u64, ttbr0: u64) -> ! {
    // Set TTBR0 for user space page tables
    core::arch::asm!(
        "msr ttbr0_el1, {ttbr0}",
        "tlbi vmalle1is",
        "dsb ish",
        "isb",
        ttbr0 = in(reg) ttbr0,
        options(nostack),
    );

    // Setup SP_EL0 for user stack
    core::arch::asm!(
        "msr sp_el0, {sp}",
        sp = in(reg) user_stack,
        options(nostack),
    );

    // Setup ELR_EL1 for return address (user entry point)
    core::arch::asm!(
        "msr elr_el1, {entry}",
        entry = in(reg) entry_point,
        options(nostack),
    );

    // Setup SPSR_EL1 for EL0t mode with interrupts enabled
    // M[1:0] = 0b00 (EL0t), D=0, A=0, I=0, F=0 (enable interrupts)
    let spsr: u64 = user_logic::el0_spsr(); // EL0t, all interrupts enabled

    core::arch::asm!(
        "msr spsr_el1, {spsr}",
        spsr = in(reg) spsr,
        options(nostack),
    );

    // Setup HCR_EL2 to enable EL1 and EL0 (if running at EL2)
    // HCR_RW = 1 (AArch64 at EL1)
    // HCR_VM = 1 (Enable stage 2 translation)
    // These would be set by EL2 code before dropping to EL1

    // Return to EL0 using ERET
    core::arch::asm!("eret", options(noreturn),);
}

/// Initialize EL0 process management
pub fn init() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("[EL0] User process manager initialized\n");
}
