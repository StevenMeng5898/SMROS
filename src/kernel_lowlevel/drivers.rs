#![allow(dead_code)]
//! Low-level ARM platform driver registry.
//!
//! The hot drivers stay ARM-specific, but their MMIO resources come from this
//! Linux-device-tree-shaped platform table instead of being hard-coded in each
//! driver. QEMU `virt` remains the default, and new ARM boards can be added by
//! describing their compatible strings and register windows here.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::lowlevel_logic;

pub const MAX_PLATFORM_IRQS: u32 = 1024;
pub const DEFAULT_PLATFORM_INDEX: usize = 0;
const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;
const FDT_HEADER_SIZE: usize = 40;
const FDT_MAX_SCAN_BYTES: usize = 0x20_0000;
const FDT_MAX_DEPTH: usize = 8;
const FDT_DEFAULT_ADDRESS_CELLS: usize = 2;
const FDT_DEFAULT_SIZE_CELLS: usize = 1;
const FDT_MAX_ADDRESS_CELLS: usize = 2;
const FDT_MAX_SIZE_CELLS: usize = 2;
const FDT_GIC_INTERRUPT_CELLS: usize = 3;
const FDT_GENERIC_MACHINE: &str = "fdt,generic-arm";

pub const QEMU_VIRT_UART_BASE: usize = 0x0900_0000;
pub const QEMU_VIRT_UART_SIZE: usize = 0x1000;
pub const QEMU_VIRT_UART_IRQ: u32 = 33;
pub const QEMU_VIRT_GICD_BASE: usize = 0x0800_0000;
pub const QEMU_VIRT_GICD_SIZE: usize = 0x10000;
pub const QEMU_VIRT_GICC_BASE: usize = 0x0801_0000;
pub const QEMU_VIRT_GICC_SIZE: usize = 0x10000;
pub const QEMU_VIRT_TIMER_IRQ: u32 = 30;
pub const QEMU_VIRT_MEMORY_BASE: usize = 0x4000_0000;
pub const QEMU_VIRT_MEMORY_SIZE: usize = 0x2000_0000;

pub const RPI4_UART_BASE: usize = 0xfe20_1000;
pub const RPI4_UART_SIZE: usize = 0x1000;
pub const RPI4_UART_IRQ: u32 = 153;
pub const RPI4_GICD_BASE: usize = 0xff84_1000;
pub const RPI4_GICD_SIZE: usize = 0x1000;
pub const RPI4_GICC_BASE: usize = 0xff84_2000;
pub const RPI4_GICC_SIZE: usize = 0x2000;
pub const RPI4_TIMER_IRQ: u32 = 30;
pub const RPI4_MEMORY_BASE: usize = 0x0000_0000;
pub const RPI4_MEMORY_SIZE: usize = 0x3c00_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverError {
    InvalidPlatform,
    DeviceNotFound,
    Disabled,
    InvalidResource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Cpu,
    Memory,
    Serial,
    InterruptController,
    Timer,
}

impl DeviceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Cpu => "cpu",
            DeviceKind::Memory => "memory",
            DeviceKind::Serial => "serial",
            DeviceKind::InterruptController => "interrupt-controller",
            DeviceKind::Timer => "timer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceReg {
    pub base: usize,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceNode {
    pub path: &'static str,
    pub name: &'static str,
    pub compatible: &'static str,
    pub status: &'static str,
    pub kind: DeviceKind,
    pub reg: Option<DeviceReg>,
    pub reg2: Option<DeviceReg>,
    pub irq: Option<u32>,
}

impl DeviceNode {
    pub fn is_available(self) -> bool {
        self.status == "okay"
    }

    pub fn primary_reg(self) -> Result<DeviceReg, DriverError> {
        let Some(reg) = self.reg else {
            return Err(DriverError::InvalidResource);
        };
        if lowlevel_logic::dt_reg_valid(reg.base, reg.size) {
            Ok(reg)
        } else {
            Err(DriverError::InvalidResource)
        }
    }

    pub fn secondary_reg(self) -> Result<DeviceReg, DriverError> {
        let Some(reg) = self.reg2 else {
            return Err(DriverError::InvalidResource);
        };
        if lowlevel_logic::dt_reg_valid(reg.base, reg.size) {
            Ok(reg)
        } else {
            Err(DriverError::InvalidResource)
        }
    }

