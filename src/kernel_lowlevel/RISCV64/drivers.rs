#![allow(dead_code)]
//! RISC-V64 platform resource discovery.
//!
//! RISC-V boards, including Kunminghu-style QEMU machines, should describe
//! UARTs, CPU harts, and timer frequency in the FDT handed over by firmware.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::lowlevel_logic;

pub const MAX_HARTS: usize = 64;
pub const MAX_VIRTIO_MMIO_TRANSPORTS: usize = 32;
const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;
const FDT_HEADER_SIZE: usize = 40;
const FDT_MAX_SCAN_BYTES: usize = 0x20_0000;
const FDT_MAX_DEPTH: usize = 16;
const FDT_DEFAULT_ADDRESS_CELLS: usize = 2;
const FDT_DEFAULT_SIZE_CELLS: usize = 1;
const FDT_MAX_ADDRESS_CELLS: usize = 2;
const FDT_MAX_SIZE_CELLS: usize = 2;
const DEFAULT_TIMEBASE_HZ: usize = 10_000_000;
const DEFAULT_MACHINE: &str = "fdt,generic-riscv64";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceSource {
    Uninitialized,
    Fdt,
}

impl ResourceSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceSource::Uninitialized => "uninitialized",
            ResourceSource::Fdt => "fdt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceReg {
    pub base: usize,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverStats {
    pub initialized: bool,
    pub machine: &'static str,
    pub source: ResourceSource,
    pub uart_base: usize,
    pub uart_size: usize,
    pub timebase_frequency: u64,
    pub hart_count: usize,
    pub virtio_mmio_count: usize,
}

#[derive(Clone, Copy)]
struct FdtInfo {
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
    compatible_addr: usize,
    compatible_len: usize,
    reg: Option<DeviceReg>,
    hart_id: Option<usize>,
    device_type_cpu: bool,
    timebase_frequency: Option<u64>,
    virtio_mmio: bool,
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
            compatible_addr: 0,
            compatible_len: 0,
            reg: None,
            hart_id: None,
            device_type_cpu: false,
            timebase_frequency: None,
            virtio_mmio: false,
        }
    }
}

#[derive(Clone, Copy)]
struct ParsedResources {
    machine: &'static str,
    uart: Option<DeviceReg>,
    timebase_frequency: Option<u64>,
    harts: [usize; MAX_HARTS],
    hart_count: usize,
    virtio_mmio: [DeviceReg; MAX_VIRTIO_MMIO_TRANSPORTS],
    virtio_mmio_count: usize,
}

impl ParsedResources {
    const fn new() -> Self {
        Self {
            machine: DEFAULT_MACHINE,
            uart: None,
            timebase_frequency: None,
            harts: [0; MAX_HARTS],
            hart_count: 0,
            virtio_mmio: [DeviceReg { base: 0, size: 0 }; MAX_VIRTIO_MMIO_TRANSPORTS],
            virtio_mmio_count: 0,
        }
    }

    fn apply_node(&mut self, node: FdtNodeScratch) {
        if !node.enabled {
            return;
        }

        if let Some(freq) = node.timebase_frequency {
            self.timebase_frequency = Some(freq);
        }

        if node.device_type_cpu || fdt_node_compatible_has(node, "riscv") {
            if let Some(hart_id) = node.hart_id {
                self.push_hart(hart_id);
            }
        }

        if fdt_node_compatible_has(node, "ns16550a")
            || fdt_node_compatible_has(node, "ns16550")
            || fdt_node_compatible_has(node, "sifive,uart0")
        {
            if let Some(reg) = node.reg {
                self.uart = Some(reg);
            }
        }

        if node.virtio_mmio || fdt_node_compatible_has(node, "virtio,mmio") {
            if let Some(reg) = node.reg {
                self.push_virtio_mmio(reg);
            }
        }
    }

    fn push_hart(&mut self, hart_id: usize) {
        for index in 0..self.hart_count {
            if self.harts[index] == hart_id {
                return;
            }
        }
        if self.hart_count < MAX_HARTS {
            self.harts[self.hart_count] = hart_id;
            self.hart_count += 1;
        }
    }

    fn push_virtio_mmio(&mut self, reg: DeviceReg) {
        for index in 0..self.virtio_mmio_count {
            if self.virtio_mmio[index].base == reg.base {
                return;
            }
        }
        if self.virtio_mmio_count < MAX_VIRTIO_MMIO_TRANSPORTS {
            self.virtio_mmio[self.virtio_mmio_count] = reg;
            self.virtio_mmio_count += 1;
        }
    }
}

