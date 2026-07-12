#![allow(dead_code)]

use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::{AtomicUsize, Ordering};

const BDA_COM1_PORT: usize = 0x400;
const DEFAULT_TSC_HZ: usize = 1_000_000_000;
const CPUID_VENDOR_BYTES: usize = 12;
const CPU_NAME_BYTES: usize = 48;
const VENDOR_BUF_BYTES: usize = CPUID_VENDOR_BYTES + 1;
const CPU_NAME_BUF_BYTES: usize = CPU_NAME_BYTES + 1;
const X86_FEATURE_EDX_APIC: u32 = 1 << 9;
const IA32_APIC_BASE: u32 = 0x1b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceSource {
    Uninitialized,
    BiosDataArea,
}

impl ResourceSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceSource::Uninitialized => "uninitialized",
            ResourceSource::BiosDataArea => "bda/cpuid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceReg {
    pub base: usize,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverStats {
    pub initialized: bool,
    pub machine: &'static str,
    pub source: ResourceSource,
    pub uart_base: usize,
    pub uart_size: usize,
    pub tsc_frequency: u64,
    pub lapic_base: usize,
    pub cpu_count: usize,
}

static INIT_STATE: AtomicUsize = AtomicUsize::new(0);
static RESOURCE_SOURCE: AtomicUsize = AtomicUsize::new(ResourceSource::Uninitialized as usize);
static UART_BASE: AtomicUsize = AtomicUsize::new(0);
static TSC_FREQUENCY: AtomicUsize = AtomicUsize::new(DEFAULT_TSC_HZ);
static LAPIC_BASE: AtomicUsize = AtomicUsize::new(0);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(1);
static mut VENDOR: [u8; VENDOR_BUF_BYTES] = [0; VENDOR_BUF_BYTES];
static mut CPU_NAME: [u8; CPU_NAME_BUF_BYTES] = [0; CPU_NAME_BUF_BYTES];

pub fn init() -> bool {
    init_from_fdt(0)
}

pub fn init_from_fdt(_boot_info: usize) -> bool {
    let serial = bios_com1_port();
    UART_BASE.store(serial, Ordering::Release);
    TSC_FREQUENCY.store(detect_tsc_frequency() as usize, Ordering::Release);
    LAPIC_BASE.store(detect_lapic_base(), Ordering::Release);
    CPU_COUNT.store(1, Ordering::Release);
    write_cpu_strings();
    RESOURCE_SOURCE.store(ResourceSource::BiosDataArea as usize, Ordering::Release);
    INIT_STATE.store(1, Ordering::Release);
    serial != 0
}

pub fn architecture_name() -> &'static str {
    "X86_64"
}

pub fn uart_base() -> usize {
    UART_BASE.load(Ordering::Acquire)
}

pub fn uart_size() -> usize {
    if uart_base() == 0 {
        0
    } else {
        8
    }
}

pub fn tsc_frequency() -> u64 {
    TSC_FREQUENCY.load(Ordering::Acquire) as u64
}

pub fn lapic_base() -> usize {
    LAPIC_BASE.load(Ordering::Acquire)
}

pub fn cpu_count() -> usize {
    CPU_COUNT.load(Ordering::Acquire)
}

pub fn virtio_mmio_count() -> usize {
    0
}

pub fn virtio_mmio_reg(_index: usize) -> Option<DeviceReg> {
    None
}

pub fn cpu_vendor() -> &'static str {
    unsafe {
        core::str::from_utf8_unchecked(cstr_bytes(&raw const VENDOR as *const u8, VENDOR_BUF_BYTES))
    }
}

pub fn cpu_name() -> &'static str {
    unsafe {
        core::str::from_utf8_unchecked(cstr_bytes(
            &raw const CPU_NAME as *const u8,
            CPU_NAME_BUF_BYTES,
        ))
    }
}

pub fn stats() -> DriverStats {
    DriverStats {
        initialized: INIT_STATE.load(Ordering::Acquire) != 0,
        machine: cpu_name(),
        source: resource_source(),
        uart_base: uart_base(),
        uart_size: uart_size(),
        tsc_frequency: tsc_frequency(),
        lapic_base: lapic_base(),
        cpu_count: cpu_count(),
    }
}

pub fn describe(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    let snapshot = stats();
    serial.write_str("[DRV] Platform: ");
    serial.write_str(snapshot.machine);
    serial.write_str(" vendor=");
    serial.write_str(cpu_vendor());
    serial.write_str(" source=");
    serial.write_str(snapshot.source.as_str());
    serial.write_str(" uart-port=0x");
    serial.write_hex(snapshot.uart_base as u64);
    serial.write_str(" tsc=");
    print_number(serial, snapshot.tsc_frequency as u32);
    serial.write_str("Hz lapic=0x");
    serial.write_hex(snapshot.lapic_base as u64);
    serial.write_str(" cpus=");
    print_number(serial, snapshot.cpu_count as u32);
    serial.write_str("\n");
}

