//! RISC-V64 supervisor interrupt control.

const SIE_STIE: usize = 1 << 5;

pub fn init() {
    unsafe {
        core::arch::asm!(
            "csrs sie, {mask}",
            mask = in(reg) SIE_STIE,
            options(nomem, nostack),
        );
    }
}

pub fn enable_timer_interrupt() {
    unsafe {
        core::arch::asm!(
            "csrs sie, {mask}",
            mask = in(reg) SIE_STIE,
            options(nomem, nostack),
        );
    }
}

pub fn acknowledge_interrupt() -> u32 {
    crate::kernel_lowlevel::timer::interrupt_id()
}

pub fn end_of_interrupt(_interrupt_id: u32) {}