    pub fn irq(self) -> Result<u32, DriverError> {
        let Some(irq) = self.irq else {
            return Err(DriverError::InvalidResource);
        };
        if lowlevel_logic::dt_irq_valid(irq, MAX_PLATFORM_IRQS) {
            Ok(irq)
        } else {
            Err(DriverError::InvalidResource)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformDescriptor {
    pub machine: &'static str,
    pub root_compatible: &'static [&'static str],
    pub nodes: &'static [DeviceNode],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformResources {
    pub machine: &'static str,
    pub source: ResourceSource,
    pub uart_base: usize,
    pub uart_size: usize,
    pub uart_irq: u32,
    pub gicd_base: usize,
    pub gicd_size: usize,
    pub gicc_base: usize,
    pub gicc_size: usize,
    pub timer_irq: u32,
}

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceSource {
    Uninitialized,
    Fdt,
    StaticFallback,
}

impl ResourceSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceSource::Uninitialized => "uninitialized",
            ResourceSource::Fdt => "fdt",
            ResourceSource::StaticFallback => "static-fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverStats {
    pub initialized: bool,
    pub platform_index: usize,
    pub machine: &'static str,
    pub source: ResourceSource,
    pub nodes: usize,
    pub uart_base: usize,
    pub uart_irq: u32,
    pub gicd_base: usize,
    pub gicc_base: usize,
    pub timer_irq: u32,
}

const QEMU_VIRT_NODES: &[DeviceNode] = &[
    DeviceNode {
        path: "/cpus/cpu@0",
        name: "cpu@0",
        compatible: "arm,cortex-a57",
        status: "okay",
        kind: DeviceKind::Cpu,
        reg: Some(DeviceReg { base: 0, size: 1 }),
        reg2: None,
        irq: None,
    },
    DeviceNode {
        path: "/memory@40000000",
        name: "memory@40000000",
        compatible: "qemu,virt-memory",
        status: "okay",
        kind: DeviceKind::Memory,
        reg: Some(DeviceReg {
            base: QEMU_VIRT_MEMORY_BASE,
            size: QEMU_VIRT_MEMORY_SIZE,
        }),
        reg2: None,
        irq: None,
    },
    DeviceNode {
        path: "/pl011@9000000",
        name: "pl011@9000000",
        compatible: "arm,pl011",
        status: "okay",
        kind: DeviceKind::Serial,
        reg: Some(DeviceReg {
            base: QEMU_VIRT_UART_BASE,
            size: QEMU_VIRT_UART_SIZE,
        }),
        reg2: None,
        irq: Some(QEMU_VIRT_UART_IRQ),
    },
    DeviceNode {
        path: "/intc@8000000",
        name: "intc@8000000",
        compatible: "arm,cortex-a15-gic",
        status: "okay",
        kind: DeviceKind::InterruptController,
        reg: Some(DeviceReg {
            base: QEMU_VIRT_GICD_BASE,
            size: QEMU_VIRT_GICD_SIZE,
        }),
        reg2: Some(DeviceReg {
            base: QEMU_VIRT_GICC_BASE,
            size: QEMU_VIRT_GICC_SIZE,
        }),
        irq: None,
    },
    DeviceNode {
        path: "/timer",
        name: "timer",
        compatible: "arm,armv8-timer",
        status: "okay",
        kind: DeviceKind::Timer,
        reg: None,
        reg2: None,
        irq: Some(QEMU_VIRT_TIMER_IRQ),
    },
];

const RPI4_NODES: &[DeviceNode] = &[
    DeviceNode {
        path: "/cpus/cpu@0",
        name: "cpu@0",
        compatible: "arm,cortex-a72",
        status: "okay",
        kind: DeviceKind::Cpu,
        reg: Some(DeviceReg { base: 0, size: 1 }),
        reg2: None,
        irq: None,
    },
    DeviceNode {
        path: "/memory@0",
        name: "memory@0",
        compatible: "raspberrypi,4-memory",
        status: "okay",
        kind: DeviceKind::Memory,
        reg: Some(DeviceReg {
            base: RPI4_MEMORY_BASE,
            size: RPI4_MEMORY_SIZE,
        }),
        reg2: None,
        irq: None,
    },
    DeviceNode {
        path: "/serial@fe201000",
        name: "serial@fe201000",
        compatible: "arm,pl011",
        status: "okay",
        kind: DeviceKind::Serial,
        reg: Some(DeviceReg {
            base: RPI4_UART_BASE,
            size: RPI4_UART_SIZE,
        }),
        reg2: None,
        irq: Some(RPI4_UART_IRQ),
    },
    DeviceNode {
        path: "/interrupt-controller@ff841000",
        name: "interrupt-controller@ff841000",
        compatible: "arm,gic-400",
        status: "okay",
        kind: DeviceKind::InterruptController,
        reg: Some(DeviceReg {
            base: RPI4_GICD_BASE,
            size: RPI4_GICD_SIZE,
        }),
        reg2: Some(DeviceReg {
            base: RPI4_GICC_BASE,
            size: RPI4_GICC_SIZE,
        }),
        irq: None,
    },
    DeviceNode {
        path: "/timer",
        name: "timer",
        compatible: "arm,armv8-timer",
        status: "okay",
        kind: DeviceKind::Timer,
        reg: None,
        reg2: None,
        irq: Some(RPI4_TIMER_IRQ),
    },
];

const PLATFORMS: &[PlatformDescriptor] = &[
    PlatformDescriptor {
        machine: "linux,dummy-virt",
        root_compatible: &["linux,dummy-virt", "qemu,virt"],
        nodes: QEMU_VIRT_NODES,
    },
    PlatformDescriptor {
        machine: "raspberrypi,4-model-b",
        root_compatible: &["raspberrypi,4-model-b", "brcm,bcm2711"],
        nodes: RPI4_NODES,
    },
];

static ACTIVE_PLATFORM_INDEX: AtomicUsize = AtomicUsize::new(DEFAULT_PLATFORM_INDEX);
static INIT_STATE: AtomicUsize = AtomicUsize::new(0);
static UART_BASE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_UART_BASE);
static UART_SIZE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_UART_SIZE);
static UART_IRQ: AtomicUsize = AtomicUsize::new(QEMU_VIRT_UART_IRQ as usize);
static GICD_BASE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_GICD_BASE);
static GICD_SIZE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_GICD_SIZE);
static GICC_BASE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_GICC_BASE);
static GICC_SIZE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_GICC_SIZE);
static TIMER_IRQ: AtomicUsize = AtomicUsize::new(QEMU_VIRT_TIMER_IRQ as usize);
static RESOURCE_SOURCE: AtomicUsize = AtomicUsize::new(ResourceSource::Uninitialized as usize);