fn resource_source() -> ResourceSource {
    match RESOURCE_SOURCE.load(Ordering::Acquire) {
        value if value == ResourceSource::BiosDataArea as usize => ResourceSource::BiosDataArea,
        _ => ResourceSource::Uninitialized,
    }
}

fn bios_com1_port() -> usize {
    unsafe { core::ptr::read_volatile(BDA_COM1_PORT as *const u16) as usize }
}

fn detect_tsc_frequency() -> u64 {
    let max_leaf = __cpuid(0).eax;
    if max_leaf >= 0x15 {
        let leaf = __cpuid_count(0x15, 0);
        let denominator = leaf.eax as u64;
        let numerator = leaf.ebx as u64;
        let crystal = leaf.ecx as u64;
        if denominator != 0 && numerator != 0 && crystal != 0 {
            return crystal.saturating_mul(numerator) / denominator;
        }
    }
    if max_leaf >= 0x16 {
        let leaf = __cpuid(0x16);
        let mhz = leaf.eax as u64;
        if mhz != 0 {
            return mhz.saturating_mul(1_000_000);
        }
    }
    DEFAULT_TSC_HZ as u64
}

fn detect_lapic_base() -> usize {
    let features = __cpuid(1);
    if features.edx & X86_FEATURE_EDX_APIC == 0 {
        return 0;
    }
    unsafe { read_msr(IA32_APIC_BASE) as usize & 0xffff_f000 }
}

fn write_cpu_strings() {
    let vendor = __cpuid(0);
    unsafe {
        let vendor_ptr = (&raw mut VENDOR).cast::<u8>();
        copy_u32(vendor_ptr, VENDOR_BUF_BYTES, 0, vendor.ebx);
        copy_u32(vendor_ptr, VENDOR_BUF_BYTES, 4, vendor.edx);
        copy_u32(vendor_ptr, VENDOR_BUF_BYTES, 8, vendor.ecx);
        core::ptr::write_volatile(vendor_ptr.add(CPUID_VENDOR_BYTES), 0);
    }

    let max_ext = __cpuid(0x8000_0000).eax;
    unsafe {
        let cpu_name_ptr = (&raw mut CPU_NAME).cast::<u8>();
        if max_ext >= 0x8000_0004 {
            let leaves = [
                __cpuid(0x8000_0002),
                __cpuid(0x8000_0003),
                __cpuid(0x8000_0004),
            ];
            let mut offset = 0;
            for leaf in leaves {
                for value in [leaf.eax, leaf.ebx, leaf.ecx, leaf.edx] {
                    copy_u32(cpu_name_ptr, CPU_NAME_BUF_BYTES, offset, value);
                    offset += 4;
                }
            }
            core::ptr::write_volatile(cpu_name_ptr.add(CPU_NAME_BYTES), 0);
            trim_cpu_name(cpu_name_ptr);
        } else {
            write_cstr(cpu_name_ptr, CPU_NAME_BUF_BYTES, b"x86_64 CPU");
        }
    }
}

unsafe fn copy_u32(buf: *mut u8, max: usize, offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    for index in 0..4 {
        if offset + index < max {
            core::ptr::write_volatile(buf.add(offset + index), bytes[index]);
        }
    }
}

unsafe fn trim_cpu_name(buf: *mut u8) {
    let mut start = 0;
    while start < CPU_NAME_BYTES && core::ptr::read_volatile(buf.add(start)) == b' ' {
        start += 1;
    }
    if start != 0 {
        let mut out = 0;
        while start + out < CPU_NAME_BYTES {
            let value = core::ptr::read_volatile(buf.add(start + out));
            core::ptr::write_volatile(buf.add(out), value);
            out += 1;
        }
        while out < CPU_NAME_BYTES {
            core::ptr::write_volatile(buf.add(out), 0);
            out += 1;
        }
    }
}

unsafe fn write_cstr(buf: *mut u8, max: usize, value: &[u8]) {
    let mut index = 0;
    while index < value.len() && index + 1 < max {
        core::ptr::write_volatile(buf.add(index), value[index]);
        index += 1;
    }
    if index < max {
        core::ptr::write_volatile(buf.add(index), 0);
    }
}

unsafe fn cstr_bytes<'a>(ptr: *const u8, max: usize) -> &'a [u8] {
    let mut len = 0;
    while len < max {
        if core::ptr::read_volatile(ptr.add(len)) == 0 {
            break;
        }
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}

fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
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
