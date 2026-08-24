#![allow(dead_code)]
//! RISC-V64 SBI timer driver.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{drivers, lowlevel_logic};

const SBI_EXT_TIME: usize = 0x5449_4d45;
const SBI_TIME_SET_TIMER: usize = 0;
const LEGACY_SBI_SET_TIMER: usize = 0;

static TIMER_FREQUENCY: AtomicU64 = AtomicU64::new(0);
static TICK_PERIOD: AtomicU64 = AtomicU64::new(0);

pub fn driver_name() -> &'static str {
    "RISC-V SBI timer"
}

pub fn init() {
    let freq = drivers::timebase_frequency();
    TIMER_FREQUENCY.store(freq, Ordering::Relaxed);
    let tick_period = lowlevel_logic::timer_period(freq);
    TICK_PERIOD.store(tick_period, Ordering::Relaxed);

    clear_pending_timer();
    arm_next_tick();
}

pub fn get_frequency() -> u64 {
    TIMER_FREQUENCY.load(Ordering::Relaxed)
}

pub fn interrupt_id() -> u32 {
    5
}

pub fn get_tick_count() -> u64 {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    lowlevel_logic::timer_tick_count(read_time(), period)
}

pub fn get_nanoseconds() -> u64 {
    lowlevel_logic::timer_counter_nanoseconds(read_time(), TIMER_FREQUENCY.load(Ordering::Relaxed))
}

pub fn arm_next_tick() {
    let period = TICK_PERIOD.load(Ordering::Relaxed);
    let compare_value = lowlevel_logic::timer_compare(read_time(), period);
    set_timer(compare_value);
}

pub fn arm_at_nanoseconds(deadline: u64) {
    let frequency = TIMER_FREQUENCY.load(Ordering::Relaxed);
    if frequency == 0 {
        return;
    }
    let current = read_time();
    let scaled = (deadline as u128).saturating_mul(frequency as u128);
    let target = scaled
        .saturating_add(999_999_999)
        / 1_000_000_000u128;
    let target = target
        .min(u64::MAX as u128)
        .max((current as u128).saturating_add(1)) as u64;
    set_timer(target);
}

pub fn clear_interrupt() {
    clear_pending_timer();
    arm_next_tick();
}

#[inline(always)]
fn read_time() -> u64 {
    crate::kernel_lowlevel::cpu::read_cycle_counter()
}

fn clear_pending_timer() {
    const SIP_STIP: usize = 1 << 5;
    unsafe {
        core::arch::asm!(
            "csrc sip, {mask}",
            mask = in(reg) SIP_STIP,
            options(nomem, nostack),
        );
    }
}

fn set_timer(value: u64) {
    let result = sbi_call(
        SBI_EXT_TIME,
        SBI_TIME_SET_TIMER,
        value as usize,
        0,
        0,
        0,
        0,
        0,
    );
    if result.error != 0 {
        let _ = sbi_call(LEGACY_SBI_SET_TIMER, value as usize, 0, 0, 0, 0, 0, 0);
    }
}

#[derive(Clone, Copy)]
struct SbiRet {
    error: isize,
    value: usize,
}

#[inline(always)]
fn sbi_call(
    extension: usize,
    function: usize,
    arg0: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> SbiRet {
    let error: isize;
    let value: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") arg0 => error,
            inlateout("a1") arg1 => value,
            in("a2") arg2,
            in("a3") arg3,
            in("a4") arg4,
            in("a5") arg5,
            in("a6") function,
            in("a7") extension,
            options(nostack),
        );
    }
    SbiRet { error, value }
}
