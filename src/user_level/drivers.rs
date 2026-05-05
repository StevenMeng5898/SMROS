//! User-space driver framework for SMROS bring-up.
//!
//! SMROS does not pass QEMU's live FDT into userspace yet, so this module keeps
//! a Linux-device-tree-shaped table for QEMU `virt` devices and binds drivers
//! against it. The block device is a real virtio-mmio block driver; attach a
//! persistent QEMU raw image with `-drive ...,if=none,id=fxfs -device
//! virtio-blk-device,drive=fxfs`.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use alloc::vec::Vec;
use core::mem::size_of;

const QEMU_VIRT_MACHINE: &str = "linux,dummy-virt";
const QEMU_VIRTIO_MMIO_BASE: usize = 0x0a00_0000;
const QEMU_VIRTIO_MMIO_STRIDE: usize = 0x200;
const QEMU_VIRTIO_MMIO_SLOT_COUNT: usize = 32;
const QEMU_VIRTIO_BLOCK_SIZE: usize = 512;
const QEMU_VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
const VIRTIO_BLK_F_CONFIG_WCE: u64 = 1 << 11;
const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;
const VIRTIO_STATUS_FEATURES_OK: u32 = 8;
const VIRTIO_STATUS_FAILED: u32 = 128;
const VIRTIO_DEVICE_ID_BLOCK: u32 = 2;
const VIRTIO_MAGIC_VALUE: u32 = 0x7472_6976;
const VIRTIO_MMIO_VERSION_LEGACY: u32 = 1;
const VIRTIO_MMIO_VERSION_MODERN: u32 = 2;
const VIRTIO_MMIO_VENDOR_QEMU: u32 = 0x554d_4551;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;
const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_QUEUE_SIZE: u16 = 8;
const VIRTIO_TIMEOUT_SPINS: usize = 10_000_000;

const REG_MAGIC_VALUE: usize = 0x000;
const REG_VERSION: usize = 0x004;
const REG_DEVICE_ID: usize = 0x008;
const REG_VENDOR_ID: usize = 0x00c;
const REG_DEVICE_FEATURES: usize = 0x010;
const REG_DEVICE_FEATURES_SEL: usize = 0x014;
const REG_DRIVER_FEATURES: usize = 0x020;
const REG_DRIVER_FEATURES_SEL: usize = 0x024;
const REG_GUEST_PAGE_SIZE: usize = 0x028;
const REG_QUEUE_SEL: usize = 0x030;
const REG_QUEUE_NUM_MAX: usize = 0x034;
const REG_QUEUE_NUM: usize = 0x038;
const REG_QUEUE_ALIGN: usize = 0x03c;
const REG_QUEUE_PFN: usize = 0x040;
const REG_QUEUE_READY: usize = 0x044;
const REG_QUEUE_NOTIFY: usize = 0x050;
const REG_INTERRUPT_STATUS: usize = 0x060;
const REG_INTERRUPT_ACK: usize = 0x064;
const REG_STATUS: usize = 0x070;
const REG_QUEUE_DESC_LOW: usize = 0x080;
const REG_QUEUE_DESC_HIGH: usize = 0x084;
const REG_QUEUE_DRIVER_LOW: usize = 0x090;
const REG_QUEUE_DRIVER_HIGH: usize = 0x094;
const REG_QUEUE_DEVICE_LOW: usize = 0x0a0;
const REG_QUEUE_DEVICE_HIGH: usize = 0x0a4;
const REG_CONFIG_GENERATION: usize = 0x0fc;
const REG_CONFIG_CAPACITY: usize = 0x100;

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
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(2))]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    ring: [u16; VIRTIO_QUEUE_SIZE as usize],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

#[repr(C, align(4))]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    ring: [VirtqUsedElem; VIRTIO_QUEUE_SIZE as usize],
}

