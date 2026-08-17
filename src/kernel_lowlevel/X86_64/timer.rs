use core::sync::atomic::{AtomicU64, Ordering};

use super::{drivers, lowlevel_logic};

static TIMER_FREQUENCY: AtomicU64 = AtomicU64::new(0);
static TICK_PERIOD: AtomicU64 = AtomicU64::new(0);

pub fn driver_name() -> &'static str {
    "x86_64 invariant TSC timer"
}

pub fn init() {
    let freq = drivers::tsc_frequency();
    TIMER_FREQUENCY.store(freq, Ordering::Relaxed);
    TICK_PERIOD.store(lowlevel_logic::timer_period(freq), Ordering::Relaxed);
}

pub fn get_frequency() -> u64 {
    TIMER_FREQUENCY.load(Ordering::Relaxed)
}

pub fn interrupt_id() -> u32 {
    32
}

pub fn get_tick_count() -> u64 {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    lowlevel_logic::timer_tick_count(crate::kernel_lowlevel::cpu::read_cycle_counter(), period)
}

pub fn get_nanoseconds() -> u64 {
    lowlevel_logic::timer_counter_nanoseconds(
        crate::kernel_lowlevel::cpu::read_cycle_counter(),
        TIMER_FREQUENCY.load(Ordering::Relaxed),
    )
}

#[allow(dead_code)]
pub fn arm_next_tick() {}

pub fn clear_interrupt() {}
