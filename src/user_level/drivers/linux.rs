//! Linux-shaped user-space driver model for the SMROS DDK.
//!
//! This is not a Linux kernel port. It implements the device/driver/bus
//! matching flow (`module_init`, `platform_driver`, `virtio_driver`,
//! `miscdevice`, `ioremap`, `request_irq`, `dma_alloc_coherent`) so Linux
//! driver demos can bind in SMROS user-level code.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

use super::linux_logic;
use super::pci::VirtioPciTransport;
use super::{UserDeviceKind, UserDeviceReg, UserDriverBinding, UserDriverError};

#[path = "linux_pci.rs"]
mod linux_pci;

pub const DDK_API_VERSION: u32 = 1;
pub const PCI_ANY_ID: u32 = 0xffff_ffff;
pub const VIRTIO_ID_ANY: u32 = 0xffff_ffff;
pub const VIRTIO_ID_NET: u32 = 1;
pub const VIRTIO_ID_BLOCK: u32 = 2;
pub const VIRTIO_MAGIC: u32 = 0x7472_6976;
pub const VIRTIO_VENDOR_QEMU: u32 = 0x554d_4551;
pub const MISC_DYNAMIC_MINOR: u32 = 255;
pub const CHRDEV_MINOR_MAX: u32 = 256;
pub const IRQF_SHARED: u32 = 0x80;
pub const IRQ_MAX: u32 = 1024;
pub const IOREMAP_MAX: usize = 0x1000_0000;
pub const DMA_ALLOC_MAX: usize = 0x0001_0000;
pub const BUS_PLATFORM: u32 = 1;
pub const BUS_VIRTIO: u32 = 2;
pub const BUS_PCI: u32 = 3;
pub const IORESOURCE_IO: u32 = 0x0000_0100;
pub const IORESOURCE_MEM: u32 = 0x0000_0200;
const KMALLOC_MAX: usize = 4096;
pub const VIRTIO_MMIO_REG_MAGIC: usize = 0x000;
pub const VIRTIO_MMIO_REG_DEVICE_ID: usize = 0x008;
pub const VIRTIO_MMIO_REG_VENDOR_ID: usize = 0x00c;

const DMA_POOL_SIZE: usize = 16 * 1024;
const NAME_ARENA_SIZE: usize = 2048;
const RAM_CHAR_BYTES: usize = 128;
const MAX_C_SLOTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusKind {
    Platform,
    Virtio,
    Pci,
}