#[repr(C, align(16))]
struct VirtioBlkReq {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

#[repr(C, align(4096))]
struct VirtioBlockQueue {
    desc: [VirtqDesc; VIRTIO_QUEUE_SIZE as usize],
    avail: VirtqAvail,
    _legacy_used_padding: [u8; 3948],
    used: VirtqUsed,
    req: VirtioBlkReq,
    status: u8,
    data: [u8; QEMU_VIRTIO_BLOCK_SIZE],
}

impl VirtioBlockQueue {
    const fn new() -> Self {
        Self {
            desc: [VirtqDesc {
                addr: 0,
                len: 0,
                flags: 0,
                next: 0,
            }; VIRTIO_QUEUE_SIZE as usize],
            avail: VirtqAvail {
                flags: 0,
                idx: 0,
                ring: [0; VIRTIO_QUEUE_SIZE as usize],
            },
            _legacy_used_padding: [0; 3948],
            used: VirtqUsed {
                flags: 0,
                idx: 0,
                ring: [VirtqUsedElem { id: 0, len: 0 }; VIRTIO_QUEUE_SIZE as usize],
            },
            req: VirtioBlkReq {
                request_type: 0,
                reserved: 0,
                sector: 0,
            },
            status: 0xff,
            data: [0; QEMU_VIRTIO_BLOCK_SIZE],
        }
    }
}

#[derive(Clone, Copy)]
struct QemuVirtBlockDriver {
    ready: bool,
    modern: bool,
    mmio_base: usize,
    flush_supported: bool,
    capacity_blocks: usize,
    last_used_idx: u16,
    reads: u64,
    writes: u64,
    flushes: u64,
    bytes_read: u64,
    bytes_written: u64,
    last_error: Option<UserDriverError>,
}

impl QemuVirtBlockDriver {
    const fn new() -> Self {
        Self {
            ready: false,
            modern: false,
            mmio_base: QEMU_VIRTIO_MMIO_BASE,
            flush_supported: false,
            capacity_blocks: 0,
            last_used_idx: 0,
            reads: 0,
            writes: 0,
            flushes: 0,
            bytes_read: 0,
            bytes_written: 0,
            last_error: None,
        }
    }

    fn bind(&mut self) -> Result<(), UserDriverError> {
        for slot in 0..QEMU_VIRTIO_MMIO_SLOT_COUNT {
            let base = QEMU_VIRTIO_MMIO_BASE + slot * QEMU_VIRTIO_MMIO_STRIDE;
            if self.bind_at(base).is_ok() {
                self.last_error = None;
                return Ok(());
            }
        }
        self.last_error = Some(UserDriverError::NotFound);
        Err(UserDriverError::NotFound)
    }