pub fn init() -> bool {
    init_for_compatible("linux,dummy-virt")
}

pub fn init_from_fdt(fdt_base: usize) -> bool {
    if let Some(resources) = fdt_platform_resources(fdt_base) {
        cache_resources(select_platform_index(resources.machine), resources);
        true
    } else {
        init()
    }
}

pub fn init_for_compatible(compatible: &str) -> bool {
    let index = select_platform_index(compatible);
    match platform_resources(index) {
        Ok(resources) => {
            cache_resources(index, resources);
            true
        }
        Err(_) => {
            let fallback = DEFAULT_PLATFORM_INDEX;
            if let Ok(resources) = platform_resources(fallback) {
                cache_resources(fallback, resources);
                true
            } else {
                INIT_STATE.store(0, Ordering::Release);
                false
            }
        }
    }
}

pub fn init_for_platform(index: usize) -> bool {
    let selected =
        lowlevel_logic::dt_platform_index(index, PLATFORMS.len(), DEFAULT_PLATFORM_INDEX);
    match platform_resources(selected) {
        Ok(resources) => {
            cache_resources(selected, resources);
            true
        }
        Err(_) => false,
    }
}

pub fn platform_count() -> usize {
    PLATFORMS.len()
}

pub fn active_platform() -> &'static PlatformDescriptor {
    let index = lowlevel_logic::dt_platform_index(
        ACTIVE_PLATFORM_INDEX.load(Ordering::Acquire),
        PLATFORMS.len(),
        DEFAULT_PLATFORM_INDEX,
    );
    &PLATFORMS[index]
}

pub fn platforms() -> &'static [PlatformDescriptor] {
    PLATFORMS
}

pub fn device_nodes() -> &'static [DeviceNode] {
    active_platform().nodes
}

pub fn find_node(kind: DeviceKind) -> Option<&'static DeviceNode> {
    active_platform()
        .nodes
        .iter()
        .find(|node| node.kind == kind && node.is_available())
}

pub fn uart_base() -> usize {
    ensure_initialized();
    UART_BASE.load(Ordering::Acquire)
}

pub fn uart_size() -> usize {
    ensure_initialized();
    UART_SIZE.load(Ordering::Acquire)
}

pub fn uart_irq() -> u32 {
    ensure_initialized();
    UART_IRQ.load(Ordering::Acquire) as u32
}

pub fn gicd_base() -> usize {
    ensure_initialized();
    GICD_BASE.load(Ordering::Acquire)
}

pub fn gicd_size() -> usize {
    ensure_initialized();
    GICD_SIZE.load(Ordering::Acquire)
}

pub fn gicc_base() -> usize {
    ensure_initialized();
    GICC_BASE.load(Ordering::Acquire)
}

pub fn gicc_size() -> usize {
    ensure_initialized();
    GICC_SIZE.load(Ordering::Acquire)
}

pub fn timer_irq() -> u32 {
    ensure_initialized();
    TIMER_IRQ.load(Ordering::Acquire) as u32
}

pub fn stats() -> DriverStats {
    ensure_initialized();
    let index = ACTIVE_PLATFORM_INDEX.load(Ordering::Acquire);
    let platform = &PLATFORMS
        [lowlevel_logic::dt_platform_index(index, PLATFORMS.len(), DEFAULT_PLATFORM_INDEX)];
    DriverStats {
        initialized: INIT_STATE.load(Ordering::Acquire) != 0,
        platform_index: index,
        machine: platform.machine,
        source: resource_source(),
        nodes: platform.nodes.len(),
        uart_base: UART_BASE.load(Ordering::Acquire),
        uart_irq: UART_IRQ.load(Ordering::Acquire) as u32,
        gicd_base: GICD_BASE.load(Ordering::Acquire),
        gicc_base: GICC_BASE.load(Ordering::Acquire),
        timer_irq: TIMER_IRQ.load(Ordering::Acquire) as u32,
    }
}

