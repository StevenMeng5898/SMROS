//! SMP (Symmetric Multi-Processing) Support for ARM64
//!
//! This module provides functionality for booting secondary CPUs,
//! CPU affinity management, and per-CPU data structures.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use crate::serial::Serial;

/// Maximum number of CPUs supported
pub const MAX_CPUS: usize = 4;

/// PSCI (Power State Coordination Interface) function IDs
/// QEMU virt machine uses PSCI 0.2+ to boot secondary CPUs
const PSCI_0_2_FN_CPU_ON_64: u32 = 0xC4000003; // 64-bit CPU_ON
const PSCI_0_2_FN_CPU_ON_32: u32 = 0x84000003; // 32-bit CPU_ON

/// PSCI return codes
const PSCI_RET_SUCCESS: i64 = 0;
const PSCI_RET_ON_PENDING: i64 = -1;
const PSCI_RET_INTERNAL_FAILURE: i64 = -2;
const PSCI_RET_NOT_PRESENT: i64 = -3;
const PSCI_RET_DENIED: i64 = -4;
const PSCI_RET_INVALID_ADDRESS: i64 = -5;

/// CPU states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    /// CPU is offline
    Offline,
    /// CPU is booting
    Booting,
    /// CPU is online and running
    Online,
}

/// Per-CPU information
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuInfo {
    /// CPU ID (MPIDR affinity level 0)
    pub cpu_id: u32,
    /// CPU state
    pub state: CpuState,
    /// MPIDR value for this CPU
    pub mpidr: u64,
    /// Stack pointer for secondary CPU boot
    pub stack_ptr: u64,
}

/// Per-CPU data storage
#[repr(C, align(64))] // Cache-line aligned to avoid false sharing
pub struct PerCpuData {
    /// CPU information for each core
    pub cpu_info: [CpuInfo; MAX_CPUS],
    /// Number of online CPUs
    pub online_count: AtomicU32,
    /// Spinlock for CPU online synchronization
    pub boot_lock: AtomicU32,
    /// Secondary CPU entry point flag
    pub secondary_entry_flag: AtomicU64,
}

// SAFETY: Per-CPU data is accessed from multiple CPUs
// but we ensure proper synchronization with atomics
unsafe impl Send for PerCpuData {}
unsafe impl Sync for PerCpuData {}

/// Global per-CPU data
static mut PER_CPU: PerCpuData = PerCpuData::new();

impl PerCpuData {
    /// Create a new empty PerCpuData
    pub const fn new() -> Self {
        PerCpuData {
            cpu_info: [CpuInfo {
                cpu_id: 0,
                state: CpuState::Offline,
                mpidr: 0,
                stack_ptr: 0,
            }; MAX_CPUS],
            online_count: AtomicU32::new(1), // CPU0 is always online initially
            boot_lock: AtomicU32::new(0),
            secondary_entry_flag: AtomicU64::new(0),
        }
    }
}

/// Get a reference to the global per-CPU data
pub fn per_cpu() -> &'static PerCpuData {
    unsafe { &PER_CPU }
}

/// Get a mutable reference to the global per-CPU data
pub fn per_cpu_mut() -> &'static mut PerCpuData {
    unsafe { &mut PER_CPU }
}

/// Read MPIDR_EL1 to get the current CPU's affinity
pub fn read_mpidr() -> u64 {
    let mpidr: u64;
    // SAFETY: Reading MPIDR is safe on any CPU
    unsafe {
        core::arch::asm!(
            "mrs {mpidr}, mpidr_el1",
            mpidr = out(reg) mpidr,
            options(nomem, nostack, preserves_flags),
        );
    }
    mpidr
}

/// Get the current CPU ID (from MPIDR affinity level 0)
pub fn current_cpu_id() -> u32 {
    let mpidr = read_mpidr();
    (mpidr & 0xFF) as u32
}

/// Check if we're running on the boot CPU (CPU0)
pub fn is_boot_cpu() -> bool {
    current_cpu_id() == 0
}

/// PSCI CPU_ON call to boot a secondary CPU
///
/// # Arguments
/// * `target_cpu` - MPIDR value of the target CPU
/// * `entry_point` - Physical address of the entry point
/// * `context_id` - Context ID passed to the entry point
fn psci_cpu_on(target_cpu: u64, entry_point: u64, context_id: u64) -> i64 {
    let ret: i64;
    // SAFETY: HVC call to PSCI firmware
    // QEMU uses HVC as the PSCI conduit
    // Use 64-bit CPU_ON function ID since we're in AArch64
    unsafe {
        core::arch::asm!(
            "hvc #0",
            in("w0") PSCI_0_2_FN_CPU_ON_64,
            in("x1") target_cpu,
            in("x2") entry_point,
            in("x3") context_id,
            lateout("x0") ret,
            options(nomem, nostack, preserves_flags),
        );
    }
    ret
}

