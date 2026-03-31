//! ARM GICv2 Interrupt Controller Driver
//!
//! This module provides interrupt controller functionality for handling
//! hardware interrupts including the timer interrupt.

use core::ptr::{read_volatile, write_volatile};

/// GICv2 Distributor Base Address (QEMU virt machine)
const GICD_BASE: usize = 0x8000000;

/// GICv2 CPU Interface Base Address (QEMU virt machine)
const GICC_BASE: usize = 0x8010000;

/// Distributor Register offsets
const GICD_CTLR: usize = 0x000;   // Distributor Control Register
const GICD_TYPER: usize = 0x004;  // Interrupt Controller Type Register
const GICD_IGROUPR: usize = 0x080; // Interrupt Group Registers
const GICD_ISENABLER: usize = 0x100; // Interrupt Set-Enable Registers
const GICD_ICENABLER: usize = 0x180; // Interrupt Clear-Enable Registers
const GICD_ISPENDR: usize = 0x200;   // Interrupt Set-Pending Registers
const GICD_ICPENDR: usize = 0x280;   // Interrupt Clear-Pending Registers
const GICD_IPRIORITYR: usize = 0x400; // Interrupt Priority Registers
const GICD_ITARGETSR: usize = 0x800;  // Interrupt Processor Targets Registers
const GICD_SGIR: usize = 0xF00;       // Software Generated Interrupt Register

/// CPU Interface Register offsets
const GICC_CTLR: usize = 0x000;   // CPU Interface Control Register
const GICC_PMR: usize = 0x004;    // Priority Mask Register
const GICC_BPR: usize = 0x008;    // Binary Point Register
const GICC_IAR: usize = 0x00C;    // Interrupt Acknowledge Register
const GICC_EOIR: usize = 0x010;   // End of Interrupt Register
const GICC_RPR: usize = 0x014;    // Running Priority Register
const GICC_HPPIR: usize = 0x018;  // Highest Priority Pending Interrupt Register
const GICC_AHPPIR: usize = 0x01C; // Aliased Highest Priority Pending Interrupt Register
const GICC_AIAR: usize = 0x020;   // Aliased Interrupt Acknowledge Register
const GICC_AEOIR: usize = 0x024;  // Aliased End of Interrupt Register

/// GICD_CTLR bits
const GICD_CTLR_ENABLE: u32 = 1 << 0;

/// GICC_CTLR bits
const GICC_CTLR_ENABLE: u32 = 1 << 0;

/// Interrupt priorities (lower number = higher priority)
const PRIORITY_HIGH: u8 = 0x50;
const PRIORITY_NORMAL: u8 = 0x80;

/// Physical Timer IRQ number (ARM Generic Timer)
pub const TIMER_IRQ: u32 = 30;

/// Initialize the GICv2 interrupt controller
///
/// # Safety
/// This function accesses hardware registers directly
pub unsafe fn init() {
    // Enable the distributor
    write_reg(GICD_BASE, GICD_CTLR, GICD_CTLR_ENABLE);
    
    // Configure all interrupts as Group 0 (secure)
    // For simplicity, configure first 32 interrupts (SGIs and PPIs)
    write_reg(GICD_BASE, GICD_IGROUPR, 0x00000000); // All Group 0
    
    // Set priority for timer interrupt (PPI 30)
    let priority_offset = GICD_IPRIORITYR + (TIMER_IRQ as usize / 4) * 4;
    let mut priorities = read_reg(GICD_BASE, priority_offset);
    let byte_shift = (TIMER_IRQ % 4) as usize * 8;
    priorities &= !(0xFF << byte_shift);
    priorities |= (PRIORITY_HIGH as u32) << byte_shift;
    write_reg(GICD_BASE, priority_offset, priorities);
    
    // Set target CPU for PPIs (CPU0)
    let target_offset = GICD_ITARGETSR + (TIMER_IRQ as usize / 4) * 4;
    let mut targets = read_reg(GICD_BASE, target_offset);
    let byte_shift = (TIMER_IRQ % 4) as usize * 8;
    targets &= !(0xFF << byte_shift);
    targets |= 0x01 << byte_shift; // Target CPU0
    write_reg(GICD_BASE, target_offset, targets);
    
    // Enable the timer interrupt at distributor
    let enable_offset = GICD_ISENABLER + (TIMER_IRQ as usize / 32) * 4;
    let enable_bit = 1 << (TIMER_IRQ % 32);
    write_reg(GICD_BASE, enable_offset, enable_bit);
    
    // Enable the CPU interface
    write_reg(GICC_BASE, GICC_CTLR, GICC_CTLR_ENABLE);
    
    // Set priority mask to allow all priorities
    write_reg(GICC_BASE, GICC_PMR, 0xFF);
    
    // Set binary point
    write_reg(GICC_BASE, GICC_BPR, 0);
}

/// Enable the timer interrupt
pub fn enable_timer_interrupt() {
    // The timer interrupt (PPI 30) is enabled at the GIC level
    // The actual enabling is done in the timer driver
}

/// Disable the timer interrupt
pub fn disable_timer_interrupt() {
    let disable_offset = GICD_ICENABLER + (TIMER_IRQ as usize / 32) * 4;
    let disable_bit = 1 << (TIMER_IRQ % 32);
    unsafe {
        write_reg(GICD_BASE, disable_offset, disable_bit);
    }
}

/// Acknowledge an interrupt and return the interrupt ID
pub fn acknowledge_interrupt() -> u32 {
    let iar = unsafe { read_reg(GICC_BASE, GICC_IAR) };
    iar & 0x3FF // Return interrupt ID (lower 10 bits)
}

/// Signal end of interrupt
pub fn end_of_interrupt(interrupt_id: u32) {
    unsafe {
        write_reg(GICC_BASE, GICC_EOIR, interrupt_id);
    }
}

/// Check if there's a pending interrupt
pub fn has_pending_interrupt() -> bool {
    let hppir = unsafe { read_reg(GICC_BASE, GICC_HPPIR) };
    (hppir & 0x3FF) != 1023 // 1023 means no pending interrupt
}

/// Write a GIC register
fn write_reg(base: usize, offset: usize, value: u32) {
    unsafe {
        write_volatile((base + offset) as *mut u32, value);
    }
}

/// Read a GIC register
fn read_reg(base: usize, offset: usize) -> u32 {
    unsafe {
        read_volatile((base + offset) as *const u32)
    }
}

/// Send a Software Generated Interrupt (SGI)
pub fn send_sgi(target_cpu: u32, sgi_id: u32) {
    let sgir_value = ((target_cpu & 0xFF) << 16) | (sgi_id & 0xF);
    unsafe {
        write_reg(GICD_BASE, GICD_SGIR, sgir_value);
    }
}
