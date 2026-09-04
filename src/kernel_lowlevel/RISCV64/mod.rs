pub mod boot;
pub mod cpu;
pub mod drivers;
pub mod interrupt;
pub mod serial;
pub mod smp;
pub mod thread;
pub mod timer;

/// QEMU virt's default DRAM range, used only when firmware omits a memory node.
pub const RISCV64_RAM_FALLBACK: drivers::DeviceReg = drivers::DeviceReg {
    base: 0x8000_0000,
    size: 0x4000_0000,
};

/// Supervisor-only virtual alias used for the UART while a Linux user root is active.
/// Linux's fixed user window starts at 0x1000_0000, which is also QEMU virt's
/// physical UART address, so the device must be reachable at a non-user address.
pub const RISCV_USER_UART_ALIAS: usize = 0x4000_0000;

/// Supervisor-only alias window for QEMU virtio-MMIO transports. Linux user
/// images occupy the physical `0x1000_0000` window, so device accesses use a
/// separate virtual range while retaining the physical FDT addresses.
pub const RISCV_VIRTIO_MMIO_PHYS_BASE: usize = 0x1000_0000;
pub const RISCV_USER_VIRTIO_MMIO_ALIAS: usize = 0x4001_0000;

pub fn virtio_mmio_user_alias(base: usize) -> Option<usize> {
    let offset = base.checked_sub(RISCV_VIRTIO_MMIO_PHYS_BASE)?;
    RISCV_USER_VIRTIO_MMIO_ALIAS.checked_add(offset)
}

pub(crate) use crate::kernel_lowlevel::lowlevel_logic;