    fn bind_at(&mut self, base: usize) -> Result<(), UserDriverError> {
        if self.ready {
            return Ok(());
        }
        self.mmio_base = base;
        set_active_mmio_base(base);

        if mmio_read(REG_MAGIC_VALUE) != VIRTIO_MAGIC_VALUE
            || mmio_read(REG_DEVICE_ID) != VIRTIO_DEVICE_ID_BLOCK
            || mmio_read(REG_VENDOR_ID) != VIRTIO_MMIO_VENDOR_QEMU
        {
            return Err(UserDriverError::NotFound);
        }

        let version = mmio_read(REG_VERSION);
        if version != VIRTIO_MMIO_VERSION_MODERN && version != VIRTIO_MMIO_VERSION_LEGACY {
            return Err(UserDriverError::Unsupported);
        }
        self.modern = version == VIRTIO_MMIO_VERSION_MODERN;

        mmio_write(REG_STATUS, 0);
        mmio_write(REG_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);
        mmio_write(REG_STATUS, VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        let features = self.read_device_features();
        let accepted = features & (VIRTIO_BLK_F_FLUSH | VIRTIO_BLK_F_CONFIG_WCE);
        self.flush_supported = accepted & VIRTIO_BLK_F_FLUSH != 0;
        if self.modern {
            self.write_driver_features(accepted | QEMU_VIRTIO_F_VERSION_1);
        } else {
            self.write_driver_features(accepted);
        }

        if self.modern {
            self.add_status(VIRTIO_STATUS_FEATURES_OK);
            if mmio_read(REG_STATUS) & VIRTIO_STATUS_FEATURES_OK == 0 {
                self.fail();
                return Err(UserDriverError::Unsupported);
            }
        }

        mmio_write(REG_QUEUE_SEL, 0);
        let max_queue = mmio_read(REG_QUEUE_NUM_MAX);
        if max_queue == 0 || max_queue < VIRTIO_QUEUE_SIZE as u32 {
            self.fail();
            return Err(UserDriverError::Unsupported);
        }

        unsafe {
            VIRTIO_QUEUE = VirtioBlockQueue::new();
            let desc = (&raw const VIRTIO_QUEUE.desc) as *const _ as u64;
            let avail = (&raw const VIRTIO_QUEUE.avail) as *const _ as u64;
            let used = (&raw const VIRTIO_QUEUE.used) as *const _ as u64;

            mmio_write(REG_QUEUE_NUM, VIRTIO_QUEUE_SIZE as u32);
            if self.modern {
                mmio_write(REG_QUEUE_DESC_LOW, desc as u32);
                mmio_write(REG_QUEUE_DESC_HIGH, (desc >> 32) as u32);
                mmio_write(REG_QUEUE_DRIVER_LOW, avail as u32);
                mmio_write(REG_QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
                mmio_write(REG_QUEUE_DEVICE_LOW, used as u32);
                mmio_write(REG_QUEUE_DEVICE_HIGH, (used >> 32) as u32);
                mmio_write(REG_QUEUE_READY, 1);
            } else {
                let page = (desc / 4096) as u32;
                mmio_write(REG_GUEST_PAGE_SIZE, 4096);
                mmio_write(REG_QUEUE_ALIGN, 4096);
                mmio_write(REG_QUEUE_PFN, page);
            }
        }

        self.capacity_blocks = self.read_capacity_blocks();
        if self.capacity_blocks == 0 {
            self.capacity_blocks = 2048;
        }
        self.last_used_idx = unsafe { core::ptr::read_volatile(&raw const VIRTIO_QUEUE.used.idx) };
        self.add_status(VIRTIO_STATUS_DRIVER_OK);
        self.ready = true;
        Ok(())
    }

    fn read_device_features(&self) -> u64 {
        mmio_write(REG_DEVICE_FEATURES_SEL, 0);
        let low = mmio_read(REG_DEVICE_FEATURES) as u64;
        mmio_write(REG_DEVICE_FEATURES_SEL, 1);
        let high = mmio_read(REG_DEVICE_FEATURES) as u64;
        low | (high << 32)
    }

    fn write_driver_features(&self, features: u64) {
        mmio_write(REG_DRIVER_FEATURES_SEL, 0);
        mmio_write(REG_DRIVER_FEATURES, features as u32);
        mmio_write(REG_DRIVER_FEATURES_SEL, 1);
        mmio_write(REG_DRIVER_FEATURES, (features >> 32) as u32);
    }

    fn add_status(&self, status: u32) {
        let current = mmio_read(REG_STATUS);
        mmio_write(REG_STATUS, current | status);
    }

    fn fail(&self) {
        let current = mmio_read(REG_STATUS);
        mmio_write(REG_STATUS, current | VIRTIO_STATUS_FAILED);
    }

    fn read_capacity_blocks(&self) -> usize {
        let before = mmio_read(REG_CONFIG_GENERATION);
        let low = mmio_read(REG_CONFIG_CAPACITY) as u64;
        let high = mmio_read(REG_CONFIG_CAPACITY + 4) as u64;
        let after = mmio_read(REG_CONFIG_GENERATION);
        let capacity = if before == after {
            low | (high << 32)
        } else {
            mmio_read(REG_CONFIG_CAPACITY) as u64
                | ((mmio_read(REG_CONFIG_CAPACITY + 4) as u64) << 32)
        };
        core::cmp::min(capacity, usize::MAX as u64) as usize
    }

    fn ensure_ready(&self) -> Result<(), UserDriverError> {
        if self.ready {
            Ok(())
        } else {
            Err(UserDriverError::NotReady)
        }
    }

    fn read_at(&mut self, offset: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
        if let Err(err) = self.ensure_ready() {
            self.last_error = Some(err);
            return Err(err);
        }
        if let Err(err) = self.check_range(offset, out.len()) {
            self.last_error = Some(err);
            return Err(err);
        }
        let mut done = 0usize;
        while done < out.len() {
            let block = (offset + done) / QEMU_VIRTIO_BLOCK_SIZE;
            let block_offset = (offset + done) % QEMU_VIRTIO_BLOCK_SIZE;
            let len = core::cmp::min(QEMU_VIRTIO_BLOCK_SIZE - block_offset, out.len() - done);
            if let Err(err) = self.read_block(block, unsafe { &mut VIRTIO_QUEUE.data }) {
                self.last_error = Some(err);
                return Err(err);
            }
            unsafe {
                out[done..done + len]
                    .copy_from_slice(&VIRTIO_QUEUE.data[block_offset..block_offset + len]);
            }
            done += len;
        }
        self.reads = self.reads.saturating_add(1);
        self.bytes_read = self.bytes_read.saturating_add(out.len() as u64);
        self.last_error = None;
        Ok(out.len())
    }

    fn write_at(&mut self, offset: usize, data: &[u8]) -> Result<usize, UserDriverError> {
        if let Err(err) = self.ensure_ready() {
            self.last_error = Some(err);
            return Err(err);
        }
        if let Err(err) = self.check_range(offset, data.len()) {
            self.last_error = Some(err);
            return Err(err);
        }
        let mut done = 0usize;
        while done < data.len() {
            let block = (offset + done) / QEMU_VIRTIO_BLOCK_SIZE;
            let block_offset = (offset + done) % QEMU_VIRTIO_BLOCK_SIZE;
            let len = core::cmp::min(QEMU_VIRTIO_BLOCK_SIZE - block_offset, data.len() - done);
            if block_offset != 0 || len != QEMU_VIRTIO_BLOCK_SIZE {
                if let Err(err) = self.read_block(block, unsafe { &mut VIRTIO_QUEUE.data }) {
                    self.last_error = Some(err);
                    return Err(err);
                }
            }
            unsafe {
                VIRTIO_QUEUE.data[block_offset..block_offset + len]
                    .copy_from_slice(&data[done..done + len]);
            }
            if let Err(err) = self.submit(
                VIRTIO_BLK_T_OUT,
                block as u64,
                unsafe { VIRTIO_QUEUE.data.as_mut_ptr() },
                QEMU_VIRTIO_BLOCK_SIZE,
            ) {
                self.last_error = Some(err);
                return Err(err);
            }
            done += len;
        }
        self.writes = self.writes.saturating_add(1);
        self.bytes_written = self.bytes_written.saturating_add(data.len() as u64);
        self.last_error = None;
        Ok(data.len())
    }

    fn read_block(&mut self, block: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
        if out.len() != QEMU_VIRTIO_BLOCK_SIZE {
            return Err(UserDriverError::InvalidBlock);
        }
        if block >= self.capacity_blocks {
            return Err(UserDriverError::OutOfRange);
        }
        if let Err(err) = self.submit(VIRTIO_BLK_T_IN, block as u64, out.as_mut_ptr(), out.len()) {
            self.last_error = Some(err);
            return Err(err);
        }
        self.last_error = None;
        Ok(out.len())
    }

    fn write_block(&mut self, block: usize, data: &[u8]) -> Result<usize, UserDriverError> {
        if data.len() != QEMU_VIRTIO_BLOCK_SIZE {
            return Err(UserDriverError::InvalidBlock);
        }
        if block >= self.capacity_blocks {
            return Err(UserDriverError::OutOfRange);
        }
        unsafe {
            VIRTIO_QUEUE.data.copy_from_slice(data);
        }
        if let Err(err) = self.submit(
            VIRTIO_BLK_T_OUT,
            block as u64,
            unsafe { VIRTIO_QUEUE.data.as_mut_ptr() },
            QEMU_VIRTIO_BLOCK_SIZE,
        ) {
            self.last_error = Some(err);
            return Err(err);
        }
        self.last_error = None;
        Ok(data.len())
    }

    fn clear(&mut self) -> Result<(), UserDriverError> {
        self.ensure_ready()?;
        unsafe {
            VIRTIO_QUEUE.data.fill(0);
        }
        for block in 0..self.capacity_blocks {
            if let Err(err) = self.submit(
                VIRTIO_BLK_T_OUT,
                block as u64,
                unsafe { VIRTIO_QUEUE.data.as_mut_ptr() },
                QEMU_VIRTIO_BLOCK_SIZE,
            ) {
                self.last_error = Some(err);
                return Err(err);
            }
        }
        self.writes = self.writes.saturating_add(1);
        self.bytes_written = self
            .bytes_written
            .saturating_add((self.capacity_blocks * QEMU_VIRTIO_BLOCK_SIZE) as u64);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), UserDriverError> {
        self.ensure_ready()?;
        if self.flush_supported {
            if let Err(err) = self.submit(VIRTIO_BLK_T_FLUSH, 0, core::ptr::null_mut(), 0) {
                self.last_error = Some(err);
                return Err(err);
            }
        } else {
            memory_barrier();
        }
        self.flushes = self.flushes.saturating_add(1);
        self.last_error = None;
        Ok(())
    }

    fn submit(
        &mut self,
        request_type: u32,
        sector: u64,
        data_ptr: *mut u8,
        data_len: usize,
    ) -> Result<(), UserDriverError> {
        unsafe {
            VIRTIO_QUEUE.req.request_type = request_type;
            VIRTIO_QUEUE.req.reserved = 0;
            VIRTIO_QUEUE.req.sector = sector;
            VIRTIO_QUEUE.status = 0xff;

            VIRTIO_QUEUE.desc[0] = VirtqDesc {
                addr: (&raw const VIRTIO_QUEUE.req) as *const _ as u64,
                len: size_of::<VirtioBlkReq>() as u32,
                flags: VIRTQ_DESC_F_NEXT,
                next: if data_len == 0 { 2 } else { 1 },
            };
            VIRTIO_QUEUE.desc[1] = VirtqDesc {
                addr: data_ptr as u64,
                len: data_len as u32,
                flags: if request_type == VIRTIO_BLK_T_IN {
                    VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT
                } else {
                    VIRTQ_DESC_F_NEXT
                },
                next: 2,
            };
            VIRTIO_QUEUE.desc[2] = VirtqDesc {
                addr: (&raw mut VIRTIO_QUEUE.status) as *mut _ as u64,
                len: 1,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            };

            let slot = (VIRTIO_QUEUE.avail.idx % VIRTIO_QUEUE_SIZE) as usize;
            VIRTIO_QUEUE.avail.ring[slot] = 0;
            memory_barrier();
            VIRTIO_QUEUE.avail.idx = VIRTIO_QUEUE.avail.idx.wrapping_add(1);
            memory_barrier();
            mmio_write(REG_QUEUE_NOTIFY, 0);

            let target = self.last_used_idx.wrapping_add(1);
            for _ in 0..VIRTIO_TIMEOUT_SPINS {
                memory_barrier();
                if core::ptr::read_volatile(&raw const VIRTIO_QUEUE.used.idx) == target {
                    self.last_used_idx = target;
                    mmio_write(REG_INTERRUPT_ACK, mmio_read(REG_INTERRUPT_STATUS));
                    return if core::ptr::read_volatile(&raw const VIRTIO_QUEUE.status)
                        == VIRTIO_BLK_S_OK
                    {
                        Ok(())
                    } else {
                        Err(UserDriverError::Io)
                    };
                }
            }
        }
        Err(UserDriverError::Timeout)
    }

    fn check_range(&self, offset: usize, len: usize) -> Result<(), UserDriverError> {
        let end = offset.checked_add(len).ok_or(UserDriverError::OutOfRange)?;
        let capacity = self.capacity_blocks * QEMU_VIRTIO_BLOCK_SIZE;
        if end > capacity {
            Err(UserDriverError::OutOfRange)
        } else {
            Ok(())
        }
    }
}

struct UserDriverFramework {
    initialized: bool,
    nodes: Vec<UserDeviceNode>,
    bindings: Vec<UserDriverBinding>,
    block: QemuVirtBlockDriver,
}

impl UserDriverFramework {
    fn new() -> Self {
        Self {
            initialized: false,
            nodes: Vec::new(),
            bindings: Vec::new(),
            block: QemuVirtBlockDriver::new(),
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
        self.initialized = self.block.ready;
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
                base: QEMU_VIRTIO_MMIO_BASE as u64,
                size: (QEMU_VIRTIO_MMIO_STRIDE * QEMU_VIRTIO_MMIO_SLOT_COUNT) as u64,
            }),
            irq: Some(48),
        });
    }

