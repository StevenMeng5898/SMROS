//! VirtIO-MMIO network driver for QEMU `virt`.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::UserDriverError;

pub const MMIO_BASE: usize = 0x0a00_0000;
pub const MMIO_STRIDE: usize = 0x200;
pub const MMIO_SLOT_COUNT: usize = 32;
pub const ETHERNET_MTU: usize = 1500;
pub const ETHERNET_HEADER_LEN: usize = 14;
pub const ETHERNET_FRAME_MAX: usize = ETHERNET_HEADER_LEN + ETHERNET_MTU;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_NET_F_MAC: u64 = 1 << 5;
const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;
const VIRTIO_STATUS_FEATURES_OK: u32 = 8;
const VIRTIO_STATUS_FAILED: u32 = 128;
const VIRTIO_DEVICE_ID_NET: u32 = 1;
const VIRTIO_MAGIC_VALUE: u32 = 0x7472_6976;
const VIRTIO_MMIO_VERSION_LEGACY: u32 = 1;
const VIRTIO_MMIO_VERSION_MODERN: u32 = 2;
const VIRTIO_MMIO_VENDOR_QEMU: u32 = 0x554d_4551;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const VIRTIO_QUEUE_SIZE: u16 = 8;
const VIRTIO_NET_HDR_LEN: usize = 10;
const NET_BUFFER_SIZE: usize = 2048;
const NET_TX_TIMEOUT_SPINS: usize = 10_000_000;
const NET_POLL_TIMEOUT_SPINS: usize = 1_000_000;
const VIRTIO_NET_S_LINK_UP: u16 = 1;

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
const REG_CONFIG: usize = 0x100;
const CONFIG_MAC: usize = 0;
const CONFIG_STATUS: usize = 6;

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

#[repr(C, align(4096))]
struct VirtioNetQueue {
    desc: [VirtqDesc; VIRTIO_QUEUE_SIZE as usize],
    avail: VirtqAvail,
    _legacy_used_padding: [u8; 3948],
    used: VirtqUsed,
}

impl VirtioNetQueue {
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
        }
    }
}

#[derive(Clone, Copy)]
struct QemuVirtNetDriver {
    ready: bool,
    modern: bool,
    mmio_base: usize,
    mac: [u8; 6],
    status_supported: bool,
    link_up: bool,
    rx_last_used_idx: u16,
    tx_last_used_idx: u16,
    rx_packets: u64,
    tx_packets: u64,
    rx_bytes: u64,
    tx_bytes: u64,
    dropped_packets: u64,
    last_error: Option<UserDriverError>,
}

impl QemuVirtNetDriver {
    const fn new() -> Self {
        Self {
            ready: false,
            modern: false,
            mmio_base: MMIO_BASE,
            mac: [0; 6],
            status_supported: false,
            link_up: false,
            rx_last_used_idx: 0,
            tx_last_used_idx: 0,
            rx_packets: 0,
            tx_packets: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            dropped_packets: 0,
            last_error: None,
        }
    }