pub fn describe(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    let snapshot = stats();
    serial.write_str("[DRV] Platform: ");
    serial.write_str(snapshot.machine);
    serial.write_str(" source=");
    serial.write_str(snapshot.source.as_str());
    serial.write_str(" nodes=");
    print_number(serial, snapshot.nodes as u32);
    serial.write_str(" uart=0x");
    serial.write_hex(snapshot.uart_base as u64);
    serial.write_str(" gicd=0x");
    serial.write_hex(snapshot.gicd_base as u64);
    serial.write_str(" gicc=0x");
    serial.write_hex(snapshot.gicc_base as u64);
    serial.write_str(" timer_irq=");
    print_number(serial, snapshot.timer_irq);
    serial.write_str("\n");
}

fn ensure_initialized() {
    if INIT_STATE.load(Ordering::Acquire) == 0 {
        let _ = init();
    }
}

fn cache_resources(index: usize, resources: PlatformResources) {
    UART_BASE.store(resources.uart_base, Ordering::Release);
    UART_SIZE.store(resources.uart_size, Ordering::Release);
    UART_IRQ.store(resources.uart_irq as usize, Ordering::Release);
    GICD_BASE.store(resources.gicd_base, Ordering::Release);
    GICD_SIZE.store(resources.gicd_size, Ordering::Release);
    GICC_BASE.store(resources.gicc_base, Ordering::Release);
    GICC_SIZE.store(resources.gicc_size, Ordering::Release);
    TIMER_IRQ.store(resources.timer_irq as usize, Ordering::Release);
    ACTIVE_PLATFORM_INDEX.store(index, Ordering::Release);
    RESOURCE_SOURCE.store(resources.source as usize, Ordering::Release);
    INIT_STATE.store(1, Ordering::Release);
}

fn select_platform_index(compatible: &str) -> usize {
    for (index, platform) in PLATFORMS.iter().enumerate() {
        for candidate in platform.root_compatible {
            if dt_compatible_has(compatible, candidate) {
                return index;
            }
        }
    }
    DEFAULT_PLATFORM_INDEX
}

fn platform_resources(index: usize) -> Result<PlatformResources, DriverError> {
    let Some(platform) = PLATFORMS.get(index) else {
        return Err(DriverError::InvalidPlatform);
    };
    let uart = find_node_in_platform(platform, DeviceKind::Serial)?;
    let gic = find_node_in_platform(platform, DeviceKind::InterruptController)?;
    let timer = find_node_in_platform(platform, DeviceKind::Timer)?;
    let uart_reg = uart.primary_reg()?;
    let gicd_reg = gic.primary_reg()?;
    let gicc_reg = gic.secondary_reg()?;
    Ok(PlatformResources {
        machine: platform.machine,
        source: ResourceSource::StaticFallback,
        uart_base: uart_reg.base,
        uart_size: uart_reg.size,
        uart_irq: uart.irq()?,
        gicd_base: gicd_reg.base,
        gicd_size: gicd_reg.size,
        gicc_base: gicc_reg.base,
        gicc_size: gicc_reg.size,
        timer_irq: timer.irq()?,
    })
}

fn resource_source() -> ResourceSource {
    match RESOURCE_SOURCE.load(Ordering::Acquire) {
        value if value == ResourceSource::Fdt as usize => ResourceSource::Fdt,
        value if value == ResourceSource::StaticFallback as usize => ResourceSource::StaticFallback,
        _ => ResourceSource::Uninitialized,
    }
}

fn find_node_in_platform(
    platform: &'static PlatformDescriptor,
    kind: DeviceKind,
) -> Result<DeviceNode, DriverError> {
    for node in platform.nodes {
        if node.kind == kind {
            if !node.is_available() {
                return Err(DriverError::Disabled);
            }
            return Ok(*node);
        }
    }
    Err(DriverError::DeviceNotFound)
}

#[derive(Clone, Copy)]
struct FdtInfo {
    base: usize,
    struct_base: usize,
    strings_base: usize,
    struct_size: usize,
    strings_size: usize,
}

#[derive(Clone, Copy)]
struct FdtNodeState {
    address_cells: usize,
    size_cells: usize,
}

#[derive(Clone, Copy)]
struct FdtNodeScratch {
    depth: usize,
    parent_address_cells: usize,
    parent_size_cells: usize,
    address_cells: usize,
    size_cells: usize,
    enabled: bool,
    matched: DeviceKindMatch,
    reg: Option<DeviceReg>,
    reg2: Option<DeviceReg>,
    irq: Option<u32>,
    timer_irq: Option<u32>,
    compatible_addr: usize,
    compatible_len: usize,
    reg_addr: usize,
    reg_len: usize,
    interrupts_addr: usize,
    interrupts_len: usize,
}