    fn probe(&mut self) {
        for node in &self.nodes {
            if node.compatible == "virtio,mmio"
                && node.status == "okay"
                && node.kind == UserDeviceKind::VirtioMmio
                && self.block.bind().is_ok()
            {
                self.bindings.push(UserDriverBinding {
                    node_path: node.path,
                    driver: "qemu-virtio-mmio-block",
                    device_name: "vblk0",
                    kind: UserDeviceKind::Block,
                    block_size: QEMU_VIRTIO_BLOCK_SIZE,
                    block_count: self.block.capacity_blocks,
                });
            }
        }
    }

    fn stats(&self) -> UserDriverStats {
        let block_count = self.block.capacity_blocks;
        UserDriverStats {
            initialized: self.initialized,
            machine: QEMU_VIRT_MACHINE,
            nodes: self.nodes.len(),
            bindings: self.bindings.len(),
            block_ready: self.block.ready,
            mmio_base: self.block.mmio_base,
            device_status: if self.block.ready {
                set_active_mmio_base(self.block.mmio_base);
                mmio_read(REG_STATUS)
            } else {
                0
            },
            last_error: self.block.last_error,
            block_size: QEMU_VIRTIO_BLOCK_SIZE,
            block_count,
            bytes: block_count.saturating_mul(QEMU_VIRTIO_BLOCK_SIZE),
            reads: self.block.reads,
            writes: self.block.writes,
            flushes: self.block.flushes,
            bytes_read: self.block.bytes_read,
            bytes_written: self.block.bytes_written,
        }
    }
}