    fn bind(&mut self) -> Result<(), UserDriverError> {
        for slot in 0..MMIO_SLOT_COUNT {
            let base = MMIO_BASE + slot * MMIO_STRIDE;
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
            || mmio_read(REG_DEVICE_ID) != VIRTIO_DEVICE_ID_NET
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
        let mut accepted = 0u64;
        if features & VIRTIO_NET_F_MAC != 0 {
            accepted |= VIRTIO_NET_F_MAC;
        }
        if features & VIRTIO_NET_F_STATUS != 0 {
            accepted |= VIRTIO_NET_F_STATUS;
            self.status_supported = true;
        }
        if self.modern {
            accepted |= VIRTIO_F_VERSION_1;
        }
        self.write_driver_features(accepted);

        if self.modern {
            self.add_status(VIRTIO_STATUS_FEATURES_OK);
            if mmio_read(REG_STATUS) & VIRTIO_STATUS_FEATURES_OK == 0 {
                self.fail();
                return Err(UserDriverError::Unsupported);
            }
        }

        unsafe {
            RX_QUEUE = VirtioNetQueue::new();
            TX_QUEUE = VirtioNetQueue::new();
            RX_BUFFERS = [[0; NET_BUFFER_SIZE]; VIRTIO_QUEUE_SIZE as usize];
            TX_BUFFER = [0; NET_BUFFER_SIZE];

            self.setup_queue(
                0,
                (&raw const RX_QUEUE.desc) as *const _ as u64,
                (&raw const RX_QUEUE.avail) as *const _ as u64,
                (&raw const RX_QUEUE.used) as *const _ as u64,
            )?;
            self.setup_queue(
                1,
                (&raw const TX_QUEUE.desc) as *const _ as u64,
                (&raw const TX_QUEUE.avail) as *const _ as u64,
                (&raw const TX_QUEUE.used) as *const _ as u64,
            )?;
        }

        self.mac = if accepted & VIRTIO_NET_F_MAC != 0 {
            self.read_mac()
        } else {
            [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
        };
        self.link_up = self.read_link_up();
        self.rx_last_used_idx = unsafe { core::ptr::read_volatile(&raw const RX_QUEUE.used.idx) };
        self.tx_last_used_idx = unsafe { core::ptr::read_volatile(&raw const TX_QUEUE.used.idx) };
        self.add_status(VIRTIO_STATUS_DRIVER_OK);
        self.post_receive_buffers();
        self.ready = true;
        Ok(())
    }

    fn setup_queue(
        &self,
        queue: u32,
        desc: u64,
        avail: u64,
        used: u64,
    ) -> Result<(), UserDriverError> {
        mmio_write(REG_QUEUE_SEL, queue);
        let max_queue = mmio_read(REG_QUEUE_NUM_MAX);
        if max_queue == 0 || max_queue < VIRTIO_QUEUE_SIZE as u32 {
            self.fail();
            return Err(UserDriverError::Unsupported);
        }

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
            mmio_write(REG_GUEST_PAGE_SIZE, 4096);
            mmio_write(REG_QUEUE_ALIGN, 4096);
            mmio_write(REG_QUEUE_PFN, (desc / 4096) as u32);
        }
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

    fn read_mac(&self) -> [u8; 6] {
        [
            config_read_u8(CONFIG_MAC),
            config_read_u8(CONFIG_MAC + 1),
            config_read_u8(CONFIG_MAC + 2),
            config_read_u8(CONFIG_MAC + 3),
            config_read_u8(CONFIG_MAC + 4),
            config_read_u8(CONFIG_MAC + 5),
        ]
    }

    fn read_link_up(&self) -> bool {
        if !self.status_supported {
            return true;
        }
        let low = config_read_u8(CONFIG_STATUS) as u16;
        let high = config_read_u8(CONFIG_STATUS + 1) as u16;
        ((high << 8) | low) & VIRTIO_NET_S_LINK_UP != 0
    }

    fn ensure_ready(&self) -> Result<(), UserDriverError> {
        if self.ready {
            Ok(())
        } else {
            Err(UserDriverError::NotReady)
        }
    }

    fn post_receive_buffers(&mut self) {
        for desc_id in 0..VIRTIO_QUEUE_SIZE as usize {
            self.post_receive_buffer(desc_id);
        }
    }

    fn post_receive_buffer(&mut self, desc_id: usize) {
        unsafe {
            RX_QUEUE.desc[desc_id] = VirtqDesc {
                addr: RX_BUFFERS[desc_id].as_mut_ptr() as u64,
                len: NET_BUFFER_SIZE as u32,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            };

            let slot = (RX_QUEUE.avail.idx % VIRTIO_QUEUE_SIZE) as usize;
            RX_QUEUE.avail.ring[slot] = desc_id as u16;
            memory_barrier();
            RX_QUEUE.avail.idx = RX_QUEUE.avail.idx.wrapping_add(1);
            memory_barrier();
            mmio_write(REG_QUEUE_NOTIFY, 0);
        }
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<usize, UserDriverError> {
        if let Err(err) = self.ensure_ready() {
            self.last_error = Some(err);
            return Err(err);
        }
        if frame.len() > ETHERNET_FRAME_MAX || frame.len() + VIRTIO_NET_HDR_LEN > NET_BUFFER_SIZE {
            self.last_error = Some(UserDriverError::OutOfRange);
            return Err(UserDriverError::OutOfRange);
        }

        set_active_mmio_base(self.mmio_base);
        unsafe {
            TX_BUFFER[..VIRTIO_NET_HDR_LEN].fill(0);
            TX_BUFFER[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + frame.len()].copy_from_slice(frame);
            TX_QUEUE.desc[0] = VirtqDesc {
                addr: TX_BUFFER.as_ptr() as u64,
                len: (VIRTIO_NET_HDR_LEN + frame.len()) as u32,
                flags: 0,
                next: 0,
            };

            let slot = (TX_QUEUE.avail.idx % VIRTIO_QUEUE_SIZE) as usize;
            TX_QUEUE.avail.ring[slot] = 0;
            memory_barrier();
            TX_QUEUE.avail.idx = TX_QUEUE.avail.idx.wrapping_add(1);
            memory_barrier();
            mmio_write(REG_QUEUE_NOTIFY, 1);

            let target = self.tx_last_used_idx.wrapping_add(1);
            for _ in 0..NET_TX_TIMEOUT_SPINS {
                memory_barrier();
                if core::ptr::read_volatile(&raw const TX_QUEUE.used.idx) == target {
                    self.tx_last_used_idx = target;
                    mmio_write(REG_INTERRUPT_ACK, mmio_read(REG_INTERRUPT_STATUS));
                    self.tx_packets = self.tx_packets.saturating_add(1);
                    self.tx_bytes = self.tx_bytes.saturating_add(frame.len() as u64);
                    self.last_error = None;
                    return Ok(frame.len());
                }
            }
        }

        self.last_error = Some(UserDriverError::Timeout);
        Err(UserDriverError::Timeout)
    }

    fn receive_frame(
        &mut self,
        out: &mut [u8],
        timeout_spins: usize,
    ) -> Result<usize, UserDriverError> {
        if let Err(err) = self.ensure_ready() {
            self.last_error = Some(err);
            return Err(err);
        }
        set_active_mmio_base(self.mmio_base);

        for _ in 0..timeout_spins {
            unsafe {
                memory_barrier();
                let used_idx = core::ptr::read_volatile(&raw const RX_QUEUE.used.idx);
                if used_idx == self.rx_last_used_idx {
                    continue;
                }

                let used_slot = (self.rx_last_used_idx % VIRTIO_QUEUE_SIZE) as usize;
                let used = core::ptr::read_volatile(&raw const RX_QUEUE.used.ring[used_slot]);
                self.rx_last_used_idx = self.rx_last_used_idx.wrapping_add(1);
                mmio_write(REG_INTERRUPT_ACK, mmio_read(REG_INTERRUPT_STATUS));

                let desc_id = used.id as usize;
                if desc_id >= VIRTIO_QUEUE_SIZE as usize {
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::Io);
                    return Err(UserDriverError::Io);
                }

                let packet_len = used.len as usize;
                if packet_len < VIRTIO_NET_HDR_LEN {
                    self.post_receive_buffer(desc_id);
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::Io);
                    return Err(UserDriverError::Io);
                }

                let frame_len = packet_len - VIRTIO_NET_HDR_LEN;
                if frame_len > out.len() {
                    self.post_receive_buffer(desc_id);
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::OutOfRange);
                    return Err(UserDriverError::OutOfRange);
                }

                out[..frame_len].copy_from_slice(
                    &RX_BUFFERS[desc_id]
                        [VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN.saturating_add(frame_len)],
                );
                self.post_receive_buffer(desc_id);
                self.rx_packets = self.rx_packets.saturating_add(1);
                self.rx_bytes = self.rx_bytes.saturating_add(frame_len as u64);
                self.last_error = None;
                return Ok(frame_len);
            }
        }

        self.last_error = Some(UserDriverError::Timeout);
        Err(UserDriverError::Timeout)
    }
}

