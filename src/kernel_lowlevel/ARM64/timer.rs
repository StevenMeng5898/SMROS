#![allow(dead_code)]
//! ARM Generic Timer Driver
//!
//! This module provides access to the ARM Generic Timer (CNTFRQ, CNTPCT, CNTP_TVAL, etc.)
//! which is used for system timing and scheduler ticks.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{cpu, drivers, lowlevel_logic};

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
/// The next periodic compare value. Precision sleepers may program an earlier
/// compare, but they must never postpone this scheduler tick.
static NEXT_PERIODIC_COMPARE: AtomicU64 = AtomicU64::new(0);
/// The earliest outstanding precision compare. Periodic IRQs preserve this
/// deadline until the precision waiter has actually expired.
static PRECISION_COMPARE: AtomicU64 = AtomicU64::new(0);
/// The compare value currently programmed in CNTP_CVAL_EL0. Keeping this
/// separately from the periodic deadline prevents a later precision waiter
/// from postponing an earlier one already armed in the hardware timer.
static PROGRAMMED_COMPARE: AtomicU64 = AtomicU64::new(0);

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

/// Read the compare value currently programmed in the CPU, not the cached copy.
fn read_cntp_cval_el0() -> u64 {
    let val: u64;
    // SAFETY: CNTP_CVAL_EL0 is the architectural physical timer compare register.
    unsafe {
        core::arch::asm!(
            "mrs {val}, cntp_cval_el0",
            val = out(reg) val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Write the Counter-timer Physical Timer CompareValue Register
fn write_cntp_cval_el0(value: u64) {
    // SAFETY: Writing CNTP_CVAL_EL0 sets the timer compare value.
    // ISB is required so a later WFI observes the new compare; ARM64 TCG
    // otherwise keeps the previous deadline and never wakes cooperative waits.
    unsafe {
        core::arch::asm!(
            "msr cntp_cval_el0, {value}",
            "isb",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Write CNTP_TVAL_EL0. Hardware sets CVAL to CNTPCT + value atomically, so a
/// host scheduling gap cannot leave an absolute compare already in the past.
fn write_cntp_tval_el0(value: u64) {
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {value}",
            "isb",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Write the Counter-timer Physical Timer Control Register
fn write_cntp_ctl_el0(value: u64) {
    // SAFETY: Writing CNTP_CTL_EL0 controls the physical timer.
    // ISB makes ENABLE/IMASK visible before a later WFI.
    unsafe {
        core::arch::asm!(
            "msr cntp_ctl_el0, {value}",
            "isb",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Initialize the ARM Generic Timer
pub fn driver_name() -> &'static str {
    "ARM Generic Timer"
}

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
    NEXT_PERIODIC_COMPARE.store(compare_value, Ordering::Release);
    PROGRAMMED_COMPARE.store(compare_value, Ordering::Release);
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

/// Get the current monotonic time in nanoseconds.
pub fn get_nanoseconds() -> u64 {
    // Keep the PC-relative address materialization for TIMER_FREQUENCY and the
    // dependent conversion together. An IRQ may use x9 as a scratch register;
    // masking it here prevents an interrupt from landing between ADRP/LDR and
    // corrupting the address while the syscall is still in flight.
    let interrupt_state = cpu::mask_interrupts();
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    let counter = read_cntpct_el0();
    let frequency = TIMER_FREQUENCY.load(Ordering::Relaxed);
    let nanoseconds = lowlevel_logic::timer_counter_nanoseconds(counter, frequency);
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    cpu::restore_interrupts(interrupt_state);
    nanoseconds
}

/// Arm the timer for the next tick
pub fn arm_next_tick() {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    let current_count = read_cntpct_el0();
    let periodic_compare = lowlevel_logic::timer_compare(current_count, period);
    NEXT_PERIODIC_COMPARE.store(periodic_compare, Ordering::Release);
    let precision_compare = PRECISION_COMPARE.load(Ordering::Acquire);
    let precision_compare = if precision_compare > current_count {
        precision_compare
    } else {
        PRECISION_COMPARE.store(0, Ordering::Release);
        0
    };
    let compare_value =
        lowlevel_logic::timer_program_compare(periodic_compare, precision_compare, 0);
    PROGRAMMED_COMPARE.store(compare_value, Ordering::Release);
    write_cntp_cval_el0(compare_value);
    ensure_hardware_compare_can_wake();
}

fn live_compare(value: u64, current_count: u64) -> u64 {
    // A compare at or behind CNTPCT has already fired. GICv3/TCG will not
    // raise a new edge until CVAL is rewritten to a future count, so a later
    // WFI would sleep forever on the stale value.
    if value > current_count {
        value
    } else {
        0
    }
}

/// Arm the physical timer for an absolute monotonic deadline.
pub fn arm_at_nanoseconds(deadline: u64) {
    let frequency = TIMER_FREQUENCY.load(Ordering::Relaxed);
    if frequency == 0 {
        return;
    }
    let current_count = read_cntpct_el0();
    let scaled = (deadline as u128).saturating_mul(frequency as u128);
    let target_count = scaled.saturating_add(999_999_999) / 1_000_000_000u128;
    let target_count = target_count
        .min(u64::MAX as u128)
        .max((current_count as u128).saturating_add(1)) as u64;
    let existing_precision = PRECISION_COMPARE.load(Ordering::Acquire);
    let precision_compare = if existing_precision > current_count {
        existing_precision.min(target_count)
    } else {
        target_count
    };
    PRECISION_COMPARE.store(precision_compare, Ordering::Release);
    let periodic_compare =
        live_compare(NEXT_PERIODIC_COMPARE.load(Ordering::Acquire), current_count);
    let armed_compare = PROGRAMMED_COMPARE.load(Ordering::Acquire);
    let programmed_compare = lowlevel_logic::timer_program_compare(
        periodic_compare,
        live_compare(armed_compare, current_count),
        target_count,
    );
    if programmed_compare != armed_compare || programmed_compare <= current_count {
        write_cntp_cval_el0(programmed_compare);
        PROGRAMMED_COMPARE.store(programmed_compare, Ordering::Release);
    }
    // The cache can claim a future compare while the register itself is still
    // in the past (a skipped write, or CNTPCT advancing past the value we just
    // computed). Re-read the hardware register and force a future edge.
    ensure_hardware_compare_can_wake();
}

/// `(CNTPCT, hardware CNTP_CVAL)` for campaign timeout diagnostics.
pub fn debug_compare_snapshot() -> (u64, u64) {
    (read_cntpct_el0(), read_cntp_cval_el0())
}

/// Arm a near periodic compare relative to the current counter.
///
/// Also forces the physical timer enabled and unmasked. A compare that is
/// already behind CNTPCT does not raise another GICv3 edge, and a disabled
/// or masked control register never raises one at all.
pub fn arm_periodic_from_now() {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    let period = if period == 0 {
        1
    } else {
        period.min(i32::MAX as u64)
    };
    write_cntp_ctl_el0(lowlevel_logic::timer_ctl(CNTP_CTL_ENABLE, CNTP_CTL_IMASK));
    write_cntp_tval_el0(period);
    let now = read_cntpct_el0();
    let mut hardware = read_cntp_cval_el0();
    let precision = PRECISION_COMPARE.load(Ordering::Acquire);
    if precision > now && (hardware <= now || precision < hardware) {
        write_cntp_cval_el0(precision);
        hardware = precision;
    }
    if hardware <= now {
        write_cntp_tval_el0(period);
        hardware = read_cntp_cval_el0();
    }
    NEXT_PERIODIC_COMPARE.store(hardware, Ordering::Release);
    PROGRAMMED_COMPARE.store(hardware, Ordering::Release);
}

/// Make sure the next WFI has a timer edge to wake on.
///
/// AArch64 TCG/GICv3 will not re-raise the physical timer from a compare that
/// is already <= CNTPCT. Callers that trust `PROGRAMMED_COMPARE` can skip the
/// MSR and then sleep forever. This reads the register itself.
pub fn ensure_hardware_compare_can_wake() {
    let now = read_cntpct_el0();
    let hardware = read_cntp_cval_el0();
    if !lowlevel_logic::timer_compare_needs_rearm(hardware, now) {
        return;
    }
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    let next = lowlevel_logic::timer_rearm_compare(now, period);
    NEXT_PERIODIC_COMPARE.store(next, Ordering::Release);
    PROGRAMMED_COMPARE.store(next, Ordering::Release);
    write_cntp_cval_el0(next);
}

/// Clear timer interrupt by re-arming the timer
pub fn clear_interrupt() {
    arm_next_tick();
}