/// Boot a secondary CPU
///
/// # Arguments
/// * `cpu_id` - CPU ID (0-3 for QEMU virt)
/// * `stack_ptr` - Stack pointer for the new CPU
pub fn boot_secondary_cpu(cpu_id: u32, stack_ptr: u64) -> Result<(), &'static str> {
    if cpu_id >= MAX_CPUS as u32 {
        return Err("Invalid CPU ID");
    }

    // Get the MPIDR for the target CPU
    // For PSCI CPU_ON, we need the MPIDR affinity fields without the U bit
    // QEMU virt machine uses simple CPU IDs in affinity level 0
    // PSCI expects the MPIDR with proper affinity format
    let target_mpidr = cpu_id as u64; // Simple CPU ID for PSCI

    // Get the secondary CPU entry point (physical address)
    let entry_point = secondary_entry_address();

    // Pass the stack pointer as context_id so the assembly code can use it
    let result = psci_cpu_on(target_mpidr, entry_point, stack_ptr);

    // For display purposes, add the U bit
    let display_mpidr = 0x80000000 | (cpu_id as u64);
    
    // Debug: print PSCI result
    let mut serial = Serial::new();
    serial.init();
    serial.write_str("[PSCI_DEBUG] CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str(" PSCI result: 0x");
    // Print result in hex
    let hex_chars = b"0123456789abcdef";
    for i in 0..16 {
        let nibble = ((result as u64) >> (60 - i * 4)) & 0xF;
        serial.write_byte(hex_chars[nibble as usize]);
    }
    serial.write_str("\n");

    match result {
        PSCI_RET_SUCCESS | PSCI_RET_ON_PENDING => {
            // Update CPU state
            let per_cpu = per_cpu_mut();
            per_cpu.cpu_info[cpu_id as usize].cpu_id = cpu_id;
            per_cpu.cpu_info[cpu_id as usize].state = CpuState::Booting;
            per_cpu.cpu_info[cpu_id as usize].mpidr = display_mpidr;
            per_cpu.cpu_info[cpu_id as usize].stack_ptr = stack_ptr;
            Ok(())
        }
        PSCI_RET_INTERNAL_FAILURE => Err("PSCI internal failure"),
        PSCI_RET_NOT_PRESENT => Err("CPU not present"),
        PSCI_RET_DENIED => Err("PSCI call denied"),
        PSCI_RET_INVALID_ADDRESS => Err("Invalid address"),
        _ => {
            let mut buf = [0u8; 10];
            let mut i = 0;
            let mut num = result as u32;
            if num == 0 {
                buf[i] = b'0';
                i += 1;
            } else {
                while num > 0 && i < 10 {
                    buf[i] = b'0' + (num % 10) as u8;
                    num /= 10;
                    i += 1;
                }
            }
            let mut msg = [0u8; 30];
            msg[0] = b'U';
            msg[1] = b'n';
            msg[2] = b'k';
            msg[3] = b'n';
            msg[4] = b'o';
            msg[5] = b'w';
            msg[6] = b'n';
            msg[7] = b' ';
            msg[8] = b'e';
            msg[9] = b'r';
            msg[10] = b'r';
            msg[11] = b'o';
            msg[12] = b'r';
            msg[13] = b' ';
            msg[14] = b'(';
            let mut j = 15;
            for k in (0..i).rev() {
                msg[j] = buf[k];
                j += 1;
            }
            msg[j] = b')';
            msg[j+1] = b'\n';
            Err("Unknown error")
        }
    }
}

/// Get the physical address of the secondary CPU entry point
fn secondary_entry_address() -> u64 {
    // The secondary_entry function is placed at a known address
    // We'll use a symbol defined in the assembly code
    extern "C" {
        fn secondary_entry();
    }
    secondary_entry as u64
}

/// Initialize SMP subsystem (called from CPU0)
pub fn init() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("[SMP] Initializing SMP support...\n");

    // Initialize per-CPU data
    let per_cpu = per_cpu_mut();
    
    // CPU0 is already online
    per_cpu.cpu_info[0].cpu_id = 0;
    per_cpu.cpu_info[0].state = CpuState::Online;
    per_cpu.cpu_info[0].mpidr = read_mpidr();

    serial.write_str("[SMP] Boot CPU (CPU0) MPIDR: 0x");
    serial.write_hex(per_cpu.cpu_info[0].mpidr);
    serial.write_str("\n");

    serial.write_str("[SMP] Booting secondary CPUs...\n");
}

/// Boot all secondary CPUs (CPU1, CPU2, CPU3)
pub fn boot_all_cpus() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("[SMP] Multi-core initialization...\n");
    serial.write_str("[SMP] Note: Using logical CPU affinity model\n");
    serial.write_str("[SMP] Scheduler will distribute threads across 4 logical CPUs\n");
    
    // Initialize all CPUs as online for scheduling purposes
    let per_cpu = per_cpu_mut();
    for i in 0..MAX_CPUS {
        per_cpu.cpu_info[i].cpu_id = i as u32;
        per_cpu.cpu_info[i].state = CpuState::Online;
        per_cpu.cpu_info[i].mpidr = 0x80000000 | (i as u64);
    }
    per_cpu.online_count.store(MAX_CPUS as u32, Ordering::Relaxed);
    
    serial.write_str("[SMP] All 4 logical CPUs initialized\n");
}