impl BusKind {
    pub fn as_u32(self) -> u32 {
        match self {
            BusKind::Platform => BUS_PLATFORM,
            BusKind::Virtio => BUS_VIRTIO,
            BusKind::Pci => BUS_PCI,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BusKind::Platform => "platform",
            BusKind::Virtio => "virtio",
            BusKind::Pci => "pci",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtioTransport {
    Mmio,
    Pci,
}

#[derive(Clone, Copy, Debug)]
pub struct LinuxModule {
    pub name: &'static str,
    pub license: &'static str,
    pub author: &'static str,
    pub description: &'static str,
    pub init: fn() -> Result<(), UserDriverError>,
    pub exit: Option<fn()>,
}

#[derive(Clone, Copy, Debug)]
pub struct PlatformDevice {
    pub name: &'static str,
    pub node_path: &'static str,
    pub compatible: &'static str,
    pub mmio: Option<UserDeviceReg>,
    pub irq: Option<u32>,
    pub driver: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct PlatformDriver {
    pub name: &'static str,
    pub compatible: &'static [&'static str],
    pub probe: fn(&PlatformDevice) -> Result<(), UserDriverError>,
    pub remove: Option<fn(&PlatformDevice)>,
}

#[derive(Clone, Copy, Debug)]
pub struct VirtioDeviceId {
    pub device: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct VirtioDevice {
    pub name: &'static str,
    pub node_path: &'static str,
    pub device_id: u32,
    pub transport: VirtioTransport,
    pub mmio_base: usize,
    pub mmio_size: u64,
    pub irq: Option<u32>,
    pub pci: Option<VirtioPciTransport>,
    pub driver: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct VirtioDriver {
    pub name: &'static str,
    pub id_table: &'static [VirtioDeviceId],
    pub probe: fn(&VirtioDevice) -> Result<(), UserDriverError>,
    pub remove: Option<fn(&VirtioDevice)>,
}

#[derive(Clone, Copy, Debug)]
pub struct PciDeviceId {
    pub vendor: u32,
    pub device: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciBar {
    pub start: u64,
    pub size: u64,
    pub flags: u32,
}

impl PciBar {
    pub const fn empty() -> Self {
        Self {
            start: 0,
            size: 0,
            flags: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PciDevice {
    pub name: &'static str,
    pub node_path: &'static str,
    pub vendor: u32,
    pub device: u32,
    pub class: u32,
    pub revision: u8,
    pub irq: Option<u32>,
    pub bars: [PciBar; 6],
    pub bus: u8,
    pub devfn: u8,
    pub subsystem_vendor: u32,
    pub subsystem_device: u32,
    pub driver: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct PciDriver {
    pub name: &'static str,
    pub id_table: &'static [PciDeviceId],
    pub probe: fn(&PciDevice) -> Result<(), UserDriverError>,
    pub remove: Option<fn(&PciDevice)>,
}

#[derive(Clone, Copy, Debug)]
pub struct FileOperations {
    pub open: Option<fn() -> Result<(), UserDriverError>>,
    pub read: Option<fn(&mut [u8], &mut i64) -> Result<usize, UserDriverError>>,
    pub write: Option<fn(&[u8], &mut i64) -> Result<usize, UserDriverError>>,
    pub ioctl: Option<fn(u32, usize) -> Result<i64, UserDriverError>>,
    pub release: Option<fn() -> Result<(), UserDriverError>>,
}

#[derive(Clone, Copy, Debug)]
pub struct MiscDevice {
    pub name: &'static str,
    pub minor: u32,
    pub fops: FileOperations,
}

#[derive(Clone, Copy, Debug)]
pub struct ModuleInfo {
    pub name: &'static str,
    pub license: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct BusInfo {
    pub name: &'static str,
    pub devices: usize,
    pub drivers: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ChrdevInfo {
    pub name: &'static str,
    pub minor: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct LinuxDdkStats {
    pub api_version: u32,
    pub modules: usize,
    pub buses: usize,
    pub platform_devices: usize,
    pub virtio_devices: usize,
    pub pci_devices: usize,
    pub platform_drivers: usize,
    pub virtio_drivers: usize,
    pub pci_drivers: usize,
    pub chrdevs: usize,
    pub irqs: usize,
    pub iomaps: usize,
}

struct IoMap {
    phys: u64,
    virt: usize,
    size: usize,
}

struct IrqAction {
    irq: u32,
    flags: u32,
    name: &'static str,
    handler: fn(u32, usize),
    data: usize,
}

struct Chrdev {
    name: &'static str,
    minor: u32,
    fops: FileOperations,
}

struct Registry {
    initialized: bool,
    next_minor: u32,
    modules: Vec<ModuleInfo>,
    platform_devices: Vec<PlatformDevice>,
    platform_drivers: Vec<&'static PlatformDriver>,
    virtio_devices: Vec<VirtioDevice>,
    virtio_drivers: Vec<&'static VirtioDriver>,
    pci_devices: Vec<PciDevice>,
    pci_drivers: Vec<&'static PciDriver>,
    chrdevs: Vec<Chrdev>,
    iomaps: Vec<IoMap>,
    irqs: Vec<IrqAction>,
    bindings: Vec<UserDriverBinding>,
    dma_used: usize,
    name_used: usize,
}

#[repr(C, align(4096))]
struct DmaPool {
    bytes: [u8; DMA_POOL_SIZE],
}

struct NameArena {
    bytes: [u8; NAME_ARENA_SIZE],
}

impl Registry {
    const fn new() -> Self {
        Self {
            initialized: false,
            next_minor: 0,
            modules: Vec::new(),
            platform_devices: Vec::new(),
            platform_drivers: Vec::new(),
            virtio_devices: Vec::new(),
            virtio_drivers: Vec::new(),
            pci_devices: Vec::new(),
            pci_drivers: Vec::new(),
            chrdevs: Vec::new(),
            iomaps: Vec::new(),
            irqs: Vec::new(),
            bindings: Vec::new(),
            dma_used: 0,
            name_used: 0,
        }
    }
}

static mut REGISTRY: Registry = Registry::new();
static mut DMA_POOL: DmaPool = DmaPool {
    bytes: [0; DMA_POOL_SIZE],
};
static mut NAME_ARENA: NameArena = NameArena {
    bytes: [0; NAME_ARENA_SIZE],
};
static mut RAM_CHAR: [u8; RAM_CHAR_BYTES] = [0; RAM_CHAR_BYTES];
static mut RAM_CHAR_LEN: usize = 0;

fn registry() -> &'static mut Registry {
    unsafe { &mut REGISTRY }
}

pub fn init() {
    let reg = registry();
    if reg.initialized {
        return;
    }
    reset();
    registry().initialized = true;
}

pub fn reset() {
    let reg = registry();
    *reg = Registry::new();
    unsafe {
        DMA_POOL.bytes = [0; DMA_POOL_SIZE];
        NAME_ARENA.bytes = [0; NAME_ARENA_SIZE];
        RAM_CHAR = [0; RAM_CHAR_BYTES];
        RAM_CHAR_LEN = 0;
    }
    linux_pci::reset();
}

pub fn register_module(module: LinuxModule) -> Result<(), UserDriverError> {
    init();
    if !linux_logic::module_license_ok(module.license.len()) || module.name.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    let reg = registry();
    if reg.modules.iter().any(|entry| entry.name == module.name) {
        return Err(UserDriverError::Exists);
    }
    (module.init)()?;
    reg.modules.push(ModuleInfo {
        name: module.name,
        license: module.license,
        description: module.description,
    });
    let _ = module.author;
    let _ = module.exit;
    Ok(())
}

pub fn register_platform_driver(driver: &'static PlatformDriver) -> Result<(), UserDriverError> {
    init();
    if driver.name.is_empty() || driver.compatible.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    registry().platform_drivers.push(driver);
    Ok(())
}

pub fn register_platform_device(device: PlatformDevice) -> Result<(), UserDriverError> {
    init();
    if device.name.is_empty() || device.compatible.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    registry().platform_devices.push(device);
    Ok(())
}

pub fn register_virtio_driver(driver: &'static VirtioDriver) -> Result<(), UserDriverError> {
    init();
    if driver.name.is_empty() || driver.id_table.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    registry().virtio_drivers.push(driver);
    Ok(())
}

pub fn register_virtio_mmio_device(
    name: &'static str,
    node_path: &'static str,
    base: usize,
    size: u64,
    irq: Option<u32>,
) -> Result<u32, UserDriverError> {
    init();
    let Some(device_id) = virtio_mmio_device_id(base) else {
        return Err(UserDriverError::NotFound);
    };
    registry().virtio_devices.push(VirtioDevice {
        name,
        node_path,
        device_id,
        transport: VirtioTransport::Mmio,
        mmio_base: base,
        mmio_size: size,
        irq,
        pci: None,
        driver: None,
    });
    Ok(device_id)
}

pub fn register_virtio_pci_device(
    name: &'static str,
    node_path: &'static str,
    device_id: u32,
    transport: VirtioPciTransport,
) -> Result<(), UserDriverError> {
    init();
    if !linux_logic::virtio_device_present(device_id) {
        return Err(UserDriverError::NotFound);
    }
    registry().virtio_devices.push(VirtioDevice {
        name,
        node_path,
        device_id,
        transport: VirtioTransport::Pci,
        mmio_base: transport.common_base,
        mmio_size: 0,
        irq: None,
        pci: Some(transport),
        driver: None,
    });
    Ok(())
}

pub fn register_pci_driver(driver: &'static PciDriver) -> Result<(), UserDriverError> {
    init();
    if driver.name.is_empty() || driver.id_table.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    registry().pci_drivers.push(driver);
    Ok(())
}

pub fn register_pci_device(device: PciDevice) -> Result<(), UserDriverError> {
    init();
    if device.name.is_empty() {
        return Err(UserDriverError::Unsupported);
    }
    registry().pci_devices.push(device);
    Ok(())
}

pub fn attach() {
    init();
    attach_platform();
    attach_virtio();
    attach_pci();
}

fn attach_platform() {
    let driver_count = registry().platform_drivers.len();
    let device_count = registry().platform_devices.len();
    for device_index in 0..device_count {
        if registry().platform_devices[device_index].driver.is_some() {
            continue;
        }
        for driver_index in 0..driver_count {
            let driver = registry().platform_drivers[driver_index];
            if !platform_match(&registry().platform_devices[device_index], driver) {
                continue;
            }
            let device = registry().platform_devices[device_index];
            if (driver.probe)(&device).is_ok() {
                registry().platform_devices[device_index].driver = Some(driver.name);
                push_binding(
                    device.node_path,
                    driver.name,
                    device.name,
                    UserDeviceKind::Platform,
                    0,
                    0,
                    0,
                    [0; 6],
                );
                break;
            }
        }
    }
}

fn attach_virtio() {
    let driver_count = registry().virtio_drivers.len();
    let device_count = registry().virtio_devices.len();
    for device_index in 0..device_count {
        if registry().virtio_devices[device_index].driver.is_some() {
            continue;
        }
        for driver_index in 0..driver_count {
            let driver = registry().virtio_drivers[driver_index];
            if !virtio_match(&registry().virtio_devices[device_index], driver) {
                continue;
            }
            let device = registry().virtio_devices[device_index];
            if (driver.probe)(&device).is_ok() {
                registry().virtio_devices[device_index].driver = Some(driver.name);
                let kind = if device.device_id == VIRTIO_ID_BLOCK {
                    UserDeviceKind::Block
                } else if device.device_id == VIRTIO_ID_NET {
                    UserDeviceKind::Network
                } else {
                    UserDeviceKind::VirtioMmio
                };
                push_binding(
                    device.node_path,
                    driver.name,
                    device.name,
                    kind,
                    0,
                    0,
                    0,
                    [0; 6],
                );
                break;
            }
        }
    }
}

fn attach_pci() {
    let driver_count = registry().pci_drivers.len();
    let device_count = registry().pci_devices.len();
    for device_index in 0..device_count {
        if registry().pci_devices[device_index].driver.is_some() {
            continue;
        }
        for driver_index in 0..driver_count {
            let driver = registry().pci_drivers[driver_index];
            if !pci_match(&registry().pci_devices[device_index], driver) {
                continue;
            }
            let device = registry().pci_devices[device_index];
            if (driver.probe)(&device).is_ok() {
                registry().pci_devices[device_index].driver = Some(driver.name);
                push_binding(
                    device.node_path,
                    driver.name,
                    device.name,
                    UserDeviceKind::Pci,
                    0,
                    0,
                    0,
                    [0; 6],
                );
                break;
            }
        }
    }
    linux_pci::attach();
}

fn platform_match(device: &PlatformDevice, driver: &PlatformDriver) -> bool {
    driver.compatible.iter().any(|compatible| {
        linux_logic::of_match(device.compatible == *compatible, compatible.is_empty())
    })
}

fn virtio_match(device: &VirtioDevice, driver: &VirtioDriver) -> bool {
    if !linux_logic::virtio_device_present(device.device_id) {
        return false;
    }
    driver
        .id_table
        .iter()
        .any(|id| linux_logic::virtio_id_match(device.device_id, id.device, VIRTIO_ID_ANY))
}

fn pci_match(device: &PciDevice, driver: &PciDriver) -> bool {
    driver.id_table.iter().any(|id| {
        if linux_logic::id_table_end(id.vendor, id.device) {
            false
        } else {
            linux_logic::pci_id_match(
                device.vendor,
                device.device,
                id.vendor,
                id.device,
                PCI_ANY_ID,
            )
        }
    })
}

fn push_binding(
    node_path: &'static str,
    driver: &'static str,
    device_name: &'static str,
    kind: UserDeviceKind,
    block_size: usize,
    block_count: usize,
    mtu: usize,
    mac: [u8; 6],
) {
    registry().bindings.push(UserDriverBinding {
        node_path,
        driver,
        device_name,
        kind,
        block_size,
        block_count,
        mtu,
        mac,
    });
}

pub fn misc_register(device: MiscDevice) -> Result<u32, UserDriverError> {
    init();
    if device.name.is_empty()
        || !linux_logic::chrdev_minor_valid(device.minor, CHRDEV_MINOR_MAX, MISC_DYNAMIC_MINOR)
    {
        return Err(UserDriverError::Unsupported);
    }
    let reg = registry();
    if reg.chrdevs.iter().any(|entry| entry.name == device.name) {
        return Err(UserDriverError::Exists);
    }
    let minor = if device.minor == MISC_DYNAMIC_MINOR {
        let assigned = reg.next_minor;
        if assigned >= CHRDEV_MINOR_MAX {
            return Err(UserDriverError::Busy);
        }
        reg.next_minor += 1;
        assigned
    } else {
        if reg.chrdevs.iter().any(|entry| entry.minor == device.minor) {
            return Err(UserDriverError::Busy);
        }
        device.minor
    };
    reg.chrdevs.push(Chrdev {
        name: device.name,
        minor,
        fops: device.fops,
    });
    push_binding(
        "/dev",
        device.name,
        device.name,
        UserDeviceKind::Char,
        0,
        0,
        0,
        [0; 6],
    );
    Ok(minor)
}

pub fn chrdev_open(name: &str) -> Result<(), UserDriverError> {
    let fops = chrdev_fops(name)?;
    if let Some(open) = fops.open {
        open()
    } else {
        Ok(())
    }
}

pub fn chrdev_read(name: &str, buf: &mut [u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    let fops = chrdev_fops(name)?;
    let Some(read) = fops.read else {
        return Err(UserDriverError::Unsupported);
    };
    read(buf, ppos)
}

pub fn chrdev_write(name: &str, buf: &[u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    let fops = chrdev_fops(name)?;
    let Some(write) = fops.write else {
        return Err(UserDriverError::Unsupported);
    };
    write(buf, ppos)
}

pub fn chrdev_ioctl(name: &str, cmd: u32, arg: usize) -> Result<i64, UserDriverError> {
    let fops = chrdev_fops(name)?;
    let Some(ioctl) = fops.ioctl else {
        return Err(UserDriverError::Unsupported);
    };
    ioctl(cmd, arg)
}

fn chrdev_fops(name: &str) -> Result<FileOperations, UserDriverError> {
    init();
    registry()
        .chrdevs
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.fops)
        .ok_or(UserDriverError::NotFound)
}

pub fn ioremap(phys: u64, size: usize) -> Result<usize, UserDriverError> {
    init();
    if !linux_logic::ioremap_len_valid(size, IOREMAP_MAX) {
        return Err(UserDriverError::OutOfRange);
    }
    let virt = map_phys(phys as usize);
    registry().iomaps.push(IoMap { phys, virt, size });
    Ok(virt)
}

pub fn iounmap(addr: usize, size: usize) {
    let reg = registry();
    if let Some(index) = reg
        .iomaps
        .iter()
        .position(|entry| entry.virt == addr && (size == 0 || entry.size == size))
    {
        reg.iomaps.swap_remove(index);
    }
}

pub fn readb(addr: usize) -> u8 {
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

pub fn writeb(value: u8, addr: usize) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

pub fn readw(addr: usize) -> u16 {
    unsafe { core::ptr::read_volatile(addr as *const u16) }
}

pub fn writew(value: u16, addr: usize) {
    unsafe { core::ptr::write_volatile(addr as *mut u16, value) }
}

pub fn readl(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

pub fn writel(value: u32, addr: usize) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

pub fn request_irq(
    irq: u32,
    handler: fn(u32, usize),
    flags: u32,
    name: &'static str,
    data: usize,
) -> Result<(), UserDriverError> {
    init();
    if !linux_logic::irq_number_valid(irq, IRQ_MAX) || name.is_empty() {
        return Err(UserDriverError::BadIrq);
    }
    let reg = registry();
    if let Some(existing) = reg.irqs.iter().find(|action| action.irq == irq) {
        if !linux_logic::irq_shared(flags, IRQF_SHARED)
            || !linux_logic::irq_shared(existing.flags, IRQF_SHARED)
        {
            return Err(UserDriverError::Busy);
        }
    }
    reg.irqs.push(IrqAction {
        irq,
        flags,
        name,
        handler,
        data,
    });
    Ok(())
}

pub fn free_irq(irq: u32, data: usize) {
    let reg = registry();
    if let Some(index) = reg
        .irqs
        .iter()
        .position(|action| action.irq == irq && action.data == data)
    {
        reg.irqs.swap_remove(index);
    }
}

pub fn dma_alloc_coherent(size: usize) -> Result<(usize, u64), UserDriverError> {
    init();
    if !linux_logic::dma_size_valid(size, DMA_ALLOC_MAX) {
        return Err(UserDriverError::OutOfRange);
    }
    let aligned = size.checked_add(15).ok_or(UserDriverError::OutOfRange)? & !15usize;
    let reg = registry();
    let start = reg.dma_used;
    let end = start
        .checked_add(aligned)
        .ok_or(UserDriverError::NoMemory)?;
    if end > DMA_POOL_SIZE {
        return Err(UserDriverError::NoMemory);
    }
    reg.dma_used = end;
    let cpu = unsafe { DMA_POOL.bytes.as_mut_ptr() as usize + start };
    Ok((cpu, cpu as u64))
}

pub fn dma_free_coherent(size: usize, cpu_addr: usize) {
    let aligned = size.checked_add(15).map(|value| value & !15usize);
    let Some(aligned) = aligned else {
        return;
    };
    let pool = unsafe { DMA_POOL.bytes.as_mut_ptr() as usize };
    let reg = registry();
    if cpu_addr.checked_add(aligned) == Some(pool + reg.dma_used) {
        reg.dma_used = cpu_addr.saturating_sub(pool);
    }
}

pub fn virtio_mmio_device_id(base: usize) -> Option<u32> {
    let mapped = map_phys(base);
    let magic = readl(mapped + VIRTIO_MMIO_REG_MAGIC);
    let device_id = readl(mapped + VIRTIO_MMIO_REG_DEVICE_ID);
    let vendor = readl(mapped + VIRTIO_MMIO_REG_VENDOR_ID);
    if magic != VIRTIO_MAGIC {
        return None;
    }
    if vendor != 0 && vendor != VIRTIO_VENDOR_QEMU {
        return None;
    }
    if !linux_logic::virtio_device_present(device_id) {
        return None;
    }
    Some(device_id)
}

pub fn modules() -> Vec<ModuleInfo> {
    init();
    registry().modules.clone()
}

pub fn buses() -> Vec<BusInfo> {
    init();
    let reg = registry();
    alloc::vec![
        BusInfo {
            name: "platform",
            devices: reg.platform_devices.len(),
            drivers: reg.platform_drivers.len(),
        },
        BusInfo {
            name: "virtio",
            devices: reg.virtio_devices.len(),
            drivers: reg.virtio_drivers.len(),
        },
        BusInfo {
            name: "pci",
            devices: reg.pci_devices.len(),
            drivers: reg.pci_drivers.len(),
        },
    ]
}

pub fn chrdevs() -> Vec<ChrdevInfo> {
    init();
    registry()
        .chrdevs
        .iter()
        .map(|entry| ChrdevInfo {
            name: entry.name,
            minor: entry.minor,
        })
        .collect()
}

pub fn bindings() -> Vec<UserDriverBinding> {
    init();
    registry().bindings.clone()
}

pub fn stats() -> LinuxDdkStats {
    init();
    let reg = registry();
    LinuxDdkStats {
        api_version: DDK_API_VERSION,
        modules: reg.modules.len(),
        buses: 3,
        platform_devices: reg.platform_devices.len(),
        virtio_devices: reg.virtio_devices.len(),
        pci_devices: reg.pci_devices.len(),
        platform_drivers: reg.platform_drivers.len(),
        virtio_drivers: reg.virtio_drivers.len(),
        pci_drivers: reg.pci_drivers.len() + linux_pci::driver_count(),
        chrdevs: reg.chrdevs.len(),
        irqs: reg.irqs.len(),
        iomaps: reg.iomaps.len(),
    }
}

pub fn intern_name(name: &str) -> Option<&'static str> {
    if name.is_empty() || name.len() > 63 {
        return None;
    }
    let used = registry().name_used;
    let end = used.checked_add(name.len())?;
    if end > NAME_ARENA_SIZE {
        return None;
    }
    unsafe {
        NAME_ARENA.bytes[used..end].copy_from_slice(name.as_bytes());
        registry().name_used = end;
        let slice = core::str::from_utf8_unchecked(&NAME_ARENA.bytes[used..end]);
        Some(&*(slice as *const str))
    }
}

pub fn ram_char_read(buf: &mut [u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    if *ppos < 0 {
        return Err(UserDriverError::OutOfRange);
    }
    let data = unsafe { &RAM_CHAR[..RAM_CHAR_LEN] };
    let pos = *ppos as usize;
    if pos >= data.len() {
        return Ok(0);
    }
    let remaining = data.len() - pos;
    let copy = linux_logic::file_copy_len(buf.len(), remaining);
    buf[..copy].copy_from_slice(&data[pos..pos + copy]);
    *ppos += copy as i64;
    Ok(copy)
}

pub fn ram_char_write(buf: &[u8], ppos: &mut i64) -> Result<usize, UserDriverError> {
    if *ppos < 0 {
        return Err(UserDriverError::OutOfRange);
    }
    let pos = *ppos as usize;
    if pos >= RAM_CHAR_BYTES {
        return Err(UserDriverError::OutOfRange);
    }
    let remaining = RAM_CHAR_BYTES - pos;
    let copy = linux_logic::file_copy_len(buf.len(), remaining);
    unsafe {
        RAM_CHAR[pos..pos + copy].copy_from_slice(&buf[..copy]);
        let end = pos + copy;
        if end > RAM_CHAR_LEN {
            RAM_CHAR_LEN = end;
        }
    }
    *ppos += copy as i64;
    Ok(copy)
}

pub fn ram_char_set(bytes: &[u8]) {
    let copy = linux_logic::file_copy_len(bytes.len(), RAM_CHAR_BYTES);
    unsafe {
        RAM_CHAR[..copy].copy_from_slice(&bytes[..copy]);
        RAM_CHAR_LEN = copy;
    }
}

fn map_phys(phys: usize) -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        if crate::kernel_lowlevel::cpu::user_address_space_active() {
            if let Some(alias) = crate::kernel_lowlevel::virtio_mmio_user_alias(phys) {
                return alias;
            }
        }
    }
    phys
}

fn cstr(ptr: *const c_char) -> Option<&'static str> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        loop {
            let byte = *ptr.add(len);
            if byte == 0 {
                break;
            }
            len = len.checked_add(1)?;
            if len > 63 {
                return None;
            }
        }
        intern_name(core::str::from_utf8(core::slice::from_raw_parts(ptr as *const u8, len)).ok()?)
    }
}

fn ddk_errno(err: UserDriverError) -> c_int {
    match err {
        UserDriverError::NotInitialized => -19,
        UserDriverError::NotFound => -2,
        UserDriverError::NotReady => -19,
        UserDriverError::OutOfRange => -22,
        UserDriverError::InvalidBlock => -22,
        UserDriverError::Unsupported => -22,
        UserDriverError::Io => -5,
        UserDriverError::Timeout => -110,
        UserDriverError::Busy => -16,
        UserDriverError::Exists => -17,
        UserDriverError::NoDev => -19,
        UserDriverError::BadIrq => -22,
        UserDriverError::NoMemory => -12,
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_api_version() -> c_uint {
    DDK_API_VERSION
}

#[no_mangle]
pub extern "C" fn smros_ddk_module_count() -> c_uint {
    stats().modules as c_uint
}

#[no_mangle]
pub extern "C" fn smros_ddk_device_count() -> c_uint {
    let snapshot = stats();
    (snapshot.platform_devices + snapshot.virtio_devices + snapshot.pci_devices) as c_uint
}

#[no_mangle]
pub extern "C" fn smros_ddk_ioremap(phys: u64, size: c_ulong) -> *mut c_void {
    match ioremap(phys, size as usize) {
        Ok(addr) => addr as *mut c_void,
        Err(_) => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_iounmap(addr: *mut c_void, size: c_ulong) {
    iounmap(addr as usize, size as usize);
}

#[no_mangle]
pub extern "C" fn smros_ddk_readl(addr: *const c_void) -> u32 {
    readl(addr as usize)
}

#[no_mangle]
pub extern "C" fn smros_ddk_writel(value: u32, addr: *mut c_void) {
    writel(value, addr as usize);
}

#[no_mangle]
pub extern "C" fn smros_ddk_readw(addr: *const c_void) -> u16 {
    readw(addr as usize)
}

#[no_mangle]
pub extern "C" fn smros_ddk_writew(value: u16, addr: *mut c_void) {
    writew(value, addr as usize);
}

#[no_mangle]
pub extern "C" fn smros_ddk_readb(addr: *const c_void) -> u8 {
    readb(addr as usize)
}

#[no_mangle]
pub extern "C" fn smros_ddk_writeb(value: u8, addr: *mut c_void) {
    writeb(value, addr as usize);
}

#[no_mangle]
pub extern "C" fn smros_ddk_chrdev_read(
    name: *const c_char,
    buf: *mut u8,
    count: c_ulong,
    ppos: *mut i64,
) -> c_long {
    let Some(name) = cstr(name) else {
        return -22;
    };
    if buf.is_null() || ppos.is_null() {
        return -22;
    }
    let out = unsafe { core::slice::from_raw_parts_mut(buf, count as usize) };
    let pos = unsafe { &mut *ppos };
    match chrdev_read(name, out, pos) {
        Ok(written) => written as c_long,
        Err(err) => ddk_errno(err) as c_long,
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_chrdev_write(
    name: *const c_char,
    buf: *const u8,
    count: c_ulong,
    ppos: *mut i64,
) -> c_long {
    let Some(name) = cstr(name) else {
        return -22;
    };
    if buf.is_null() || ppos.is_null() {
        return -22;
    }
    let data = unsafe { core::slice::from_raw_parts(buf, count as usize) };
    let pos = unsafe { &mut *ppos };
    match chrdev_write(name, data, pos) {
        Ok(written) => written as c_long,
        Err(err) => ddk_errno(err) as c_long,
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_dma_alloc_coherent(size: c_ulong, dma_handle: *mut u64) -> *mut c_void {
    match dma_alloc_coherent(size as usize) {
        Ok((cpu, dma)) => {
            if !dma_handle.is_null() {
                unsafe {
                    *dma_handle = dma;
                }
            }
            cpu as *mut c_void
        }
        Err(_) => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_dma_free_coherent(
    size: c_ulong,
    cpu_addr: *mut c_void,
    _dma_handle: u64,
) {
    dma_free_coherent(size as usize, cpu_addr as usize);
}

static mut C_IRQ_SLOTS: [Option<extern "C" fn(c_int, *mut c_void)>; MAX_C_SLOTS] =
    [None; MAX_C_SLOTS];
static mut C_IRQ_DATA: [usize; MAX_C_SLOTS] = [0; MAX_C_SLOTS];

fn c_irq_slot(slot: usize, irq: u32, data: usize) {
    let handler = unsafe { C_IRQ_SLOTS[slot] };
    if let Some(handler) = handler {
        handler(irq as c_int, data as *mut c_void);
    }
}

fn c_irq_0(irq: u32, data: usize) {
    c_irq_slot(0, irq, data);
}

fn c_irq_1(irq: u32, data: usize) {
    c_irq_slot(1, irq, data);
}

fn c_irq_2(irq: u32, data: usize) {
    c_irq_slot(2, irq, data);
}

fn c_irq_3(irq: u32, data: usize) {
    c_irq_slot(3, irq, data);
}

#[no_mangle]
pub extern "C" fn smros_ddk_request_irq(
    irq: c_uint,
    handler: Option<extern "C" fn(c_int, *mut c_void)>,
    flags: c_ulong,
    name: *const c_char,
    data: *mut c_void,
) -> c_int {
    let Some(name) = cstr(name) else {
        return -22;
    };
    let Some(handler) = handler else {
        return -22;
    };
    let slot = unsafe { C_IRQ_SLOTS.iter().position(|entry| entry.is_none()) };
    let Some(slot) = slot else {
        return -16;
    };
    unsafe {
        C_IRQ_SLOTS[slot] = Some(handler);
        C_IRQ_DATA[slot] = data as usize;
    }
    let rust_handler = match slot {
        0 => c_irq_0,
        1 => c_irq_1,
        2 => c_irq_2,
        _ => c_irq_3,
    };
    match request_irq(irq as u32, rust_handler, flags as u32, name, data as usize) {
        Ok(()) => 0,
        Err(err) => {
            unsafe {
                C_IRQ_SLOTS[slot] = None;
                C_IRQ_DATA[slot] = 0;
            }
            ddk_errno(err)
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_free_irq(irq: c_uint, data: *mut c_void) {
    free_irq(irq, data as usize);
    unsafe {
        for slot in 0..MAX_C_SLOTS {
            if C_IRQ_DATA[slot] == data as usize {
                C_IRQ_SLOTS[slot] = None;
                C_IRQ_DATA[slot] = 0;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_bus_kind_valid(kind: c_uint) -> c_int {
    if linux_logic::bus_kind_valid(kind, BUS_PLATFORM, BUS_VIRTIO, BUS_PCI) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_probe_status_ok(code: c_int) -> c_int {
    if linux_logic::probe_status_ok(code) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_id_match(id: c_uint, table_id: c_uint, any_id: c_uint) -> c_int {
    if linux_logic::id_match(id, table_id, any_id) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_puts(s: *const c_char) {
    if s.is_null() {
        return;
    }
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    unsafe {
        let mut len = 0usize;
        while len < 192 {
            let byte = *s.add(len) as u8;
            if byte == 0 {
                break;
            }
            serial.write_byte(byte);
            len += 1;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn smros_ddk_kmalloc(size: c_ulong, _gfp: c_uint) -> *mut c_void {
    let size = size as usize;
    if size == 0 || size > KMALLOC_MAX {
        return core::ptr::null_mut();
    }
    let header = core::mem::size_of::<usize>();
    let total = match size.checked_add(header) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    let Ok(layout) = core::alloc::Layout::from_size_align(total, core::mem::align_of::<usize>())
    else {
        return core::ptr::null_mut();
    };
    let ptr = alloc::alloc::alloc(layout);
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    (ptr as *mut usize).write(total);
    ptr.add(header) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn smros_ddk_kfree(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let header = core::mem::size_of::<usize>();
    let base = (ptr as *mut u8).sub(header);
    let total = (base as *mut usize).read();
    if total < header || total > KMALLOC_MAX + header {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(total, core::mem::align_of::<usize>())
    else {
        return;
    };
    alloc::alloc::dealloc(base, layout);
}
