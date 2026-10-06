//! User-space Linux driver framework for SMROS bring-up.
//!
//! Device discovery still uses a Linux-device-tree-shaped table for QEMU
//! `virt` plus PCI on x86_64. Drivers now register through the Linux-shaped
//! DDK (`module_init` / bus matching / `probe`) in `linux.rs`.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;
#[cfg(target_arch = "x86_64")]
use core::sync::atomic::Ordering;

pub mod block;
pub mod ddk;
pub mod demos;
pub(crate) mod driver_logic;
pub mod linux;
pub(crate) mod linux_logic;
pub mod linuxcompat;
pub mod net;
pub mod pci;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDriverError {
    NotInitialized,
    NotFound,
    NotReady,
    OutOfRange,
    InvalidBlock,
    Unsupported,
    Io,
    Timeout,
    Busy,
    Exists,
    NoDev,
    BadIrq,
    NoMemory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDeviceKind {
    Cpu,
    Memory,
    Serial,
    InterruptController,
    Timer,
    VirtioMmio,
    Block,
    Network,
    Platform,
    Char,
    Pci,
}

impl UserDeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UserDeviceKind::Cpu => "cpu",
            UserDeviceKind::Memory => "memory",
            UserDeviceKind::Serial => "serial",
            UserDeviceKind::InterruptController => "interrupt-controller",
            UserDeviceKind::Timer => "timer",
            UserDeviceKind::VirtioMmio => "virtio-mmio",
            UserDeviceKind::Block => "block",
            UserDeviceKind::Network => "network",
            UserDeviceKind::Platform => "platform",
            UserDeviceKind::Char => "char",
            UserDeviceKind::Pci => "pci",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UserDeviceReg {
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct UserDeviceNode {
    pub path: &'static str,
    pub name: &'static str,
    pub compatible: &'static str,
    pub status: &'static str,
    pub kind: UserDeviceKind,
    pub reg: Option<UserDeviceReg>,
    pub irq: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct UserDriverBinding {
    pub node_path: &'static str,
    pub driver: &'static str,
    pub device_name: &'static str,
    pub kind: UserDeviceKind,
    pub block_size: usize,
    pub block_count: usize,
    pub mtu: usize,
    pub mac: [u8; 6],
}

#[derive(Clone, Copy, Debug)]
pub struct UserDriverStats {
    pub initialized: bool,
    pub machine: &'static str,
    pub nodes: usize,
    pub bindings: usize,
    pub block_ready: bool,
    pub mmio_base: usize,
    pub device_status: u32,
    pub last_error: Option<UserDriverError>,
    pub block_size: usize,
    pub block_count: usize,
    pub bytes: usize,
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub net_ready: bool,
    pub net_mmio_base: usize,
    pub net_device_status: u32,
    pub net_last_error: Option<UserDriverError>,
    pub net_mac: [u8; 6],
    pub net_link_up: bool,
    pub net_mtu: usize,
    pub net_rx_packets: u64,
    pub net_tx_packets: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_dropped_packets: u64,
    pub ddk_api_version: u32,
    pub ddk_modules: usize,
    pub ddk_buses: usize,
    pub ddk_devices: usize,
    pub ddk_drivers: usize,
    pub ddk_chrdevs: usize,
    pub hello_ready: bool,
    pub dummy_ready: bool,
    pub edu_ready: bool,
}

struct UserDriverFramework {
    initialized: bool,
    nodes: Vec<UserDeviceNode>,
    bindings: Vec<UserDriverBinding>,
}

impl UserDriverFramework {
    fn new() -> Self {
        Self {
            initialized: false,
            nodes: Vec::new(),
            bindings: Vec::new(),
        }
    }

    #[inline(never)]
    fn init(&mut self) -> bool {
        if self.initialized {
            return block::ready() || net::ready();
        }
        self.nodes.clear();
        self.bindings.clear();
        linux::init();
        self.install_qemu_virt_tree();
        demos::register();
        block::register_linux_driver();
        net::register_linux_driver();
        self.discover_linux_devices();
        linuxcompat::init();
        linux::attach();
        self.sync_bindings();
        let ready = block::ready() || net::ready();
        self.initialized = true;
        #[cfg(target_arch = "x86_64")]
        if ready && !DRIVER_INIT_LOGGED.swap(true, Ordering::AcqRel) {
            let mut serial = crate::kernel_lowlevel::serial::Serial::new();
            serial.init();
            serial.write_str("[USER][DRV] x86_64 VirtIO-PCI drivers initialized\n");
        }
        ready
    }

    fn install_qemu_virt_tree(&mut self) {
        self.nodes.push(UserDeviceNode {
            path: "/cpus/cpu@0",
            name: "cpu@0",
            compatible: if cfg!(target_arch = "x86_64") {
                "intel,x86_64"
            } else if cfg!(target_arch = "riscv64") {
                "riscv"
            } else {
                "arm,cortex-a710"
            },
            status: "okay",
            kind: UserDeviceKind::Cpu,
            reg: Some(UserDeviceReg { base: 0, size: 1 }),
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/memory@40000000",
            name: "memory@40000000",
            compatible: "qemu,virt-memory",
            status: "okay",
            kind: UserDeviceKind::Memory,
            reg: Some(UserDeviceReg {
                base: 0x4000_0000,
                size: 0x2000_0000,
            }),
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/serial",
            name: "serial",
            compatible: if cfg!(target_arch = "x86_64") {
                "ns16550a"
            } else if cfg!(target_arch = "riscv64") {
                "ns16550a"
            } else {
                "arm,pl011"
            },
            status: "okay",
            kind: UserDeviceKind::Serial,
            reg: Some(UserDeviceReg {
                base: crate::kernel_lowlevel::drivers::uart_base() as u64,
                size: crate::kernel_lowlevel::drivers::uart_size() as u64,
            }),
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/interrupt-controller",
            name: "interrupt-controller",
            compatible: if cfg!(target_arch = "x86_64") {
                "intel,local-apic"
            } else if cfg!(target_arch = "riscv64") {
                "riscv,plic0"
            } else {
                "arm,gic"
            },
            status: "okay",
            kind: UserDeviceKind::InterruptController,
            reg: None,
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/timer",
            name: "timer",
            compatible: if cfg!(target_arch = "x86_64") {
                "intel,invariant-tsc"
            } else if cfg!(target_arch = "riscv64") {
                "riscv,timer"
            } else {
                "arm,armv8-timer"
            },
            status: "okay",
            kind: UserDeviceKind::Timer,
            reg: None,
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/smros-dummy",
            name: "smros-dummy",
            compatible: demos::DUMMY_COMPATIBLE,
            status: "okay",
            kind: UserDeviceKind::Platform,
            reg: Some(UserDeviceReg {
                base: demos::dummy_reg_base(),
                size: demos::dummy_reg_size(),
            }),
            irq: Some(demos::DUMMY_IRQ),
        });
        self.nodes.push(UserDeviceNode {
            path: linuxcompat::EDU_NODE,
            name: linuxcompat::EDU_NAME,
            compatible: "qemu,edu",
            status: "okay",
            kind: UserDeviceKind::Pci,
            reg: Some(UserDeviceReg {
                base: linuxcompat::bar0_start(),
                size: linuxcompat::bar0_size(),
            }),
            irq: Some(linuxcompat::EDU_IRQ),
        });
        for index in 0..crate::kernel_lowlevel::drivers::virtio_mmio_count() {
            let Some(reg) = crate::kernel_lowlevel::drivers::virtio_mmio_reg(index) else {
                continue;
            };
            self.nodes.push(UserDeviceNode {
                path: "/virtio_mmio",
                name: "virtio_mmio",
                compatible: "virtio,mmio",
                status: "okay",
                kind: UserDeviceKind::VirtioMmio,
                reg: Some(UserDeviceReg {
                    base: reg.base as u64,
                    size: reg.size as u64,
                }),
                irq: None,
            });
        }
    }

    fn discover_linux_devices(&mut self) {
        for node in &self.nodes {
            if node.kind == UserDeviceKind::Platform && node.compatible == demos::DUMMY_COMPATIBLE {
                let _ = linux::register_platform_device(linux::PlatformDevice {
                    name: node.name,
                    node_path: node.path,
                    compatible: node.compatible,
                    mmio: node.reg,
                    irq: node.irq,
                    driver: None,
                });
            }
            if node.kind != UserDeviceKind::VirtioMmio {
                continue;
            }
            let Some(reg) = node.reg else {
                continue;
            };
            if node.compatible != "virtio,mmio" || node.status != "okay" {
                continue;
            }
            let Some(device_id) = linux::virtio_mmio_device_id(reg.base as usize) else {
                continue;
            };
            let _ = linux::register_virtio_mmio_device(
                virtio_device_name(device_id),
                node.path,
                reg.base as usize,
                reg.size,
                node.irq,
            );
        }

        #[cfg(target_arch = "x86_64")]
        {
            if let Some(transport) = pci::find_virtio_block_transport() {
                let _ = linux::register_virtio_pci_device(
                    "vblk0",
                    "/pci/virtio-block",
                    linux::VIRTIO_ID_BLOCK,
                    transport,
                );
            }
            if let Some(transport) = pci::find_virtio_net_transport() {
                let _ = linux::register_virtio_pci_device(
                    "eth0",
                    "/pci/virtio-net",
                    linux::VIRTIO_ID_NET,
                    transport,
                );
            }
            for (index, hw) in pci::enumerate_devices().into_iter().enumerate() {
                let mut bars = [linux::PciBar::empty(); 6];
                for (bar_index, bar) in hw.bars.iter().enumerate() {
                    bars[bar_index] = linux::PciBar {
                        start: bar.start,
                        size: bar.size,
                        flags: if bar.io {
                            linux::IORESOURCE_IO
                        } else if bar.size != 0 {
                            linux::IORESOURCE_MEM
                        } else {
                            0
                        },
                    };
                }
                let name = linux::intern_name(&alloc::format!("pci{index}")).unwrap_or("pci");
                let path = linux::intern_name(&alloc::format!(
                    "/pci/{:02x}:{:02x}.{}",
                    hw.bus,
                    hw.device,
                    hw.function
                ))
                .unwrap_or("/pci");
                let _ = linux::register_pci_device(linux::PciDevice {
                    name,
                    node_path: path,
                    vendor: hw.vendor as u32,
                    device: hw.device_id as u32,
                    class: hw.class,
                    revision: hw.revision,
                    irq: if hw.irq == 0 {
                        None
                    } else {
                        Some(hw.irq as u32)
                    },
                    bars,
                    bus: hw.bus,
                    devfn: ((hw.device & 0x1f) << 3) | (hw.function & 0x7),
                    subsystem_vendor: hw.subsystem_vendor as u32,
                    subsystem_device: hw.subsystem_device as u32,
                    driver: None,
                });
            }
        }
    }

    fn sync_bindings(&mut self) {
        self.bindings = linux::bindings();
        for binding in &mut self.bindings {
            if binding.kind == UserDeviceKind::Block {
                binding.device_name = "vblk0";
                binding.block_size = block::BLOCK_SIZE;
                binding.block_count = block::capacity_blocks();
            }
            if binding.kind == UserDeviceKind::Network {
                binding.device_name = "eth0";
                binding.mtu = net::ETHERNET_MTU;
                binding.mac = net::mac();
            }
        }
    }

    fn stats(&self) -> UserDriverStats {
        let block_count = block::capacity_blocks();
        let ddk = linux::stats();
        UserDriverStats {
            initialized: self.initialized,
            machine: crate::kernel_lowlevel::drivers::architecture_name(),
            nodes: self.nodes.len(),
            bindings: self.bindings.len(),
            block_ready: block::ready(),
            mmio_base: block::mmio_base(),
            device_status: block::device_status(),
            last_error: block::last_error(),
            block_size: block::BLOCK_SIZE,
            block_count,
            bytes: block::capacity_bytes(),
            reads: block::reads(),
            writes: block::writes(),
            flushes: block::flushes(),
            bytes_read: block::bytes_read(),
            bytes_written: block::bytes_written(),
            net_ready: net::ready(),
            net_mmio_base: net::mmio_base(),
            net_device_status: net::device_status(),
            net_last_error: net::last_error(),
            net_mac: net::mac(),
            net_link_up: net::link_up(),
            net_mtu: net::ETHERNET_MTU,
            net_rx_packets: net::rx_packets(),
            net_tx_packets: net::tx_packets(),
            net_rx_bytes: net::rx_bytes(),
            net_tx_bytes: net::tx_bytes(),
            net_dropped_packets: net::dropped_packets(),
            ddk_api_version: ddk.api_version,
            ddk_modules: ddk.modules,
            ddk_buses: ddk.buses,
            ddk_devices: ddk.platform_devices + ddk.virtio_devices + ddk.pci_devices,
            ddk_drivers: ddk.platform_drivers + ddk.virtio_drivers + ddk.pci_drivers,
            ddk_chrdevs: ddk.chrdevs,
            hello_ready: demos::hello_ready(),
            dummy_ready: demos::dummy_ready(),
            edu_ready: linuxcompat::ready(),
        }
    }
}

static mut DRIVER_FRAMEWORK: Option<UserDriverFramework> = None;
static DRIVER_INIT_LOGGED: AtomicBool = AtomicBool::new(false);

fn framework() -> &'static mut UserDriverFramework {
    unsafe {
        if DRIVER_FRAMEWORK.is_none() {
            DRIVER_FRAMEWORK = Some(UserDriverFramework::new());
        }
        DRIVER_FRAMEWORK.as_mut().unwrap()
    }
}

fn virtio_device_name(device_id: u32) -> &'static str {
    match device_id {
        linux::VIRTIO_ID_BLOCK => "vblk0",
        linux::VIRTIO_ID_NET => "eth0",
        _ => "virtio",
    }
}

#[inline(never)]
pub fn init() -> bool {
    let ready = framework().init();
    crate::kernel_lowlevel::cpu::mmio_barrier();
    ready
}

pub fn stats() -> UserDriverStats {
    framework().stats()
}

pub fn device_nodes() -> Vec<UserDeviceNode> {
    framework().nodes.clone()
}

pub fn bindings() -> Vec<UserDriverBinding> {
    framework().bindings.clone()
}

pub fn block_ready() -> bool {
    block::ready()
}

pub fn block_size() -> usize {
    block::BLOCK_SIZE
}

pub fn block_count() -> usize {
    block::capacity_blocks()
}

pub fn block_capacity() -> usize {
    block::capacity_bytes()
}

pub fn block_read_at(offset: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::read_at(offset, out)
}

pub fn block_write_at(offset: usize, data: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::write_at(offset, data)
}

pub fn block_read(block_id: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::read(block_id, out)
}

pub fn block_write(block_id: usize, data: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::write(block_id, data)
}

pub fn block_clear() -> Result<(), UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::clear()
}

