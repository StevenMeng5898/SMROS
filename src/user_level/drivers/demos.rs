//! Linux driver demos hosted on the SMROS user-space DDK.
//!
//! `hello` is the classic misc character driver. `dummy` is a platform driver
//! that matches `smros,dummy` and exercises ioremap / IRQ / DMA helpers.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::linux::{
    dma_alloc_coherent, dma_free_coherent, free_irq, intern_name, ioremap, iounmap, misc_register,
    readl, register_module, register_platform_driver, request_irq, writel, FileOperations,
    LinuxModule, MiscDevice, PlatformDevice, PlatformDriver, IRQF_SHARED, MISC_DYNAMIC_MINOR,
};
use super::UserDriverError;

pub const HELLO_NAME: &str = "hello";
pub const HELLO_DEFAULT: &[u8] = b"Hello from SMROS Linux DDK\n";
pub const DUMMY_COMPATIBLE: &str = "smros,dummy";
pub const DUMMY_IRQ: u32 = 32;
pub const DUMMY_MAGIC: u32 = 0x534d_5253;
pub const HELLO_IOCTL_MAGIC: u32 = 0x534d_5244;

#[repr(C, align(16))]
struct DummyRegs {
    magic: u32,
    status: u32,
    scratch: u32,
    irqstat: u32,
}

static mut DUMMY_REGS: DummyRegs = DummyRegs {
    magic: DUMMY_MAGIC,
    status: 1,
    scratch: 0,
    irqstat: 0,
};
static mut HELLO_READY: bool = false;
static mut DUMMY_READY: bool = false;
static mut DUMMY_IOMEM: usize = 0;
static mut DUMMY_DMA_CPU: usize = 0;
static mut DUMMY_DMA_SIZE: usize = 0;

const DUMMY_COMPAT_TABLE: [&str; 1] = [DUMMY_COMPATIBLE];

const HELLO_FOPS: FileOperations = FileOperations {
    open: Some(hello_open),
    read: Some(hello_read),
    write: Some(hello_write),
    ioctl: Some(hello_ioctl),
    release: Some(hello_release),
};

const HELLO_MISC: MiscDevice = MiscDevice {
    name: HELLO_NAME,
    minor: MISC_DYNAMIC_MINOR,
    fops: HELLO_FOPS,
};

const DUMMY_DRIVER: PlatformDriver = PlatformDriver {
    name: "dummy",
    compatible: &DUMMY_COMPAT_TABLE,
    probe: dummy_probe,
    remove: Some(dummy_remove),
};

const HELLO_MODULE: LinuxModule = LinuxModule {
    name: "hello",
    license: "GPL",
    author: "SMROS",
    description: "Linux misc character driver demo",
    init: hello_init,
    exit: None,
};

const DUMMY_MODULE: LinuxModule = LinuxModule {
    name: "dummy",
    license: "GPL",
    author: "SMROS",
    description: "Linux platform driver demo",
    init: dummy_init,
    exit: None,
};

pub fn register() {
    let _ = register_module(HELLO_MODULE);
    let _ = register_module(DUMMY_MODULE);
}

pub fn dummy_reg_base() -> u64 {
    &raw const DUMMY_REGS as usize as u64
}

pub fn dummy_reg_size() -> u64 {
    core::mem::size_of::<DummyRegs>() as u64
}

pub fn hello_ready() -> bool {
    unsafe { HELLO_READY }
}

pub fn dummy_ready() -> bool {
    unsafe { DUMMY_READY }
}

pub fn smoke() -> bool {
    if !hello_ready() || !dummy_ready() {
        return false;
    }
    if hello_open().is_err() {
        return false;
    }
    let mut pos = 0i64;
    let mut buf = [0u8; 64];
    let Ok(read) = hello_read(&mut buf, &mut pos) else {
        return false;
    };
    if read < HELLO_DEFAULT.len() || &buf[..HELLO_DEFAULT.len()] != HELLO_DEFAULT {
        return false;
    }
    matches!(hello_ioctl(HELLO_IOCTL_MAGIC, 0), Ok(value) if value == HELLO_IOCTL_MAGIC as i64)
}

fn hello_init() -> Result<(), UserDriverError> {
    super::linux::ram_char_set(HELLO_DEFAULT);
    misc_register(HELLO_MISC)?;
    unsafe {
        HELLO_READY = true;
    }
    Ok(())
}

fn hello_open() -> Result<(), UserDriverError> {
    if hello_ready() {
        Ok(())
    } else {
        Err(UserDriverError::NotReady)
    }
}

fn hello_release() -> Result<(), UserDriverError> {
    Ok(())
}

fn hello_read(buf: &mut [u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    super::linux::ram_char_read(buf, ppos)
}

fn hello_write(buf: &[u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    super::linux::ram_char_write(buf, ppos)
}

fn hello_ioctl(cmd: u32, _arg: usize) -> Result<i64, UserDriverError> {
    if cmd == HELLO_IOCTL_MAGIC {
        Ok(HELLO_IOCTL_MAGIC as i64)
    } else {
        Err(UserDriverError::Unsupported)
    }
}

fn dummy_init() -> Result<(), UserDriverError> {
    register_platform_driver(&DUMMY_DRIVER)
}

fn dummy_probe(device: &PlatformDevice) -> Result<(), UserDriverError> {
    let mmio = device.mmio.ok_or(UserDriverError::NotFound)?;
    let iomem = ioremap(mmio.base, mmio.size as usize)?;
    if readl(iomem) != DUMMY_MAGIC {
        iounmap(iomem, mmio.size as usize);
        return Err(UserDriverError::NotFound);
    }
    writel(0x1, iomem + 4);
    request_irq(DUMMY_IRQ, dummy_irq, IRQF_SHARED, "dummy", iomem)?;
    let (cpu, _dma) = dma_alloc_coherent(64)?;
    unsafe {
        DUMMY_IOMEM = iomem;
        DUMMY_DMA_CPU = cpu;
        DUMMY_DMA_SIZE = 64;
        DUMMY_READY = true;
    }
    let _ = intern_name(device.name);
    Ok(())
}

fn dummy_remove(device: &PlatformDevice) {
    let _ = device;
    unsafe {
        if DUMMY_DMA_SIZE != 0 {
            dma_free_coherent(DUMMY_DMA_SIZE, DUMMY_DMA_CPU);
            DUMMY_DMA_SIZE = 0;
        }
        if DUMMY_IOMEM != 0 {
            free_irq(DUMMY_IRQ, DUMMY_IOMEM);
            iounmap(DUMMY_IOMEM, dummy_reg_size() as usize);
            DUMMY_IOMEM = 0;
        }
        DUMMY_READY = false;
    }
}

fn dummy_irq(_irq: u32, _data: usize) {}