static INIT_STATE: AtomicUsize = AtomicUsize::new(0);
static RESOURCE_SOURCE: AtomicUsize = AtomicUsize::new(ResourceSource::Uninitialized as usize);
static UART_BASE: AtomicUsize = AtomicUsize::new(0);
static UART_SIZE: AtomicUsize = AtomicUsize::new(0);
static TIMEBASE_FREQUENCY: AtomicUsize = AtomicUsize::new(DEFAULT_TIMEBASE_HZ);
static HART_COUNT: AtomicUsize = AtomicUsize::new(1);
static VIRTIO_MMIO_COUNT: AtomicUsize = AtomicUsize::new(0);
static MACHINE_INDEX: AtomicUsize = AtomicUsize::new(0);
static mut HART_IDS: [usize; MAX_HARTS] = [0; MAX_HARTS];
static mut VIRTIO_MMIO_REGS: [DeviceReg; MAX_VIRTIO_MMIO_TRANSPORTS] =
    [DeviceReg { base: 0, size: 0 }; MAX_VIRTIO_MMIO_TRANSPORTS];
static mut MACHINE_NAME: [u8; 64] = [0; 64];

pub fn init() -> bool {
    INIT_STATE.load(Ordering::Acquire) != 0
}

pub fn init_from_fdt(fdt_base: usize) -> bool {
    let Some(parsed) = fdt_parse_resources(fdt_base) else {
        INIT_STATE.store(0, Ordering::Release);
        return false;
    };
    let Some(uart) = parsed.uart else {
        INIT_STATE.store(0, Ordering::Release);
        return false;
    };

    UART_BASE.store(uart.base, Ordering::Release);
    UART_SIZE.store(uart.size, Ordering::Release);
    TIMEBASE_FREQUENCY.store(
        parsed
            .timebase_frequency
            .unwrap_or(DEFAULT_TIMEBASE_HZ as u64) as usize,
        Ordering::Release,
    );
    HART_COUNT.store(core::cmp::max(parsed.hart_count, 1), Ordering::Release);
    VIRTIO_MMIO_COUNT.store(parsed.virtio_mmio_count, Ordering::Release);
    RESOURCE_SOURCE.store(ResourceSource::Fdt as usize, Ordering::Release);
    MACHINE_INDEX.store(copy_machine_name(parsed.machine), Ordering::Release);

    unsafe {
        HART_IDS = parsed.harts;
        VIRTIO_MMIO_REGS = parsed.virtio_mmio;
    }
    INIT_STATE.store(1, Ordering::Release);
    true
}

pub fn architecture_name() -> &'static str {
    "RISCV64"
}

pub fn uart_base() -> usize {
    UART_BASE.load(Ordering::Acquire)
}

pub fn uart_size() -> usize {
    UART_SIZE.load(Ordering::Acquire)
}

pub fn timebase_frequency() -> u64 {
    TIMEBASE_FREQUENCY.load(Ordering::Acquire) as u64
}

pub fn hart_count() -> usize {
    HART_COUNT.load(Ordering::Acquire)
}

pub fn virtio_mmio_count() -> usize {
    VIRTIO_MMIO_COUNT.load(Ordering::Acquire)
}

pub fn virtio_mmio_reg(index: usize) -> Option<DeviceReg> {
    if index >= virtio_mmio_count() || index >= MAX_VIRTIO_MMIO_TRANSPORTS {
        None
    } else {
        Some(unsafe { VIRTIO_MMIO_REGS[index] })
    }
}

pub fn hart_id(index: usize) -> Option<usize> {
    if index >= hart_count() || index >= MAX_HARTS {
        None
    } else {
        Some(unsafe { HART_IDS[index] })
    }
}

pub fn hart_index(hart_id: usize) -> usize {
    let count = core::cmp::min(hart_count(), MAX_HARTS);
    for index in 0..count {
        if unsafe { HART_IDS[index] } == hart_id {
            return index;
        }
    }
    hart_id
}

pub fn stats() -> DriverStats {
    DriverStats {
        initialized: INIT_STATE.load(Ordering::Acquire) != 0,
        machine: machine_name(),
        source: resource_source(),
        uart_base: uart_base(),
        uart_size: uart_size(),
        timebase_frequency: timebase_frequency(),
        hart_count: hart_count(),
        virtio_mmio_count: virtio_mmio_count(),
    }
}

pub fn describe(serial: &mut crate::kernel_lowlevel::serial::Serial) {
    let snapshot = stats();
    serial.write_str("[DRV] Platform: ");
    serial.write_str(snapshot.machine);
    serial.write_str(" source=");
    serial.write_str(snapshot.source.as_str());
    serial.write_str(" uart=0x");
    serial.write_hex(snapshot.uart_base as u64);
    serial.write_str(" timebase=");
    print_number(serial, snapshot.timebase_frequency as u32);
    serial.write_str("Hz harts=");
    print_number(serial, snapshot.hart_count as u32);
    serial.write_str(" virtio-mmio=");
    print_number(serial, snapshot.virtio_mmio_count as u32);
    serial.write_str("\n");
}

fn resource_source() -> ResourceSource {
    match RESOURCE_SOURCE.load(Ordering::Acquire) {
        value if value == ResourceSource::Fdt as usize => ResourceSource::Fdt,
        _ => ResourceSource::Uninitialized,
    }
}