impl FdtNodeScratch {
    fn new(depth: usize, parent: FdtNodeState) -> Self {
        Self {
            depth,
            parent_address_cells: parent.address_cells,
            parent_size_cells: parent.size_cells,
            address_cells: parent.address_cells,
            size_cells: parent.size_cells,
            enabled: true,
            matched: DeviceKindMatch::None,
            reg: None,
            reg2: None,
            irq: None,
            timer_irq: None,
            compatible_addr: 0,
            compatible_len: 0,
            reg_addr: 0,
            reg_len: 0,
            interrupts_addr: 0,
            interrupts_len: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeviceKindMatch {
    None,
    Serial,
    InterruptController,
    Timer,
}

#[derive(Clone, Copy)]
struct FdtParsedResources {
    machine: &'static str,
    uart_base: Option<usize>,
    uart_size: Option<usize>,
    uart_irq: Option<u32>,
    gicd_base: Option<usize>,
    gicd_size: Option<usize>,
    gicc_base: Option<usize>,
    gicc_size: Option<usize>,
    timer_irq: Option<u32>,
}

impl FdtParsedResources {
    fn new(machine: &'static str) -> Self {
        Self {
            machine,
            uart_base: None,
            uart_size: None,
            uart_irq: None,
            gicd_base: None,
            gicd_size: None,
            gicc_base: None,
            gicc_size: None,
            timer_irq: None,
        }
    }

    fn apply_node(&mut self, mut node: FdtNodeScratch) {
        if !node.enabled {
            return;
        }

        match node.matched {
            DeviceKindMatch::Serial => {
                if node.reg.is_none() && node.reg_addr != 0 {
                    node.reg = fdt_read_reg_tuple(
                        node.reg_addr,
                        node.reg_len,
                        0,
                        node.parent_address_cells,
                        node.parent_size_cells,
                    );
                }
                if node.irq.is_none() && node.interrupts_addr != 0 {
                    node.irq = fdt_read_interrupt(node.interrupts_addr, node.interrupts_len, 0);
                }
                if let (Some(reg), Some(irq)) = (node.reg, node.irq) {
                    self.uart_base = Some(reg.base);
                    self.uart_size = Some(reg.size);
                    self.uart_irq = Some(irq);
                }
            }
            DeviceKindMatch::InterruptController => {
                if node.reg.is_none() && node.reg_addr != 0 {
                    node.reg = fdt_read_reg_tuple(
                        node.reg_addr,
                        node.reg_len,
                        0,
                        node.parent_address_cells,
                        node.parent_size_cells,
                    );
                    node.reg2 = fdt_read_reg_tuple(
                        node.reg_addr,
                        node.reg_len,
                        1,
                        node.parent_address_cells,
                        node.parent_size_cells,
                    );
                }
                if let (Some(gicd), Some(gicc)) = (node.reg, node.reg2) {
                    self.gicd_base = Some(gicd.base);
                    self.gicd_size = Some(gicd.size);
                    self.gicc_base = Some(gicc.base);
                    self.gicc_size = Some(gicc.size);
                }
            }
            DeviceKindMatch::Timer => {
                if node.timer_irq.is_none() && node.interrupts_addr != 0 {
                    let tuple_bytes =
                        lowlevel_logic::fdt_reg_tuple_bytes(FDT_GIC_INTERRUPT_CELLS, 0);
                    if let Some(tuple_bytes) = tuple_bytes {
                        if tuple_bytes != 0 && node.interrupts_len % tuple_bytes == 0 {
                            let entry_count = node.interrupts_len / tuple_bytes;
                            node.timer_irq = fdt_read_interrupt(
                                node.interrupts_addr,
                                node.interrupts_len,
                                lowlevel_logic::dt_timer_irq_index(entry_count),
                            );
                            node.irq =
                                fdt_read_interrupt(node.interrupts_addr, node.interrupts_len, 0);
                        }
                    }
                }
                if let Some(irq) = node.timer_irq.or(node.irq) {
                    self.timer_irq = Some(irq);
                }
            }
            DeviceKindMatch::None => {}
        }
    }

    fn finish(self) -> Option<PlatformResources> {
        Some(PlatformResources {
            machine: self.machine,
            source: ResourceSource::Fdt,
            uart_base: self.uart_base?,
            uart_size: self.uart_size?,
            uart_irq: self.uart_irq?,
            gicd_base: self.gicd_base?,
            gicd_size: self.gicd_size?,
            gicc_base: self.gicc_base?,
            gicc_size: self.gicc_size?,
            timer_irq: self.timer_irq?,
        })
    }
}

fn fdt_platform_resources(fdt_base: usize) -> Option<PlatformResources> {
    let info = fdt_info(fdt_base)?;
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut parent_stack = [FdtNodeState {
        address_cells: FDT_DEFAULT_ADDRESS_CELLS,
        size_cells: FDT_DEFAULT_SIZE_CELLS,
    }; FDT_MAX_DEPTH];
    let mut node_stack = [None::<FdtNodeScratch>; FDT_MAX_DEPTH];
    let mut root_compatible: Option<&'static str> = None;
    let mut parsed = FdtParsedResources::new(FDT_GENERIC_MACHINE);

    while lowlevel_logic::fdt_range_valid(cursor, 4, info.struct_size) {
        let token = fdt_read_be_u32(fdt_addr(info.struct_base, cursor)?)?;
        cursor += 4;
        match token {
            FDT_BEGIN_NODE => {
                let name_start = cursor;
                while cursor < info.struct_size
                    && fdt_read_u8(fdt_addr(info.struct_base, cursor)?)? != 0
                {
                    cursor += 1;
                }
                if cursor >= info.struct_size || depth >= FDT_MAX_DEPTH {
                    return None;
                }
                let name_len = cursor - name_start;
                cursor += 1;
                cursor = lowlevel_logic::fdt_align4(cursor)?;
                if depth == 0 && name_len != 0 {
                    return None;
                }

                let parent = if depth == 0 {
                    FdtNodeState {
                        address_cells: FDT_DEFAULT_ADDRESS_CELLS,
                        size_cells: FDT_DEFAULT_SIZE_CELLS,
                    }
                } else {
                    parent_stack[depth - 1]
                };
                node_stack[depth] = Some(FdtNodeScratch::new(depth, parent));
                parent_stack[depth] = parent;
                depth += 1;
            }
            FDT_END_NODE => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                let Some(node) = node_stack[depth].take() else {
                    return None;
                };
                if node.depth == 0 {
                    if let Some(machine) = root_compatible {
                        parsed.machine = machine;
                    }
                } else {
                    parsed.apply_node(node);
                }
            }
            FDT_PROP => {
                if !lowlevel_logic::fdt_range_valid(cursor, 8, info.struct_size) || depth == 0 {
                    return None;
                }
                let prop_header = fdt_addr(info.struct_base, cursor)?;
                let len = fdt_read_be_u32(prop_header)? as usize;
                let nameoff = fdt_read_be_u32(fdt_addr(prop_header, 4)?)? as usize;
                cursor += 8;
                if !lowlevel_logic::fdt_range_valid(cursor, len, info.struct_size) {
                    return None;
                }
                let value_addr = fdt_addr(info.struct_base, cursor)?;
                let node_index = depth - 1;
                if let Some(mut node) = node_stack[node_index] {
                    handle_fdt_property(
                        &info,
                        &mut root_compatible,
                        &mut node,
                        nameoff,
                        value_addr,
                        len,
                    )?;
                    if node.depth == 0 {
                        if let Some(machine) = root_compatible {
                            parsed.machine = machine;
                        }
                    }
                    parent_stack[node_index] = FdtNodeState {
                        address_cells: node.address_cells,
                        size_cells: node.size_cells,
                    };
                    node_stack[node_index] = Some(node);
                }
                cursor += len;
                cursor = lowlevel_logic::fdt_align4(cursor)?;
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => return None,
        }
    }

    parsed.finish()
}

fn fdt_info(fdt_base: usize) -> Option<FdtInfo> {
    if fdt_base == 0 || fdt_base & 0x3 != 0 {
        return None;
    }

    let _header_end = lowlevel_logic::mmio_addr(fdt_base, FDT_HEADER_SIZE)?;
    let magic = fdt_read_be_u32(fdt_base)?;
    if magic != FDT_MAGIC {
        return None;
    }

    let totalsize = fdt_read_be_u32(fdt_addr(fdt_base, 4)?)? as usize;
    if totalsize < FDT_HEADER_SIZE || totalsize > FDT_MAX_SCAN_BYTES {
        return None;
    }

    let off_dt_struct = fdt_read_be_u32(fdt_addr(fdt_base, 8)?)? as usize;
    let off_dt_strings = fdt_read_be_u32(fdt_addr(fdt_base, 12)?)? as usize;
    let size_dt_strings = fdt_read_be_u32(fdt_addr(fdt_base, 32)?)? as usize;
    let size_dt_struct = fdt_read_be_u32(fdt_addr(fdt_base, 36)?)? as usize;
    let _blob_end = lowlevel_logic::mmio_addr(fdt_base, totalsize)?;
    if !lowlevel_logic::fdt_range_valid(off_dt_struct, size_dt_struct, totalsize)
        || !lowlevel_logic::fdt_range_valid(off_dt_strings, size_dt_strings, totalsize)
    {
        return None;
    }
    let struct_base = lowlevel_logic::mmio_addr(fdt_base, off_dt_struct)?;
    let strings_base = lowlevel_logic::mmio_addr(fdt_base, off_dt_strings)?;

    Some(FdtInfo {
        base: fdt_base,
        struct_base,
        strings_base,
        struct_size: size_dt_struct,
        strings_size: size_dt_strings,
    })
}

fn handle_fdt_property(
    info: &FdtInfo,
    root_compatible: &mut Option<&'static str>,
    node: &mut FdtNodeScratch,
    nameoff: usize,
    value_addr: usize,
    len: usize,
) -> Option<()> {
    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "compatible") {
        if node.depth == 0 {
            *root_compatible = fdt_compatible_value(value_addr, len);
        } else {
            node.compatible_addr = value_addr;
            node.compatible_len = len;
            node.matched = fdt_device_match(value_addr, len);
        }
        return Some(());
    }

    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "status") {
        node.enabled = !fdt_bytes_eq(value_addr, len, "disabled");
        return Some(());
    }

    if fdt_string_eq(
        info.strings_base,
        info.strings_size,
        nameoff,
        "#address-cells",
    ) {
        if let Some(cells) = fdt_read_cell_property(value_addr, len) {
            if cells <= FDT_MAX_ADDRESS_CELLS {
                node.address_cells = cells;
            }
        }
        return Some(());
    }

    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "#size-cells") {
        if let Some(cells) = fdt_read_cell_property(value_addr, len) {
            if cells <= FDT_MAX_SIZE_CELLS {
                node.size_cells = cells;
            }
        }
        return Some(());
    }

    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "reg") {
        node.reg_addr = value_addr;
        node.reg_len = len;
        node.reg = fdt_read_reg_tuple(
            value_addr,
            len,
            0,
            node.parent_address_cells,
            node.parent_size_cells,
        );
        node.reg2 = fdt_read_reg_tuple(
            value_addr,
            len,
            1,
            node.parent_address_cells,
            node.parent_size_cells,
        );
        return Some(());
    }

    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "interrupts") {
        node.interrupts_addr = value_addr;
        node.interrupts_len = len;
        let tuple_bytes = lowlevel_logic::fdt_reg_tuple_bytes(FDT_GIC_INTERRUPT_CELLS, 0)?;
        if tuple_bytes == 0 || len % tuple_bytes != 0 {
            return Some(());
        }
        let entry_count = len / tuple_bytes;
        node.irq = fdt_read_interrupt(value_addr, len, 0);
        node.timer_irq = fdt_read_interrupt(
            value_addr,
            len,
            lowlevel_logic::dt_timer_irq_index(entry_count),
        );
    }

    Some(())
}

