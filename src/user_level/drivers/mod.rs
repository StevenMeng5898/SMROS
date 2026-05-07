//! User-space driver framework for SMROS bring-up.
//!
//! SMROS does not pass QEMU's live FDT into userspace yet, so this module keeps
//! a Linux-device-tree-shaped table for QEMU `virt` devices and binds drivers
//! against it. Device-specific drivers live under this module.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;

pub mod block;
pub mod net;

const QEMU_VIRT_MACHINE: &str = "linux,dummy-virt";

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

    fn init(&mut self) -> bool {
        if self.initialized {
            return true;
        }

        self.nodes.clear();
        self.bindings.clear();
        self.install_qemu_virt_tree();
        self.probe();
        self.initialized = block::ready() || net::ready();
        self.initialized
    }

    fn install_qemu_virt_tree(&mut self) {
        self.nodes.push(UserDeviceNode {
            path: "/cpus/cpu@0",
            name: "cpu@0",
            compatible: "arm,cortex-a57",
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
            path: "/pl011@9000000",
            name: "pl011@9000000",
            compatible: "arm,pl011",
            status: "okay",
            kind: UserDeviceKind::Serial,
            reg: Some(UserDeviceReg {
                base: 0x0900_0000,
                size: 0x1000,
            }),
            irq: Some(33),
        });
        self.nodes.push(UserDeviceNode {
            path: "/intc@8000000",
            name: "intc@8000000",
            compatible: "arm,cortex-a15-gic",
            status: "okay",
            kind: UserDeviceKind::InterruptController,
            reg: Some(UserDeviceReg {
                base: 0x0800_0000,
                size: 0x10000,
            }),
            irq: None,
        });
        self.nodes.push(UserDeviceNode {
            path: "/timer",
            name: "timer",
            compatible: "arm,armv8-timer",
            status: "okay",
            kind: UserDeviceKind::Timer,
            reg: None,
            irq: Some(27),
        });
        self.nodes.push(UserDeviceNode {
            path: "/virtio_mmio@a000000",
            name: "virtio_mmio@a000000..a003e00",
            compatible: "virtio,mmio",
            status: "okay",
            kind: UserDeviceKind::VirtioMmio,
            reg: Some(UserDeviceReg {
                base: block::MMIO_BASE as u64,
                size: (block::MMIO_STRIDE * block::MMIO_SLOT_COUNT) as u64,
            }),
            irq: Some(48),
        });
    }

    fn probe(&mut self) {
        let mut block_bound = false;
        let mut net_bound = false;
        for node in &self.nodes {
            if node.compatible == "virtio,mmio"
                && node.status == "okay"
                && node.kind == UserDeviceKind::VirtioMmio
                && !block_bound
                && block::bind().is_ok()
            {
                block_bound = true;
                self.bindings.push(UserDriverBinding {
                    node_path: node.path,
                    driver: "qemu-virtio-mmio-block",
                    device_name: "vblk0",
                    kind: UserDeviceKind::Block,
                    block_size: block::BLOCK_SIZE,
                    block_count: block::capacity_blocks(),
                    mtu: 0,
                    mac: [0; 6],
                });
            }

            if node.compatible == "virtio,mmio"
                && node.status == "okay"
                && node.kind == UserDeviceKind::VirtioMmio
                && !net_bound
                && net::bind().is_ok()
            {
                net_bound = true;
                self.bindings.push(UserDriverBinding {
                    node_path: node.path,
                    driver: "qemu-virtio-mmio-net",
                    device_name: "eth0",
                    kind: UserDeviceKind::Network,
                    block_size: 0,
                    block_count: 0,
                    mtu: net::ETHERNET_MTU,
                    mac: net::mac(),
                });
            }
        }
    }

    fn stats(&self) -> UserDriverStats {
        let block_count = block::capacity_blocks();
        UserDriverStats {
            initialized: self.initialized,
            machine: QEMU_VIRT_MACHINE,
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
        }
    }
}

static mut DRIVER_FRAMEWORK: Option<UserDriverFramework> = None;

fn framework() -> &'static mut UserDriverFramework {
    unsafe {
        if DRIVER_FRAMEWORK.is_none() {
            DRIVER_FRAMEWORK = Some(UserDriverFramework::new());
        }
        DRIVER_FRAMEWORK.as_mut().unwrap()
    }
}

pub fn init() -> bool {
    framework().init()
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

pub fn smoke_test() -> bool {
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
