#![allow(dead_code)]
//! RISC-V64 hart bookkeeping.

use crate::kernel_lowlevel::serial::Serial;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use super::lowlevel_logic;

pub const MAX_CPUS: usize = include!(concat!(env!("OUT_DIR"), "/build_config.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Offline,
    Booting,
    Online,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuInfo {
    pub cpu_id: u32,
    pub state: CpuState,
    pub mpidr: u64,
    pub stack_ptr: u64,
}

#[repr(C, align(64))]
pub struct PerCpuData {
    pub cpu_info: [CpuInfo; MAX_CPUS],
    pub online_count: AtomicU32,
    pub boot_lock: AtomicU32,
    pub secondary_entry_flag: AtomicU64,
}

unsafe impl Send for PerCpuData {}
unsafe impl Sync for PerCpuData {}

static mut PER_CPU: PerCpuData = PerCpuData::new();
static BOOT_HART_ID: AtomicUsize = AtomicUsize::new(0);

impl PerCpuData {
    pub const fn new() -> Self {
        PerCpuData {
            cpu_info: [CpuInfo {
                cpu_id: 0,
                state: CpuState::Offline,
                mpidr: 0,
                stack_ptr: 0,
            }; MAX_CPUS],
            online_count: AtomicU32::new(1),
            boot_lock: AtomicU32::new(0),
            secondary_entry_flag: AtomicU64::new(0),
        }
    }
}

pub fn per_cpu() -> *const PerCpuData {
    &raw const PER_CPU
}

pub fn per_cpu_mut() -> *mut PerCpuData {
    &raw mut PER_CPU
}

pub fn online_cpu_count() -> u32 {
    let per_cpu = unsafe { &*per_cpu() };
    per_cpu.online_count.load(Ordering::Relaxed)
}

pub fn read_hartid() -> usize {
    BOOT_HART_ID.load(Ordering::Relaxed)
}

pub fn read_mpidr() -> u64 {
    read_hartid() as u64
}

#[no_mangle]
pub extern "C" fn riscv64_record_boot_hart(hartid: usize) {
    BOOT_HART_ID.store(hartid, Ordering::Relaxed);
}

pub fn current_cpu_id() -> u32 {
    let index = crate::kernel_lowlevel::drivers::hart_index(read_hartid());
    if lowlevel_logic::valid_cpu_id(index as u32, MAX_CPUS) {
        index as u32
    } else {
        0
    }
}

pub fn is_boot_cpu() -> bool {
    current_cpu_id() == 0
}

pub fn boot_secondary_cpu(cpu_id: u32, stack_ptr: u64) -> Result<(), &'static str> {
    let _ = stack_ptr;
    if !lowlevel_logic::valid_cpu_id(cpu_id, MAX_CPUS) {
        return Err("Invalid CPU ID");
    }
    Err("RISC-V SBI HSM hart start not wired yet")
}

pub fn system_reset() -> ! {
    let mut serial = Serial::new();
    serial.init();
    serial.write_str("[SBI] System reset requested\n");
    let _ = sbi_system_reset();
    serial.write_str("[SBI] Reset returned; halting\n");
    loop {
        crate::kernel_lowlevel::cpu::wait_for_interrupt();
    }
}

pub fn init() {
    let mut serial = Serial::new();
    serial.init();
    serial.write_str("[SMP] Initializing RISC-V hart support...\n");

    let per_cpu = unsafe { &mut *per_cpu_mut() };
    per_cpu.cpu_info[0].cpu_id = current_cpu_id();
    per_cpu.cpu_info[0].state = CpuState::Online;
    per_cpu.cpu_info[0].mpidr = read_hartid() as u64;

    serial.write_str("[SMP] Boot hart: ");
    print_number(&mut serial, per_cpu.cpu_info[0].mpidr as u32);
    serial.write_str("\n");
}

pub fn boot_all_cpus() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("[SMP] Multi-core initialization...\n");
    serial.write_str("[SMP] Note: Using logical CPU affinity model\n");
    serial.write_str("[SMP] Scheduler will distribute threads across ");
    print_number(&mut serial, MAX_CPUS as u32);
    serial.write_str(" logical CPUs\n");

    let per_cpu = unsafe { &mut *per_cpu_mut() };
    for i in 0..MAX_CPUS {
        per_cpu.cpu_info[i].cpu_id = i as u32;
        per_cpu.cpu_info[i].state = CpuState::Online;
        per_cpu.cpu_info[i].mpidr = crate::kernel_lowlevel::drivers::hart_id(i).unwrap_or(i) as u64;
    }
    per_cpu
        .online_count
        .store(MAX_CPUS as u32, Ordering::Relaxed);

    serial.write_str("[SMP] All ");
    print_number(&mut serial, MAX_CPUS as u32);
    serial.write_str(" logical CPUs initialized\n");
}

pub fn mark_cpu_online() {
    let cpu_id = current_cpu_id();
    if lowlevel_logic::valid_cpu_id(cpu_id, MAX_CPUS) {
        let per_cpu = unsafe { &mut *per_cpu_mut() };
        per_cpu.cpu_info[cpu_id as usize].state = CpuState::Online;
        let _ = per_cpu.online_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn print_status() {
    let mut serial = Serial::new();
    serial.init();

    serial.write_str("\n=== SMP Status ===\n");
    serial.write_str("Online CPUs: ");
    let per_cpu = unsafe { &*per_cpu() };
    print_number(&mut serial, per_cpu.online_count.load(Ordering::Relaxed));
    serial.write_str("/");
    print_number(&mut serial, MAX_CPUS as u32);
    serial.write_str("\n\n");

    for i in 0..MAX_CPUS {
        serial.write_str("CPU");
        print_number(&mut serial, i as u32);
        serial.write_str(": ");
        match per_cpu.cpu_info[i].state {
            CpuState::Offline => serial.write_str("Offline"),
            CpuState::Booting => serial.write_str("Booting"),
            CpuState::Online => serial.write_str("Online"),
        }
        serial.write_str("  hart: ");
        print_number(&mut serial, per_cpu.cpu_info[i].mpidr as u32);
        serial.write_str("\n");
    }
    serial.write_str("====================\n");
}

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

#[no_mangle]
pub extern "C" fn secondary_cpu_entry() -> ! {
    mark_cpu_online();
    crate::kernel_objects::scheduler::start_first_thread_for_cpu(current_cpu_id() as usize);
}

#[inline(always)]
fn sbi_system_reset() -> isize {
    const SBI_EXT_SRST: usize = 0x5352_5354;
    const SBI_SRST_RESET: usize = 0;
    const SBI_SRST_TYPE_COLD_REBOOT: usize = 1;
    const SBI_SRST_REASON_NONE: usize = 0;
    let error: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") SBI_SRST_TYPE_COLD_REBOOT => error,
            in("a1") SBI_SRST_REASON_NONE,
            in("a6") SBI_SRST_RESET,
            in("a7") SBI_EXT_SRST,
            options(nostack),
        );
    }
    error
}