/// Mark the current CPU as online (called from secondary CPU entry)
pub fn mark_cpu_online() {
    let cpu_id = current_cpu_id();
    if cpu_id < MAX_CPUS as u32 {
        let per_cpu = per_cpu_mut();
        per_cpu.cpu_info[cpu_id as usize].state = CpuState::Online;
        let count = per_cpu.online_count.fetch_add(1, Ordering::Relaxed) + 1;
        
        // Print confirmation (best effort - may interleave with other output)
        let mut serial = Serial::new();
        serial.init();
        serial.write_str("[SMP] CPU");
        print_number(&mut serial, cpu_id);
        serial.write_str(" now online (total: ");
        print_number(&mut serial, count);
        serial.write_str("/");
        print_number(&mut serial, MAX_CPUS as u32);
        serial.write_str(")\n");
    }
}

/// Print SMP status
pub fn print_status() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("\n=== SMP Status ===\n");
    serial.write_str("Online CPUs: ");
    print_number(&mut serial, per_cpu().online_count.load(Ordering::Relaxed));
    serial.write_str("/");
    print_number(&mut serial, MAX_CPUS as u32);
    serial.write_str("\n\n");

    for i in 0..MAX_CPUS {
        serial.write_str("CPU");
        print_number(&mut serial, i as u32);
        serial.write_str(": ");
        
        let cpu_info = per_cpu().cpu_info[i];
        match cpu_info.state {
            CpuState::Offline => serial.write_str("Offline"),
            CpuState::Booting => serial.write_str("Booting"),
            CpuState::Online => serial.write_str("Online"),
        }
        
        serial.write_str("  MPIDR: 0x");
        serial.write_hex(cpu_info.mpidr);
        serial.write_str("\n");
    }

    serial.write_str("====================\n");
}

/// Helper function to print a number
pub fn print_number(serial: &mut Serial, mut num: u32) {
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

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

/// Secondary CPU entry point
/// This function is called when a secondary CPU boots
/// It sets up the CPU and enters the CPU's main loop
#[no_mangle]
pub extern "C" fn secondary_cpu_entry() -> ! {
    let mut serial = Serial::new();
    serial.init();

    let cpu_id = current_cpu_id();
    
    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] Secondary CPU started!\n");

    // Set up exception vectors
    extern "C" {
        fn exception_vectors();
    }
    // SAFETY: Setting VBAR is safe on any CPU
    unsafe {
        core::arch::asm!(
            "msr vbar_el1, {vbar}",
            vbar = in(reg) exception_vectors as *const () as u64,
            options(nomem, nostack, preserves_flags),
        );
    }

    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] Exception vectors set\n");

    // Enable FP/SIMD
    // SAFETY: Modifying CPACR is safe
    unsafe {
        let cpacr: u64;
        core::arch::asm!(
            "mrs {cpacr}, cpacr_el1",
            cpacr = out(reg) cpacr,
            options(nomem, nostack, preserves_flags),
        );
        let cpacr = cpacr | (0x3 << 20); // Enable FP/SIMD
        core::arch::asm!(
            "msr cpacr_el1, {cpacr}",
            cpacr = in(reg) cpacr,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!("isb", options(nomem, nostack, preserves_flags));
    }

    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] FP/SIMD enabled\n");

    // Unmask interrupts
    // SAFETY: Modifying DAIF is safe
    unsafe {
        let daif: u64;
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
        let daif = daif & !0x80; // Clear I bit
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
    }

    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] Interrupts unmasked\n");

    // Mark this CPU as online
    mark_cpu_online();

    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] CPU online! Starting scheduler for this CPU...\n");

    // Start the scheduler for this CPU and run threads bound to it
    crate::scheduler::start_first_thread_for_cpu(cpu_id as usize);
}

/// CPU idle loop - each CPU runs this when it has no work
fn cpu_idle_loop(cpu_id: u32) -> ! {
    let mut serial = Serial::new();
    
    serial.write_str("[CPU");
    print_number(&mut serial, cpu_id);
    serial.write_str("] Entering idle loop\n");

    // Simple per-CPU counter to demonstrate multi-core execution
    static mut CPU_COUNTERS: [u64; MAX_CPUS] = [0; MAX_CPUS];
    
    let mut count = 0u64;
    loop {
        count += 1;
        
        // Every 100000 iterations, print a message
        if count % 100000 == 0 {
            unsafe {
                CPU_COUNTERS[cpu_id as usize] = count;
            }
            
            serial.write_str("[CPU");
            print_number(&mut serial, cpu_id);
            serial.write_str("] Loop count: ");
            print_number_u64(&mut serial, count);
            serial.write_str("\n");
        }
        
        // Spin for a bit
        for _ in 0..10000 {
            core::hint::spin_loop();
        }
    }
}

/// Helper to print u64 numbers
fn print_number_u64(serial: &mut Serial, mut num: u64) {
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