fn fdt_device_match(value_addr: usize, len: usize) -> DeviceKindMatch {
    if fdt_compatible_list_has(value_addr, len, "arm,pl011") {
        DeviceKindMatch::Serial
    } else if fdt_compatible_list_has(value_addr, len, "arm,cortex-a15-gic")
        || fdt_compatible_list_has(value_addr, len, "arm,gic-400")
    {
        DeviceKindMatch::InterruptController
    } else if fdt_compatible_list_has(value_addr, len, "arm,armv8-timer") {
        DeviceKindMatch::Timer
    } else {
        DeviceKindMatch::None
    }
}

fn fdt_read_cell_property(value_addr: usize, len: usize) -> Option<usize> {
    if len != 4 {
        return None;
    }
    Some(fdt_read_be_u32(value_addr)? as usize)
}

fn fdt_read_reg_tuple(
    value_addr: usize,
    len: usize,
    index: usize,
    address_cells: usize,
    size_cells: usize,
) -> Option<DeviceReg> {
    if address_cells == 0
        || address_cells > FDT_MAX_ADDRESS_CELLS
        || size_cells == 0
        || size_cells > FDT_MAX_SIZE_CELLS
    {
        return None;
    }
    let tuple_offset = lowlevel_logic::fdt_reg_tuple_offset(index, address_cells, size_cells)?;
    let tuple_bytes = lowlevel_logic::fdt_reg_tuple_bytes(address_cells, size_cells)?;
    if tuple_bytes == 0
        || len % tuple_bytes != 0
        || !lowlevel_logic::fdt_range_valid(tuple_offset, tuple_bytes, len)
    {
        return None;
    }
    let tuple_addr = fdt_addr(value_addr, tuple_offset)?;
    let size_addr = fdt_addr(
        tuple_addr,
        lowlevel_logic::fdt_cells_to_bytes(address_cells)?,
    )?;
    let base = fdt_read_cells(tuple_addr, address_cells)?;
    let size = fdt_read_cells(size_addr, size_cells)?;
    if base > usize::MAX as u64 || size == 0 || size > usize::MAX as u64 {
        return None;
    }
    let reg = DeviceReg {
        base: base as usize,
        size: size as usize,
    };
    if lowlevel_logic::dt_reg_valid(reg.base, reg.size) {
        Some(reg)
    } else {
        None
    }
}

