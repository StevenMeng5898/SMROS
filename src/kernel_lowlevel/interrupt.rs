//! ARM GIC interrupt controller driver.
//!
//! QEMU `virt,gic-version=4` exposes the GICv3 architectural CPU interface to
//! the guest. The distributor/redistributor setup below is enough for the
//! physical timer PPI used by the current scheduler tick path. The GICv2 path is
//! kept for the existing Raspberry Pi fallback descriptor.

use core::ptr::{read_volatile, write_volatile};

use super::{drivers, lowlevel_logic};

const GICD_CTLR: usize = 0x000;
const GICD_IGROUPR: usize = 0x080;
const GICD_ISENABLER: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
const GICC_IAR: usize = 0x00c;
const GICC_EOIR: usize = 0x010;

const GICR_WAKER: usize = 0x014;
const GICR_SGI_BASE: usize = 0x1_0000;
const GICR_IGROUPR0: usize = GICR_SGI_BASE + 0x080;
const GICR_ISENABLER0: usize = GICR_SGI_BASE + 0x100;
const GICR_IPRIORITYR: usize = GICR_SGI_BASE + 0x400;

const GICD_CTLR_ENABLE_G0: u32 = 1 << 0;
const GICD_CTLR_ENABLE_G1NS: u32 = 1 << 1;
const GICD_CTLR_ARE_NS: u32 = 1 << 4;
const GICC_CTLR_ENABLE: u32 = 1 << 0;
const GICR_WAKER_PROCESSOR_SLEEP: u32 = 1 << 1;
const GICR_WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;

const ICC_CTLR_EL1_EOIMODE_DROP_DEACTIVATE: u64 = 0;
const ICC_IGRPEN1_EL1_ENABLE: u64 = 1;
const ICC_PMR_ALLOW_ALL: u64 = 0xff;

const PRIORITY_HIGH: u8 = 0x50;
const SPURIOUS_INTERRUPT_ID: u32 = 1023;

