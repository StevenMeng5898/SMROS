//! ARM GICv2 Interrupt Controller Driver
//!
//! This module provides interrupt controller functionality for handling
//! hardware interrupts including the timer interrupt.

use core::ptr::{read_volatile, write_volatile};

use super::lowlevel_logic;

/// GICv2 Distributor Base Address (QEMU virt machine)
const GICD_BASE: usize = 0x8000000;

/// GICv2 CPU Interface Base Address (QEMU virt machine)
const GICC_BASE: usize = 0x8010000;

/// Distributor Register offsets
const GICD_CTLR: usize = 0x000; // Distributor Control Register
const GICD_IGROUPR: usize = 0x080; // Interrupt Group Registers
const GICD_ISENABLER: usize = 0x100; // Interrupt Set-Enable Registers
const GICD_IPRIORITYR: usize = 0x400; // Interrupt Priority Registers
const GICD_ITARGETSR: usize = 0x800; // Interrupt Processor Targets Registers

/// CPU Interface Register offsets
const GICC_CTLR: usize = 0x000; // CPU Interface Control Register
const GICC_PMR: usize = 0x004; // Priority Mask Register
const GICC_BPR: usize = 0x008; // Binary Point Register
const GICC_IAR: usize = 0x00C; // Interrupt Acknowledge Register
const GICC_EOIR: usize = 0x010; // End of Interrupt Register

/// GICD_CTLR bits
const GICD_CTLR_ENABLE: u32 = 1 << 0;

/// GICC_CTLR bits
const GICC_CTLR_ENABLE: u32 = 1 << 0;

/// Interrupt priorities (lower number = higher priority)
const PRIORITY_HIGH: u8 = 0x50;

/// Physical Timer IRQ number (ARM Generic Timer)
pub const TIMER_IRQ: u32 = 30;

/// Write a GIC register (MMIO access)
fn write_reg(base: usize, offset: usize, value: u32) {
    // SAFETY: GICD_BASE and GICC_BASE are valid MMIO addresses defined by the
    // QEMU virt machine spec. Offsets are constants from the GICv2 TRM.
    let addr = lowlevel_logic::mmio_addr(base, offset).unwrap_or(base);
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Read a GIC register (MMIO access)
fn read_reg(base: usize, offset: usize) -> u32 {
    // SAFETY: GICD_BASE and GICC_BASE are valid MMIO addresses defined by the
    // QEMU virt machine spec. Offsets are constants from the GICv2 TRM.
    let addr = lowlevel_logic::mmio_addr(base, offset).unwrap_or(base);
    unsafe { read_volatile(addr as *const u32) }
}

/// Initialize the GICv2 interrupt controller
pub fn init() {
    // Enable the distributor
    write_reg(GICD_BASE, GICD_CTLR, GICD_CTLR_ENABLE);

    // Configure all interrupts as Group 0 (secure)
    write_reg(GICD_BASE, GICD_IGROUPR, 0x00000000);

    // Set priority for timer interrupt (PPI 30)
    let priority_offset = lowlevel_logic::gic_reg_offset(GICD_IPRIORITYR, TIMER_IRQ, 4);
    let mut priorities = read_reg(GICD_BASE, priority_offset);
    let byte_shift = lowlevel_logic::gic_byte_shift(TIMER_IRQ);
    priorities = lowlevel_logic::gic_set_byte_field(priorities, byte_shift, PRIORITY_HIGH);
    write_reg(GICD_BASE, priority_offset, priorities);

    // Set target CPU for PPIs (CPU0)
    let target_offset = lowlevel_logic::gic_reg_offset(GICD_ITARGETSR, TIMER_IRQ, 4);
    let mut targets = read_reg(GICD_BASE, target_offset);
    let byte_shift = lowlevel_logic::gic_byte_shift(TIMER_IRQ);
    targets = lowlevel_logic::gic_set_byte_field(targets, byte_shift, 0x01);
    write_reg(GICD_BASE, target_offset, targets);

    // Enable the timer interrupt at distributor
    let enable_offset = lowlevel_logic::gic_reg_offset(GICD_ISENABLER, TIMER_IRQ, 32);
    let enable_bit = lowlevel_logic::gic_enable_bit(TIMER_IRQ);
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

/// Acknowledge an interrupt and return the interrupt ID
pub fn acknowledge_interrupt() -> u32 {
    let iar = read_reg(GICC_BASE, GICC_IAR);
    lowlevel_logic::gic_interrupt_id(iar) // Return interrupt ID (lower 10 bits)
}

/// Signal end of interrupt
pub fn end_of_interrupt(interrupt_id: u32) {
    write_reg(GICC_BASE, GICC_EOIR, interrupt_id);
}
