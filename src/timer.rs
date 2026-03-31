//! ARM Generic Timer Driver
//!
//! This module provides access to the ARM Generic Timer (CNTFRQ, CNTPCT, CNTP_TVAL, etc.)
//! which is used for system timing and scheduler ticks.

/// ARM Generic Timer registers (Physical Timer)
const CNTFRQ_EL0: usize = 0xFD80; // Counter-timer Frequency Register
const CNTPCT_EL0: usize = 0xFD40; // Counter-timer Physical Count Register
const CNTP_CTL_EL0: usize = 0xFC80; // Counter-timer Physical Timer Control Register
const CNTP_CVAL_EL0: usize = 0xFC90; // Counter-timer Physical Timer CompareValue Register

/// CNTP_CTL_EL0 bits
const CNTP_CTL_ENABLE: u64 = 1 << 0;  // Timer enable
const CNTP_CTL_IMASK: u64 = 1 << 1;   // Timer interrupt mask
const CNTP_CTL_ISTATUS: u64 = 1 << 2; // Timer interrupt status

/// Timer tick frequency (will be detected at runtime)
static mut TIMER_FREQUENCY: u64 = 0;

/// Timer tick period in timer counts (for 10ms tick)
static mut TICK_PERIOD: u64 = 0;

/// Initialize the ARM Generic Timer
///
/// # Safety
/// This function accesses system registers directly
pub unsafe fn init() {
    // Read the timer frequency
    let freq = read_cntfrq_el0();
    TIMER_FREQUENCY = freq;
    
    // Set tick period for 10ms (100Hz scheduler tick)
    TICK_PERIOD = freq / 100;
    
    // Disable timer during setup
    write_cntp_ctl_el0(0);
    
    // Set the timer to fire after TICK_PERIOD counts
    let current_count = read_cntpct_el0();
    let compare_value = current_count.wrapping_add(TICK_PERIOD);
    write_cntp_cval_el0(compare_value);
    
    // Enable timer with interrupt unmasked
    write_cntp_ctl_el0(CNTP_CTL_ENABLE | CNTP_CTL_IMASK);
}

/// Read the Counter-timer Frequency Register
fn read_cntfrq_el0() -> u64 {
    let val: u64;
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
    unsafe {
        core::arch::asm!(
            "msr cntp_ctl_el0, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read the Counter-timer Physical Timer Control Register
pub fn read_cntp_ctl_el0() -> u64 {
    let val: u64;
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntp_ctl_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Get the timer frequency
pub fn get_frequency() -> u64 {
    unsafe { TIMER_FREQUENCY }
}

/// Get the current tick count
pub fn get_tick_count() -> u64 {
    unsafe { read_cntpct_el0() / TICK_PERIOD }
}

/// Arm the timer for the next tick
pub fn arm_next_tick() {
    unsafe {
        let current_count = read_cntpct_el0();
        let compare_value = current_count.wrapping_add(TICK_PERIOD);
        write_cntp_cval_el0(compare_value);
    }
}

/// Check if timer interrupt is pending
pub fn is_interrupt_pending() -> bool {
    let ctl = read_cntp_ctl_el0();
    (ctl & CNTP_CTL_ISTATUS) != 0
}

/// Clear timer interrupt by re-arming the timer
pub fn clear_interrupt() {
    arm_next_tick();
}
