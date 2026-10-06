//! Linux C PCI compatibility: software EDU device plus module_init hook.
//!
//! When `build.rs` compiles `edu.c`, `smros_linux_compat_init` comes from
//! `module_pci_driver(edu_driver)`. Architectures without a C cross compiler
//! (currently RISC-V) use the Rust fallback PCI driver below.

#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(unused_imports)]

use super::linux::{
    ioremap, iounmap, readl, register_module, register_pci_device, register_pci_driver, writel,
    LinuxModule, PciBar, PciDevice, PciDeviceId, PciDriver,
};
use super::UserDriverError;

pub const EDU_VENDOR: u32 = 0x1234;
pub const EDU_DEVICE: u32 = 0x11e8;
pub const EDU_IDENT: u32 = 0x0100_00ed;
pub const EDU_IRQ: u32 = 33;
pub const EDU_BAR_SIZE: u64 = 0x1000;
pub const EDU_NAME: &str = "edu";
pub const EDU_NODE: &str = "/pci/edu";

const EDU_REG_LIVE: usize = 0x04;

#[repr(C, align(4096))]
struct EduMmio {
    ident: u32,
    live: u32,
    factorial: u32,
    status: u32,
    irq_status: u32,
    irq_raise: u32,
    irq_ack: u32,
    _pad0: u32,
    dma_src: u32,
    dma_dst: u32,
    dma_cnt: u32,
    dma_cmd: u32,
    _tail: [u8; 4096 - 48],
}

static mut EDU_MMIO: EduMmio = EduMmio {
    ident: EDU_IDENT,
    live: 0,
    factorial: 0,
    status: 0,
    irq_status: 0,
    irq_raise: 0,
    irq_ack: 0,
    _pad0: 0,
    dma_src: 0,
    dma_dst: 0,
    dma_cnt: 0,
    dma_cmd: 0,
    _tail: [0; 4096 - 48],
};

const EDU_IDS: [PciDeviceId; 1] = [PciDeviceId {
    vendor: EDU_VENDOR,
    device: EDU_DEVICE,
}];

const EDU_DRIVER: PciDriver = PciDriver {
    name: EDU_NAME,
    id_table: &EDU_IDS,
    probe: rust_edu_probe,
    remove: Some(rust_edu_remove),
};

const EDU_MODULE: LinuxModule = LinuxModule {
    name: EDU_NAME,
    license: "GPL",
    author: "SMROS",
    description: "QEMU EDU PCI driver",
    init: rust_edu_init,
    exit: None,
};

pub fn bar0_start() -> u64 {
    core::ptr::addr_of!(EDU_MMIO) as usize as u64
}

pub fn bar0_size() -> u64 {
    EDU_BAR_SIZE
}

pub fn ident() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(EDU_MMIO.ident)) }
}

pub fn register_edu_device() -> Result<(), UserDriverError> {
    let mut bars = [PciBar::empty(); 6];
    bars[0] = PciBar {
        start: bar0_start(),
        size: EDU_BAR_SIZE,
        flags: super::linux::IORESOURCE_MEM,
    };
    register_pci_device(PciDevice {
        name: EDU_NAME,
        node_path: EDU_NODE,
        vendor: EDU_VENDOR,
        device: EDU_DEVICE,
        class: 0x00_ff_00,
        revision: 0x01,
        irq: Some(EDU_IRQ),
        bars,
        bus: 0xff,
        devfn: 0,
        subsystem_vendor: 0,
        subsystem_device: 0,
        driver: None,
    })
}

pub fn init() {
    let _ = register_edu_device();
    let _ = linux_compat_init();
}

fn linux_compat_init() -> i32 {
    #[cfg(smros_linuxcompat)]
    unsafe {
        extern "C" {
            fn smros_linux_compat_init() -> i32;
        }
        smros_linux_compat_init()
    }
    #[cfg(not(smros_linuxcompat))]
    rust_compat_init()
}

#[cfg(not(smros_linuxcompat))]
fn rust_compat_init() -> i32 {
    match register_module(EDU_MODULE) {
        Ok(()) => 0,
        Err(_) => -22,
    }
}

pub fn ready() -> bool {
    super::linux::bindings()
        .iter()
        .any(|binding| binding.driver == EDU_NAME)
}

fn rust_edu_init() -> Result<(), UserDriverError> {
    register_pci_driver(&EDU_DRIVER)
}

fn rust_edu_probe(device: &PciDevice) -> Result<(), UserDriverError> {
    let bar = device.bars[0];
    if bar.size == 0 {
        return Err(UserDriverError::NotFound);
    }
    let io = ioremap(bar.start, bar.size as usize)?;
    if readl(io) & 0xffff != 0x00ed {
        iounmap(io, bar.size as usize);
        return Err(UserDriverError::NotFound);
    }
    writel(0xa5a5_a5a5, io + EDU_REG_LIVE);
    Ok(())
}

fn rust_edu_remove(device: &PciDevice) {
    let bar = device.bars[0];
    if bar.size != 0 {
        iounmap(bar.start as usize, bar.size as usize);
    }
}