fn fdt_read_interrupt(value_addr: usize, len: usize, index: usize) -> Option<u32> {
    let tuple_offset = lowlevel_logic::fdt_reg_tuple_offset(index, FDT_GIC_INTERRUPT_CELLS, 0)?;
    let tuple_bytes = lowlevel_logic::fdt_reg_tuple_bytes(FDT_GIC_INTERRUPT_CELLS, 0)?;
    if tuple_bytes == 0
        || len % tuple_bytes != 0
        || !lowlevel_logic::fdt_range_valid(tuple_offset, tuple_bytes, len)
    {
        return None;
    }
    let tuple_addr = fdt_addr(value_addr, tuple_offset)?;
    let kind = fdt_read_be_u32(tuple_addr)?;
    let hwirq = fdt_read_be_u32(fdt_addr(tuple_addr, 4)?)?;
    lowlevel_logic::dt_gic_irq(kind, hwirq, MAX_PLATFORM_IRQS)
}

fn fdt_read_cells(value_addr: usize, cells: usize) -> Option<u64> {
    if cells == 0 || cells > FDT_MAX_ADDRESS_CELLS {
        return None;
    }
    let mut value = 0u64;
    for index in 0..cells {
        value = (value << 32) | fdt_read_be_u32(fdt_addr(value_addr, index * 4)?)? as u64;
    }
    Some(value)
}

