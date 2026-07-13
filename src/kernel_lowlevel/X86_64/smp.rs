#![allow(dead_code)]

use crate::kernel_lowlevel::serial::Serial;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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

impl PerCpuData {
    pub const fn new() -> Self {
        Self {
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

pub fn read_mpidr() -> u64 {
    current_cpu_id() as u64
}

pub fn current_cpu_id() -> u32 {
    0
}

pub fn is_boot_cpu() -> bool {
    true
}

pub fn boot_secondary_cpu(cpu_id: u32, _stack_ptr: u64) -> Result<(), &'static str> {
    if !lowlevel_logic::valid_cpu_id(cpu_id, MAX_CPUS) {
        return Err("Invalid CPU ID");
    }
    Err("x86_64 AP startup is not wired yet")
}

pub fn system_reset() -> ! {
    let mut serial = Serial::new();
    serial.init();
    serial.write_str("[X86] System reset requested\n");

    // QEMU q35 exposes the ACPI reset control at 0xcf9. The 8042 command is a
    // fallback for older PC-compatible machines if the first write returns.
    unsafe {
        outb(0xcf9, 0x06);
        outb(0x64, 0xfe);
    }

    serial.write_str("[X86] System reset returned; halting\n");
    loop {
        crate::kernel_lowlevel::cpu::wait_for_event();
    }
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags),
    );
}

pub fn init() {
    let mut serial = Serial::new();
    serial.init();
    serial.write_str("[SMP] Initializing x86_64 CPU support...\n");
    let per_cpu = unsafe { &mut *per_cpu_mut() };
    per_cpu.cpu_info[0].cpu_id = 0;
    per_cpu.cpu_info[0].state = CpuState::Online;
    per_cpu.cpu_info[0].mpidr = 0;
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
        per_cpu.cpu_info[i].mpidr = i as u64;
    }
    per_cpu
        .online_count
        .store(MAX_CPUS as u32, Ordering::Relaxed);
    serial.write_str("[SMP] All ");
    print_number(&mut serial, MAX_CPUS as u32);
    serial.write_str(" logical CPUs initialized\n");
}

pub fn mark_cpu_online() {}

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
        serial.write_str("  apic: ");
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
    let mut len = 0;
    while num > 0 && len < buf.len() {
        buf[len] = b'0' + (num % 10) as u8;
        num /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        serial.write_byte(buf[len]);
    }
}

#[no_mangle]
pub extern "C" fn secondary_cpu_entry() -> ! {
    crate::kernel_objects::scheduler::start_first_thread_for_cpu(current_cpu_id() as usize);
}
