//! ARM Generic Timer Driver
//!
//! This module provides access to the ARM Generic Timer (CNTFRQ, CNTPCT, CNTP_TVAL, etc.)
//! which is used for system timing and scheduler ticks.

use core::sync::atomic::{AtomicU64, Ordering};

/// ARM Generic Timer registers (Physical Timer)
const CNTFRQ_EL0: usize = 0xFD80; // Counter-timer Frequency Register
const CNTPCT_EL0: usize = 0xFD40; // Counter-timer Physical Count Register
const CNTP_CTL_EL0: usize = 0xFC80; // Counter-timer Physical Timer Control Register
const CNTP_CVAL_EL0: usize = 0xFC90; // Counter-timer Physical Timer CompareValue Register

/// CNTP_CTL_EL0 bits
const CNTP_CTL_ENABLE: u64 = 1 << 0;  // Timer enable
const CNTP_CTL_IMASK: u64 = 1 << 1;   // Timer interrupt mask

/// Timer tick frequency (set at runtime)
static TIMER_FREQUENCY: AtomicU64 = AtomicU64::new(0);

/// Timer tick period in timer counts (for 10ms tick)
static TICK_PERIOD: AtomicU64 = AtomicU64::new(0);

/// Initialize the ARM Generic Timer
///
/// # Safety
/// This function accesses system registers directly.
/// Must only be called once during kernel initialization.
pub fn init() {
    // SAFETY: We're reading system registers during early boot.
    // This is safe because we're single-threaded at this point.
    let freq = unsafe { read_cntfrq_el0() };
    TIMER_FREQUENCY.store(freq, Ordering::Relaxed);

    // Set tick period for 10ms (100Hz scheduler tick)
    let tick_period = freq / 100;
    TICK_PERIOD.store(tick_period, Ordering::Relaxed);

    // Disable timer during setup
    unsafe { write_cntp_ctl_el0(0) };

    // Set the timer to fire after TICK_PERIOD counts
    let current_count = unsafe { read_cntpct_el0() };
    let compare_value = current_count.wrapping_add(tick_period);
    unsafe { write_cntp_cval_el0(compare_value) };

    // Enable timer with interrupt unmasked
    unsafe { write_cntp_ctl_el0(CNTP_CTL_ENABLE | CNTP_CTL_IMASK) };
}

/// Read the Counter-timer Frequency Register
///
/// # Safety
/// Accesses ARM system register directly.
unsafe fn read_cntfrq_el0() -> u64 {
    let val: u64;
    core::arch::asm!(
        "mrs {val}, cntfrq_el0",
        val = out(reg) val,
        options(nomem, nostack, preserves_flags),
    );
    val
}

/// Read the Counter-timer Physical Count Register
///
/// # Safety
/// Accesses ARM system register directly.
unsafe fn read_cntpct_el0() -> u64 {
    let val: u64;
    core::arch::asm!(
        "mrs {val}, cntpct_el0",
        val = out(reg) val,
        options(nomem, nostack, preserves_flags),
    );
    val
}

/// Write the Counter-timer Physical Timer CompareValue Register
///
/// # Safety
/// Accesses ARM system register directly.
unsafe fn write_cntp_cval_el0(value: u64) {
    core::arch::asm!(
        "msr cntp_cval_el0, {value}",
        value = in(reg) value,
        options(nomem, nostack, preserves_flags),
    );
}

/// Write the Counter-timer Physical Timer Control Register
///
/// # Safety
/// Accesses ARM system register directly.
unsafe fn write_cntp_ctl_el0(value: u64) {
    core::arch::asm!(
        "msr cntp_ctl_el0, {value}",
        value = in(reg) value,
        options(nomem, nostack, preserves_flags),
    );
}

/// Get the timer frequency
pub fn get_frequency() -> u64 {
    TIMER_FREQUENCY.load(Ordering::Relaxed)
}

/// Get the current tick count
pub fn get_tick_count() -> u64 {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    if period == 0 {
        return 0;
    }
    // SAFETY: read_cntpct_el0 is safe to call after init
    unsafe { read_cntpct_el0() / period }
}

/// Arm the timer for the next tick
pub fn arm_next_tick() {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    // SAFETY: These register accesses are safe after init
    unsafe {
        let current_count = read_cntpct_el0();
        let compare_value = current_count.wrapping_add(period);
        write_cntp_cval_el0(compare_value);
    }
}

/// Clear timer interrupt by re-arming the timer
pub fn clear_interrupt() {
    arm_next_tick();
}