static mut DRIVER_FRAMEWORK: Option<UserDriverFramework> = None;
static mut VIRTIO_QUEUE: VirtioBlockQueue = VirtioBlockQueue::new();
static mut ACTIVE_VIRTIO_MMIO_BASE: usize = QEMU_VIRTIO_MMIO_BASE;

fn framework() -> &'static mut UserDriverFramework {
    unsafe {
        if DRIVER_FRAMEWORK.is_none() {
            DRIVER_FRAMEWORK = Some(UserDriverFramework::new());
        }
        DRIVER_FRAMEWORK.as_mut().unwrap()
    }
}

fn set_active_mmio_base(base: usize) {
    unsafe {
        ACTIVE_VIRTIO_MMIO_BASE = base;
    }
}

fn active_mmio_base() -> usize {
    unsafe { ACTIVE_VIRTIO_MMIO_BASE }
}

fn mmio_read(offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((active_mmio_base() + offset) as *const u32) }
}

fn mmio_write(offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((active_mmio_base() + offset) as *mut u32, value) }
}

fn memory_barrier() {
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
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
    framework().block.ready
}

pub fn block_size() -> usize {
    QEMU_VIRTIO_BLOCK_SIZE
}

pub fn block_count() -> usize {
    framework().block.capacity_blocks
}