fn fdt_compatible_value(value_addr: usize, len: usize) -> Option<&'static str> {
    for platform in PLATFORMS {
        for compatible in platform.root_compatible {
            if fdt_compatible_list_has(value_addr, len, compatible) {
                return Some(*compatible);
            }
        }
    }
    if fdt_string_list_valid(value_addr, len) {
        Some(FDT_GENERIC_MACHINE)
    } else {
        None
    }
}

fn fdt_string_list_valid(value_addr: usize, len: usize) -> bool {
    if len == 0 {
        return false;
    }

    let mut offset = 0usize;
    let mut saw_string = false;
    while offset < len {
        let start = offset;
        while offset < len {
            let Some(byte_addr) = fdt_addr(value_addr, offset) else {
                return false;
            };
            let Some(byte) = fdt_read_u8(byte_addr) else {
                return false;
            };
            if byte == 0 {
                break;
            }
            offset += 1;
        }
        if offset == len || offset == start {
            return false;
        }
        saw_string = true;
        offset += 1;
    }
    saw_string
}

fn fdt_compatible_list_has(value_addr: usize, len: usize, wanted: &str) -> bool {
    let mut offset = 0usize;
    while offset < len {
        let start = offset;
        while offset < len {
            let Some(byte_addr) = fdt_addr(value_addr, offset) else {
                return false;
            };
            let Some(byte) = fdt_read_u8(byte_addr) else {
                return false;
            };
            if byte == 0 {
                break;
            }
            offset += 1;
        }
        if offset == len {
            return false;
        }
        let Some(start_addr) = fdt_addr(value_addr, start) else {
            return false;
        };
        let candidate_len = offset - start;
        if fdt_bytes_eq(start_addr, candidate_len + 1, wanted) {
            return true;
        }
        offset += 1;
    }
    false
}

fn fdt_string_eq(strings_base: usize, strings_size: usize, nameoff: usize, wanted: &str) -> bool {
    if nameoff >= strings_size {
        return false;
    }
    let Some(name_addr) = fdt_addr(strings_base, nameoff) else {
        return false;
    };
    fdt_bytes_eq(name_addr, strings_size - nameoff, wanted)
}

fn fdt_bytes_eq(addr: usize, max_len: usize, wanted: &str) -> bool {
    let bytes = wanted.as_bytes();
    if bytes.len() >= max_len {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let Some(byte_addr) = fdt_addr(addr, index) else {
            return false;
        };
        let Some(actual) = fdt_read_u8(byte_addr) else {
            return false;
        };
        if actual != *byte {
            return false;
        }
    }
    match fdt_addr(addr, bytes.len()) {
        Some(term_addr) => fdt_read_u8(term_addr) == Some(0),
        None => false,
    }
}

fn dt_compatible_has(list: &str, wanted: &str) -> bool {
    let bytes = list.as_bytes();
    let wanted_bytes = wanted.as_bytes();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let start = offset;
        while offset < bytes.len() && bytes[offset] != 0 {
            offset += 1;
        }
        if offset - start == wanted_bytes.len() {
            let mut matched = true;
            for index in 0..wanted_bytes.len() {
                if bytes[start + index] != wanted_bytes[index] {
                    matched = false;
                    break;
                }
            }
            if matched {
                return true;
            }
        }
        if offset == bytes.len() {
            break;
        }
        offset += 1;
    }
    false
}

fn fdt_read_be_u32(addr: usize) -> Option<u32> {
    let b0 = fdt_read_u8(addr)? as u32;
    let b1 = fdt_read_u8(fdt_addr(addr, 1)?)? as u32;
    let b2 = fdt_read_u8(fdt_addr(addr, 2)?)? as u32;
    let b3 = fdt_read_u8(fdt_addr(addr, 3)?)? as u32;
    Some((b0 << 24) | (b1 << 16) | (b2 << 8) | b3)
}

fn fdt_addr(base: usize, offset: usize) -> Option<usize> {
    lowlevel_logic::mmio_addr(base, offset)
}

fn fdt_read_u8(addr: usize) -> Option<u8> {
    if addr == 0 {
        return None;
    }
    Some(unsafe { core::ptr::read_volatile(addr as *const u8) })
}

fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 10];
    let mut i = 0;
    while num > 0 && i < buf.len() {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}