pub fn block_flush() -> Result<(), UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    block::flush()
}

pub fn net_ready() -> bool {
    net::ready()
}

pub fn net_mac() -> [u8; 6] {
    net::mac()
}

pub fn net_link_up() -> bool {
    net::link_up()
}

pub fn net_send_frame(frame: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    net::send_frame(frame)
}

pub fn net_receive_frame(out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    net::receive_frame(out)
}

pub fn net_receive_frame_timeout(
    out: &mut [u8],
    timeout_spins: usize,
) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    net::receive_frame_timeout(out, timeout_spins)
}

pub fn ddk_smoke_test() -> bool {
    if !framework().initialized {
        let _ = framework().init();
    }
    demos::smoke() && linuxcompat::ready()
}

pub fn edu_ready() -> bool {
    linuxcompat::ready()
}

pub fn edu_ident() -> u32 {
    linuxcompat::ident()
}

pub fn hello_read(out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized {
        let _ = framework().init();
    }
    let mut pos = 0i64;
    linux::chrdev_open(demos::HELLO_NAME)?;
    linux::chrdev_read(demos::HELLO_NAME, out, &mut pos)
}

pub fn hello_write(data: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized {
        let _ = framework().init();
    }
    let mut pos = 0i64;
    linux::chrdev_open(demos::HELLO_NAME)?;
    linux::chrdev_write(demos::HELLO_NAME, data, &mut pos)
}

pub fn smoke_test() -> bool {
    if !ddk_smoke_test() {
        return false;
    }
    if !init() || !block_ready() || block_count() < 2 {
        return false;
    }
    let mut block_buf = [0u8; block::BLOCK_SIZE];
    if block_read(1, &mut block_buf).is_err() {
        return false;
    }
    let saved = block_buf;
    block_buf[0..11].copy_from_slice(b"smros-block");
    if block_write(1, &block_buf).is_err() {
        return false;
    }
    let mut out = [0u8; block::BLOCK_SIZE];
    let ok = block_read(1, &mut out).is_ok() && out[0..11] == *b"smros-block";
    let _ = block_write(1, &saved);
    ok && block_flush().is_ok()
}