fn write_reg(base: usize, size: usize, offset: usize, value: u32) {
    let Some(addr) = checked_mmio_addr(base, size, offset) else {
        return;
    };
    // SAFETY: The address is checked against the platform-provided MMIO range.
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn read_reg(base: usize, size: usize, offset: usize) -> u32 {
    let Some(addr) = checked_mmio_addr(base, size, offset) else {
        return 0;
    };
    // SAFETY: The address is checked against the platform-provided MMIO range.
    unsafe { read_volatile(addr as *const u32) }
}

fn checked_mmio_addr(base: usize, size: usize, offset: usize) -> Option<usize> {
    let addr = lowlevel_logic::mmio_addr(base, offset)?;
    if lowlevel_logic::dt_reg_contains(base, size, addr) {
        Some(addr)
    } else {
        None
    }
}

fn set_priority(base: usize, size: usize, priority_base: usize, irq: u32) {
    let priority_offset = lowlevel_logic::gic_reg_offset(priority_base, irq, 4);
    let mut priorities = read_reg(base, size, priority_offset);
    let byte_shift = lowlevel_logic::gic_byte_shift(irq);
    priorities = lowlevel_logic::gic_set_byte_field(priorities, byte_shift, PRIORITY_HIGH);
    write_reg(base, size, priority_offset, priorities);
}

fn enable_irq(base: usize, size: usize, enable_base: usize, irq: u32) {
    let enable_offset = lowlevel_logic::gic_reg_offset(enable_base, irq, 32);
    let enable_bit = lowlevel_logic::gic_enable_bit(irq);
    write_reg(base, size, enable_offset, enable_bit);
}

fn init_gicv2() {
    let gicd_base = drivers::gicd_base();
    let gicd_size = drivers::gicd_size();
    let gicc_base = drivers::gicc_base();
    let gicc_size = drivers::gicc_size();
    let timer_irq = drivers::timer_irq();

    write_reg(gicd_base, gicd_size, GICD_CTLR, GICD_CTLR_ENABLE_G0);
    write_reg(gicd_base, gicd_size, GICD_IGROUPR, 0);

    set_priority(gicd_base, gicd_size, GICD_IPRIORITYR, timer_irq);

    let target_offset = lowlevel_logic::gic_reg_offset(GICD_ITARGETSR, timer_irq, 4);
    let mut targets = read_reg(gicd_base, gicd_size, target_offset);
    let byte_shift = lowlevel_logic::gic_byte_shift(timer_irq);
    targets = lowlevel_logic::gic_set_byte_field(targets, byte_shift, 0x01);
    write_reg(gicd_base, gicd_size, target_offset, targets);

    enable_irq(gicd_base, gicd_size, GICD_ISENABLER, timer_irq);

    write_reg(gicc_base, gicc_size, GICC_CTLR, GICC_CTLR_ENABLE);
    write_reg(gicc_base, gicc_size, GICC_PMR, 0xff);
    write_reg(gicc_base, gicc_size, GICC_BPR, 0);
}

fn init_gicv3_v4() {
    let gicd_base = drivers::gicd_base();
    let gicd_size = drivers::gicd_size();
    let gicr_base = drivers::gicr_base();
    let gicr_size = drivers::gicr_size();
    let timer_irq = drivers::timer_irq();

    write_reg(gicd_base, gicd_size, GICD_CTLR, 0);
    write_reg(
        gicd_base,
        gicd_size,
        GICD_CTLR,
        GICD_CTLR_ARE_NS | GICD_CTLR_ENABLE_G1NS,
    );

    wake_redistributor(gicr_base, gicr_size);

    if timer_irq < 32 {
        write_reg(gicr_base, gicr_size, GICR_IGROUPR0, 0xffff_ffff);
        set_priority(gicr_base, gicr_size, GICR_IPRIORITYR, timer_irq);
        enable_irq(gicr_base, gicr_size, GICR_ISENABLER0, timer_irq);
    } else {
        set_priority(gicd_base, gicd_size, GICD_IPRIORITYR, timer_irq);
        enable_irq(gicd_base, gicd_size, GICD_ISENABLER, timer_irq);
    }

    write_icc_pmr_el1(ICC_PMR_ALLOW_ALL);
    write_icc_bpr1_el1(0);
    write_icc_ctlr_el1(ICC_CTLR_EL1_EOIMODE_DROP_DEACTIVATE);
    write_icc_igrpen1_el1(ICC_IGRPEN1_EL1_ENABLE);
    isb();
}

fn wake_redistributor(gicr_base: usize, gicr_size: usize) {
    let mut waker = read_reg(gicr_base, gicr_size, GICR_WAKER);
    waker &= !GICR_WAKER_PROCESSOR_SLEEP;
    write_reg(gicr_base, gicr_size, GICR_WAKER, waker);

    for _ in 0..100_000 {
        if read_reg(gicr_base, gicr_size, GICR_WAKER) & GICR_WAKER_CHILDREN_ASLEEP == 0 {
            break;
        }
        core::hint::spin_loop();
    }
}

/// Initialize the platform interrupt controller.
pub fn init() {
    match drivers::gic_version() {
        drivers::GicVersion::GicV2 => init_gicv2(),
        drivers::GicVersion::GicV3V4 => init_gicv3_v4(),
    }
}

/// Enable the timer interrupt.
pub fn enable_timer_interrupt() {
    // The timer PPI is enabled during GIC initialization. The timer driver
    // programs CNTP_CTL_EL0 and the compare value.
}

/// Acknowledge an interrupt and return the interrupt ID.
pub fn acknowledge_interrupt() -> u32 {
    match drivers::gic_version() {
        drivers::GicVersion::GicV2 => {
            let iar = read_reg(drivers::gicc_base(), drivers::gicc_size(), GICC_IAR);
            lowlevel_logic::gic_interrupt_id(iar)
        }
        drivers::GicVersion::GicV3V4 => lowlevel_logic::gic_interrupt_id(read_icc_iar1_el1()),
    }
}

/// Signal end of interrupt.
pub fn end_of_interrupt(interrupt_id: u32) {
    if interrupt_id == SPURIOUS_INTERRUPT_ID {
        return;
    }

    match drivers::gic_version() {
        drivers::GicVersion::GicV2 => {
            write_reg(drivers::gicc_base(), drivers::gicc_size(), GICC_EOIR, interrupt_id);
        }
        drivers::GicVersion::GicV3V4 => write_icc_eoir1_el1(interrupt_id),
    }
}

fn isb() {
    // SAFETY: ISB is a local CPU synchronization barrier.
    unsafe {
        core::arch::asm!("isb", options(nomem, nostack, preserves_flags));
    }
}

fn read_icc_iar1_el1() -> u32 {
    let value: u64;
    // SAFETY: ICC_IAR1_EL1 is the architectural GICv3/v4 interrupt acknowledge register.
    unsafe {
        core::arch::asm!(
            "mrs {value}, ICC_IAR1_EL1",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value as u32
}

fn write_icc_eoir1_el1(interrupt_id: u32) {
    let value = interrupt_id as u64;
    // SAFETY: EOIR writes complete interrupt processing for the acknowledged ID.
    unsafe {
        core::arch::asm!(
            "msr ICC_EOIR1_EL1, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn write_icc_pmr_el1(value: u64) {
    // SAFETY: PMR masks interrupt priorities for the current CPU interface.
    unsafe {
        core::arch::asm!(
            "msr ICC_PMR_EL1, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn write_icc_bpr1_el1(value: u64) {
    // SAFETY: BPR1 controls group-1 interrupt preemption for the current CPU.
    unsafe {
        core::arch::asm!(
            "msr ICC_BPR1_EL1, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn write_icc_ctlr_el1(value: u64) {
    // SAFETY: CTLR configures the current CPU's architectural GIC interface.
    unsafe {
        core::arch::asm!(
            "msr ICC_CTLR_EL1, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn write_icc_igrpen1_el1(value: u64) {
    // SAFETY: IGRPEN1 enables non-secure group-1 interrupt signaling locally.
    unsafe {
        core::arch::asm!(
            "msr ICC_IGRPEN1_EL1, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}