fn copy_machine_name(machine: &'static str) -> usize {
    let bytes = machine.as_bytes();
    let len = core::cmp::min(bytes.len(), 63);
    unsafe {
        for index in 0..len {
            MACHINE_NAME[index] = bytes[index];
        }
        MACHINE_NAME[len] = 0;
    }
    len
}

fn machine_name() -> &'static str {
    if MACHINE_INDEX.load(Ordering::Acquire) == 0 {
        return DEFAULT_MACHINE;
    }
    let len = MACHINE_INDEX.load(Ordering::Acquire);
    unsafe { core::str::from_utf8_unchecked(&MACHINE_NAME[..len]) }
}

fn fdt_parse_resources(fdt_base: usize) -> Option<ParsedResources> {
    let info = fdt_info(fdt_base)?;
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut parent_stack = [FdtNodeState {
        address_cells: FDT_DEFAULT_ADDRESS_CELLS,
        size_cells: FDT_DEFAULT_SIZE_CELLS,
    }; FDT_MAX_DEPTH];
    let mut node_stack = [None::<FdtNodeScratch>; FDT_MAX_DEPTH];
    let mut parsed = ParsedResources::new();

    while lowlevel_logic::fdt_range_valid(cursor, 4, info.struct_size) {
        let token = fdt_read_be_u32(fdt_addr(info.struct_base, cursor)?)?;
        cursor += 4;
        match token {
            FDT_BEGIN_NODE => {
                while cursor < info.struct_size
                    && fdt_read_u8(fdt_addr(info.struct_base, cursor)?)? != 0
                {
                    cursor += 1;
                }
                if cursor >= info.struct_size || depth >= FDT_MAX_DEPTH {
                    return None;
                }
                cursor += 1;
                cursor = lowlevel_logic::fdt_align4(cursor)?;

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
                let node = node_stack[depth].take()?;
                if node.depth == 0 {
                    if node.compatible_addr != 0
                        && fdt_string_list_valid(node.compatible_addr, node.compatible_len)
                    {
                        parsed.machine = DEFAULT_MACHINE;
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
                    handle_fdt_property(&info, &mut node, nameoff, value_addr, len)?;
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

    Some(parsed)
}

fn fdt_info(fdt_base: usize) -> Option<FdtInfo> {
    if fdt_base == 0 || fdt_base & 0x3 != 0 {
        return None;
    }
    let _header_end = lowlevel_logic::mmio_addr(fdt_base, FDT_HEADER_SIZE)?;
    if fdt_read_be_u32(fdt_base)? != FDT_MAGIC {
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
    if !lowlevel_logic::fdt_range_valid(off_dt_struct, size_dt_struct, totalsize)
        || !lowlevel_logic::fdt_range_valid(off_dt_strings, size_dt_strings, totalsize)
    {
        return None;
    }
    Some(FdtInfo {
        struct_base: lowlevel_logic::mmio_addr(fdt_base, off_dt_struct)?,
        strings_base: lowlevel_logic::mmio_addr(fdt_base, off_dt_strings)?,
        struct_size: size_dt_struct,
        strings_size: size_dt_strings,
    })
}

fn handle_fdt_property(
    info: &FdtInfo,
    node: &mut FdtNodeScratch,
    nameoff: usize,
    value_addr: usize,
    len: usize,
) -> Option<()> {
    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "compatible") {
        node.compatible_addr = value_addr;
        node.compatible_len = len;
        node.virtio_mmio = fdt_compatible_list_has(value_addr, len, "virtio,mmio");
        return Some(());
    }
    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "status") {
        node.enabled = !fdt_bytes_eq(value_addr, len, "disabled");
        return Some(());
    }
    if fdt_string_eq(info.strings_base, info.strings_size, nameoff, "device_type") {
        node.device_type_cpu = fdt_bytes_eq(value_addr, len, "cpu");
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
        node.reg = fdt_read_reg_tuple(
            value_addr,
            len,
            0,
            node.parent_address_cells,
            node.parent_size_cells,
        );
        if let Some(reg) = node.reg {
            node.hart_id = Some(reg.base);
        }
        return Some(());
    }
    if fdt_string_eq(
        info.strings_base,
        info.strings_size,
        nameoff,
        "timebase-frequency",
    ) {
        if let Some(freq) = fdt_read_cell_property(value_addr, len) {
            node.timebase_frequency = Some(freq as u64);
        }
    }
    Some(())
}

fn fdt_node_compatible_has(node: FdtNodeScratch, wanted: &str) -> bool {
    node.compatible_addr != 0
        && fdt_compatible_list_has(node.compatible_addr, node.compatible_len, wanted)
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
    let size = if size_cells == 0 {
        1
    } else {
        fdt_read_cells(size_addr, size_cells)?
    };
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