static mut DRIVER: QemuVirtNetDriver = QemuVirtNetDriver::new();
static mut RX_QUEUE: VirtioNetQueue = VirtioNetQueue::new();
static mut TX_QUEUE: VirtioNetQueue = VirtioNetQueue::new();
static mut RX_BUFFERS: [[u8; NET_BUFFER_SIZE]; VIRTIO_QUEUE_SIZE as usize] =
    [[0; NET_BUFFER_SIZE]; VIRTIO_QUEUE_SIZE as usize];
static mut TX_BUFFER: [u8; NET_BUFFER_SIZE] = [0; NET_BUFFER_SIZE];
static mut ACTIVE_VIRTIO_MMIO_BASE: usize = MMIO_BASE;

fn driver() -> &'static mut QemuVirtNetDriver {
    unsafe { &mut DRIVER }
}

pub fn bind() -> Result<(), UserDriverError> {
    driver().bind()
}

pub fn ready() -> bool {
    driver().ready
}

pub fn mmio_base() -> usize {
    driver().mmio_base
}

pub fn device_status() -> u32 {
    let driver = driver();
    if driver.ready {
        set_active_mmio_base(driver.mmio_base);
        mmio_read(REG_STATUS)
    } else {
        0
    }
}

pub fn last_error() -> Option<UserDriverError> {
    driver().last_error
}

pub fn mac() -> [u8; 6] {
    driver().mac
}

pub fn link_up() -> bool {
    let driver = driver();
    if driver.ready {
        set_active_mmio_base(driver.mmio_base);
        driver.link_up = driver.read_link_up();
    }
    driver.link_up
}

pub fn rx_packets() -> u64 {
    driver().rx_packets
}

pub fn tx_packets() -> u64 {
    driver().tx_packets
}

pub fn rx_bytes() -> u64 {
    driver().rx_bytes
}

pub fn tx_bytes() -> u64 {
    driver().tx_bytes
}

pub fn dropped_packets() -> u64 {
    driver().dropped_packets
}

pub fn send_frame(frame: &[u8]) -> Result<usize, UserDriverError> {
    driver().send_frame(frame)
}

pub fn receive_frame(out: &mut [u8]) -> Result<usize, UserDriverError> {
    driver().receive_frame(out, NET_POLL_TIMEOUT_SPINS)
}

pub fn receive_frame_timeout(
    out: &mut [u8],
    timeout_spins: usize,
) -> Result<usize, UserDriverError> {
    driver().receive_frame(out, timeout_spins)
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

fn config_read_u8(offset: usize) -> u8 {
    unsafe { core::ptr::read_volatile((active_mmio_base() + REG_CONFIG + offset) as *const u8) }
}

fn memory_barrier() {
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
}