pub fn block_capacity() -> usize {
    block_count().saturating_mul(QEMU_VIRTIO_BLOCK_SIZE)
}

pub fn block_read_at(offset: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.read_at(offset, out)
}

pub fn block_write_at(offset: usize, data: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.write_at(offset, data)
}

pub fn block_read(block: usize, out: &mut [u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.read_block(block, out)
}

pub fn block_write(block: usize, data: &[u8]) -> Result<usize, UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.write_block(block, data)
}

pub fn block_clear() -> Result<(), UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.clear()
}

pub fn block_flush() -> Result<(), UserDriverError> {
    if !framework().initialized && !framework().init() {
        return Err(UserDriverError::NotInitialized);
    }
    framework().block.flush()
}

pub fn smoke_test() -> bool {
    if !init() || !block_ready() || block_count() < 2 {
        return false;
    }
    let mut block = [0u8; QEMU_VIRTIO_BLOCK_SIZE];
    if block_read(1, &mut block).is_err() {
        return false;
    }
    let saved = block;
    block[0..11].copy_from_slice(b"smros-block");
    if block_write(1, &block).is_err() {
        return false;
    }
    let mut out = [0u8; QEMU_VIRTIO_BLOCK_SIZE];
    let ok = block_read(1, &mut out).is_ok() && out[0..11] == *b"smros-block";
    let _ = block_write(1, &saved);
    ok && block_flush().is_ok()
}
