#![allow(dead_code)]
//! ARM Generic Timer Driver
//!
//! This module provides access to the ARM Generic Timer (CNTFRQ, CNTPCT, CNTP_TVAL, etc.)
//! which is used for system timing and scheduler ticks.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{drivers, lowlevel_logic};

/// ARM Generic Timer registers (Physical Timer)
const CNTFRQ_EL0: usize = 0xFD80; // Counter-timer Frequency Register
const CNTPCT_EL0: usize = 0xFD40; // Counter-timer Physical Count Register
const CNTP_CTL_EL0: usize = 0xFC80; // Counter-timer Physical Timer Control Register
const CNTP_CVAL_EL0: usize = 0xFC90; // Counter-timer Physical Timer CompareValue Register

/// CNTP_CTL_EL0 bits
const CNTP_CTL_ENABLE: u64 = 1 << 0; // Timer enable
const CNTP_CTL_IMASK: u64 = 1 << 1; // Timer interrupt mask

/// Timer tick frequency (set at runtime)
static TIMER_FREQUENCY: AtomicU64 = AtomicU64::new(0);

/// Timer tick period in timer counts (for 10ms tick)
static TICK_PERIOD: AtomicU64 = AtomicU64::new(0);

/// Read the Counter-timer Frequency Register
fn read_cntfrq_el0() -> u64 {
    let val: u64;
    // SAFETY: Reading CNTFRQ_EL0 is a standard ARM system register access.
    // This is safe because it's a read-only register that returns the timer frequency.
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntfrq_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Read the Counter-timer Physical Count Register
fn read_cntpct_el0() -> u64 {
    let val: u64;
    // SAFETY: Reading CNTPCT_EL0 is a standard ARM system register access.
    // This is safe because it's a read-only register that returns the current tick count.
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntpct_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Write the Counter-timer Physical Timer CompareValue Register
fn write_cntp_cval_el0(value: u64) {
    // SAFETY: Writing CNTP_CVAL_EL0 sets the timer compare value.
    // This is safe because we own the timer and are single-threaded during init.
    unsafe {
        core::arch::asm!(
            "msr cntp_cval_el0, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Write the Counter-timer Physical Timer Control Register
fn write_cntp_ctl_el0(value: u64) {
    // SAFETY: Writing CNTP_CTL_EL0 controls the physical timer.
    // This is safe because we own the timer and are single-threaded during init.
    unsafe {
        core::arch::asm!(
            "msr cntp_ctl_el0, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Initialize the ARM Generic Timer
pub fn init() {
    let _platform_irq = interrupt_id();
    let freq = read_cntfrq_el0();
    TIMER_FREQUENCY.store(freq, Ordering::Relaxed);

    // Set tick period for 10ms (100Hz scheduler tick)
    let tick_period = lowlevel_logic::timer_period(freq);
    TICK_PERIOD.store(tick_period, Ordering::Relaxed);

    // Disable timer during setup
    write_cntp_ctl_el0(0);

    // Set the timer to fire after TICK_PERIOD counts
    let current_count = read_cntpct_el0();
    let compare_value = lowlevel_logic::timer_compare(current_count, tick_period);
    write_cntp_cval_el0(compare_value);

    // Enable timer with interrupt unmasked
    write_cntp_ctl_el0(lowlevel_logic::timer_ctl(CNTP_CTL_ENABLE, CNTP_CTL_IMASK));
}

/// Get the timer frequency
pub fn get_frequency() -> u64 {
    TIMER_FREQUENCY.load(Ordering::Relaxed)
}

/// Get the platform interrupt ID wired to the ARM physical timer.
pub fn interrupt_id() -> u32 {
    drivers::timer_irq()
}

/// Get the current tick count
pub fn get_tick_count() -> u64 {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    lowlevel_logic::timer_tick_count(read_cntpct_el0(), period)
}

/// Arm the timer for the next tick
pub fn arm_next_tick() {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    let current_count = read_cntpct_el0();
    let compare_value = lowlevel_logic::timer_compare(current_count, period);
    write_cntp_cval_el0(compare_value);
}

/// Clear timer interrupt by re-arming the timer
pub fn clear_interrupt() {
    arm_next_tick();
}
