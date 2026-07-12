//! VirtIO network driver for QEMU VirtIO-MMIO and VirtIO-PCI transports.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::{driver_logic, pci, UserDriverError};

pub const MMIO_BASE: usize = 0x0a00_0000;
pub const MMIO_STRIDE: usize = 0x200;
pub const MMIO_SLOT_COUNT: usize = 32;
pub const ETHERNET_MTU: usize = 1500;
pub const ETHERNET_HEADER_LEN: usize = 14;
pub const ETHERNET_FRAME_MAX: usize = ETHERNET_HEADER_LEN + ETHERNET_MTU;
const ETHERNET_FRAME_MIN: usize = 60;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_NET_F_MAC: u64 = 1 << 5;
const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
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
const VIRTIO_NET_HDR_MRG_LEN: usize = 12;
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
const PCI_COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const PCI_COMMON_DEVICE_FEATURE: usize = 0x04;
const PCI_COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const PCI_COMMON_DRIVER_FEATURE: usize = 0x0c;
const PCI_COMMON_QUEUE_SELECT: usize = 0x16;
const PCI_COMMON_QUEUE_SIZE: usize = 0x18;
const PCI_COMMON_QUEUE_MSIX_VECTOR: usize = 0x1a;
const PCI_COMMON_QUEUE_ENABLE: usize = 0x1c;
const PCI_COMMON_QUEUE_NOTIFY_OFF: usize = 0x1e;
const PCI_COMMON_QUEUE_DESC: usize = 0x20;
const PCI_COMMON_QUEUE_DRIVER: usize = 0x28;
const PCI_COMMON_QUEUE_DEVICE: usize = 0x30;
const PCI_COMMON_DEVICE_STATUS: usize = 0x14;
const VIRTIO_MSI_NO_VECTOR: u16 = 0xffff;

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
enum VirtioNetTransport {
    Mmio,
    PciModern {
        common_base: usize,
        notify_base: usize,
        notify_multiplier: u32,
        isr_base: usize,
        device_config_base: usize,
    },
}

#[derive(Clone, Copy)]
struct QemuVirtNetDriver {
    ready: bool,
    modern: bool,
    mmio_base: usize,
    transport: VirtioNetTransport,
    mac: [u8; 6],
    status_supported: bool,
    link_up: bool,
    net_header_len: usize,
    notify_offsets: [u16; 2],
    rx_posted: bool,
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
            transport: VirtioNetTransport::Mmio,
            mac: [0; 6],
            status_supported: false,
            link_up: false,
            net_header_len: VIRTIO_NET_HDR_LEN,
            notify_offsets: [0; 2],
            rx_posted: false,
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
            let Some(base) = driver_logic::mmio_slot_base(MMIO_BASE, slot, MMIO_STRIDE) else {
                continue;
            };
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
        self.transport = VirtioNetTransport::Mmio;
        set_active_mmio_base(base);

        if !driver_logic::virtio_identity_valid(
            mmio_read(REG_MAGIC_VALUE),
            mmio_read(REG_DEVICE_ID),
            VIRTIO_DEVICE_ID_NET,
            mmio_read(REG_VENDOR_ID),
            VIRTIO_MMIO_VENDOR_QEMU,
        ) {
            return Err(UserDriverError::NotFound);
        }

        let version = mmio_read(REG_VERSION);
        if !driver_logic::virtio_version_supported(
            version,
            VIRTIO_MMIO_VERSION_LEGACY,
            VIRTIO_MMIO_VERSION_MODERN,
        ) {
            return Err(UserDriverError::Unsupported);
        }
        self.modern = driver_logic::virtio_version_is_modern(version, VIRTIO_MMIO_VERSION_MODERN);

        self.finish_bind()
    }

    fn bind_pci(&mut self) -> Result<(), UserDriverError> {
        if self.ready {
            return Ok(());
        }
        let Some(transport) = pci::find_virtio_net_transport() else {
            self.last_error = Some(UserDriverError::NotFound);
            return Err(UserDriverError::NotFound);
        };
        self.mmio_base = transport.common_base;
        self.transport = VirtioNetTransport::PciModern {
            common_base: transport.common_base,
            notify_base: transport.notify_base,
            notify_multiplier: transport.notify_multiplier,
            isr_base: transport.isr_base,
            device_config_base: transport.device_config_base,
        };
        self.modern = true;

        self.finish_bind()
    }

    fn finish_bind(&mut self) -> Result<(), UserDriverError> {
        self.write_status(0);
        self.write_status(VIRTIO_STATUS_ACKNOWLEDGE);
        self.write_status(VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        let features = self.read_device_features();
        let mut accepted = driver_logic::virtio_net_accepted_features(
            features,
            VIRTIO_NET_F_MAC,
            VIRTIO_NET_F_STATUS,
            VIRTIO_F_VERSION_1,
            self.modern,
        );
        if features & VIRTIO_NET_F_MRG_RXBUF != 0 {
            accepted |= VIRTIO_NET_F_MRG_RXBUF;
            self.net_header_len = VIRTIO_NET_HDR_MRG_LEN;
        } else {
            self.net_header_len = VIRTIO_NET_HDR_LEN;
        }
        self.status_supported = driver_logic::virtio_feature_present(accepted, VIRTIO_NET_F_STATUS);
        self.write_driver_features(accepted);

        if self.modern {
            self.add_status(VIRTIO_STATUS_FEATURES_OK);
            if self.read_status() & VIRTIO_STATUS_FEATURES_OK == 0 {
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
        self.rx_posted = false;
        self.ready = true;
        Ok(())
    }

    fn setup_queue(
        &mut self,
        queue: u32,
        desc: u64,
        avail: u64,
        used: u64,
    ) -> Result<(), UserDriverError> {
        self.select_queue(queue);
        let max_queue = self.queue_size_max();
        if !driver_logic::virtio_queue_size_valid(max_queue, VIRTIO_QUEUE_SIZE) {
            self.fail();
            return Err(UserDriverError::Unsupported);
        }

        self.write_queue_size(VIRTIO_QUEUE_SIZE);
        if self.modern {
            self.write_queue_modern(queue, desc, avail, used);
        } else {
            mmio_write(REG_GUEST_PAGE_SIZE, 4096);
            mmio_write(REG_QUEUE_ALIGN, 4096);
            mmio_write(REG_QUEUE_PFN, (desc / 4096) as u32);
        }
        Ok(())
    }

    fn read_device_features(&self) -> u64 {
        match self.transport {
            VirtioNetTransport::Mmio => {
                mmio_write(REG_DEVICE_FEATURES_SEL, 0);
                let low = mmio_read(REG_DEVICE_FEATURES) as u64;
                mmio_write(REG_DEVICE_FEATURES_SEL, 1);
                let high = mmio_read(REG_DEVICE_FEATURES) as u64;
                low | (high << 32)
            }
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u32(common_base + PCI_COMMON_DEVICE_FEATURE_SELECT, 0);
                let low = pci_read_u32(common_base + PCI_COMMON_DEVICE_FEATURE) as u64;
                pci_write_u32(common_base + PCI_COMMON_DEVICE_FEATURE_SELECT, 1);
                let high = pci_read_u32(common_base + PCI_COMMON_DEVICE_FEATURE) as u64;
                low | (high << 32)
            }
        }
    }

    fn write_driver_features(&self, features: u64) {
        match self.transport {
            VirtioNetTransport::Mmio => {
                mmio_write(REG_DRIVER_FEATURES_SEL, 0);
                mmio_write(REG_DRIVER_FEATURES, features as u32);
                mmio_write(REG_DRIVER_FEATURES_SEL, 1);
                mmio_write(REG_DRIVER_FEATURES, (features >> 32) as u32);
            }
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u32(common_base + PCI_COMMON_DRIVER_FEATURE_SELECT, 0);
                pci_write_u32(common_base + PCI_COMMON_DRIVER_FEATURE, features as u32);
                pci_write_u32(common_base + PCI_COMMON_DRIVER_FEATURE_SELECT, 1);
                pci_write_u32(
                    common_base + PCI_COMMON_DRIVER_FEATURE,
                    (features >> 32) as u32,
                );
            }
        }
    }

    fn add_status(&self, status: u32) {
        let current = self.read_status();
        self.write_status(current | status);
    }

    fn fail(&self) {
        let current = self.read_status();
        self.write_status(current | VIRTIO_STATUS_FAILED);
    }

    fn read_mac(&self) -> [u8; 6] {
        [
            self.config_read_u8(CONFIG_MAC),
            self.config_read_u8(CONFIG_MAC + 1),
            self.config_read_u8(CONFIG_MAC + 2),
            self.config_read_u8(CONFIG_MAC + 3),
            self.config_read_u8(CONFIG_MAC + 4),
            self.config_read_u8(CONFIG_MAC + 5),
        ]
    }

    fn read_link_up(&self) -> bool {
        if !self.status_supported {
            return true;
        }
        let low = self.config_read_u8(CONFIG_STATUS) as u16;
        let high = self.config_read_u8(CONFIG_STATUS + 1) as u16;
        ((high << 8) | low) & VIRTIO_NET_S_LINK_UP != 0
    }

    fn read_status(&self) -> u32 {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_read(REG_STATUS),
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_read_u8(common_base + PCI_COMMON_DEVICE_STATUS) as u32
            }
        }
    }

    fn write_status(&self, status: u32) {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_write(REG_STATUS, status),
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u8(common_base + PCI_COMMON_DEVICE_STATUS, status as u8)
            }
        }
    }

    fn select_queue(&self, queue: u32) {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_write(REG_QUEUE_SEL, queue),
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u16(common_base + PCI_COMMON_QUEUE_SELECT, queue as u16)
            }
        }
    }

    fn queue_size_max(&self) -> u32 {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_read(REG_QUEUE_NUM_MAX),
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_read_u16(common_base + PCI_COMMON_QUEUE_SIZE) as u32
            }
        }
    }

    fn write_queue_size(&self, size: u16) {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_write(REG_QUEUE_NUM, size as u32),
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u16(common_base + PCI_COMMON_QUEUE_SIZE, size)
            }
        }
    }

    fn write_queue_modern(&mut self, queue: u32, desc: u64, avail: u64, used: u64) {
        match self.transport {
            VirtioNetTransport::Mmio => {
                mmio_write(REG_QUEUE_DESC_LOW, desc as u32);
                mmio_write(REG_QUEUE_DESC_HIGH, (desc >> 32) as u32);
                mmio_write(REG_QUEUE_DRIVER_LOW, avail as u32);
                mmio_write(REG_QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
                mmio_write(REG_QUEUE_DEVICE_LOW, used as u32);
                mmio_write(REG_QUEUE_DEVICE_HIGH, (used >> 32) as u32);
                mmio_write(REG_QUEUE_READY, 1);
            }
            VirtioNetTransport::PciModern { common_base, .. } => {
                pci_write_u16(
                    common_base + PCI_COMMON_QUEUE_MSIX_VECTOR,
                    VIRTIO_MSI_NO_VECTOR,
                );
                if (queue as usize) < self.notify_offsets.len() {
                    self.notify_offsets[queue as usize] =
                        pci_read_u16(common_base + PCI_COMMON_QUEUE_NOTIFY_OFF);
                }
                pci_write_u64(common_base + PCI_COMMON_QUEUE_DESC, desc);
                pci_write_u64(common_base + PCI_COMMON_QUEUE_DRIVER, avail);
                pci_write_u64(common_base + PCI_COMMON_QUEUE_DEVICE, used);
                pci_write_u16(common_base + PCI_COMMON_QUEUE_ENABLE, 1);
            }
        }
    }

    fn notify_queue(&self, queue: u32) {
        match self.transport {
            VirtioNetTransport::Mmio => mmio_write(REG_QUEUE_NOTIFY, queue),
            VirtioNetTransport::PciModern {
                notify_base,
                notify_multiplier,
                ..
            } => {
                let offset = self
                    .notify_offsets
                    .get(queue as usize)
                    .copied()
                    .unwrap_or(queue as u16) as usize;
                let notify_addr =
                    notify_base.saturating_add(offset.saturating_mul(notify_multiplier as usize));
                pci_write_u16(notify_addr, queue as u16);
            }
        }
    }

    fn ack_interrupt(&self) {
        match self.transport {
            VirtioNetTransport::Mmio => {
                mmio_write(REG_INTERRUPT_ACK, mmio_read(REG_INTERRUPT_STATUS))
            }
            VirtioNetTransport::PciModern { isr_base, .. } => {
                let _ = pci_read_u8(isr_base);
            }
        }
    }

    fn config_read_u8(&self, offset: usize) -> u8 {
        match self.transport {
            VirtioNetTransport::Mmio => config_read_u8(offset),
            VirtioNetTransport::PciModern {
                device_config_base, ..
            } => pci_read_u8(device_config_base + offset),
        }
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
        self.rx_posted = true;
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
            self.notify_queue(0);
        }
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<usize, UserDriverError> {
        if let Err(err) = self.ensure_ready() {
            self.last_error = Some(err);
            return Err(err);
        }
        if !self.rx_posted {
            self.post_receive_buffers();
        }
        if !driver_logic::net_tx_frame_len_valid(
            frame.len(),
            ETHERNET_FRAME_MAX,
            self.net_header_len,
            NET_BUFFER_SIZE,
        ) {
            self.last_error = Some(UserDriverError::OutOfRange);
            return Err(UserDriverError::OutOfRange);
        }

        unsafe {
            let header_len = self.net_header_len;
            TX_BUFFER[..header_len].fill(0);
            TX_BUFFER[header_len..header_len + frame.len()].copy_from_slice(frame);
            let tx_frame_len = core::cmp::max(frame.len(), ETHERNET_FRAME_MIN);
            if tx_frame_len > frame.len() {
                TX_BUFFER[header_len + frame.len()..header_len + tx_frame_len].fill(0);
            }
            TX_QUEUE.desc[0] = VirtqDesc {
                addr: TX_BUFFER.as_ptr() as u64,
                len: (header_len + tx_frame_len) as u32,
                flags: 0,
                next: 0,
            };

            let slot = (TX_QUEUE.avail.idx % VIRTIO_QUEUE_SIZE) as usize;
            TX_QUEUE.avail.ring[slot] = 0;
            memory_barrier();
            TX_QUEUE.avail.idx = TX_QUEUE.avail.idx.wrapping_add(1);
            memory_barrier();
            self.notify_queue(1);

            let target = self.tx_last_used_idx.wrapping_add(1);
            for _ in 0..NET_TX_TIMEOUT_SPINS {
                memory_barrier();
                if core::ptr::read_volatile(&raw const TX_QUEUE.used.idx) == target {
                    self.tx_last_used_idx = target;
                    self.ack_interrupt();
                    self.tx_packets = self.tx_packets.saturating_add(1);
                    self.tx_bytes = self.tx_bytes.saturating_add(tx_frame_len as u64);
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
        if !self.rx_posted {
            self.post_receive_buffers();
        }
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
                self.ack_interrupt();

                let desc_id = used.id as usize;
                if desc_id >= VIRTIO_QUEUE_SIZE as usize {
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::Io);
                    return Err(UserDriverError::Io);
                }

                let packet_len = used.len as usize;
                if !driver_logic::net_rx_packet_len_valid(
                    packet_len,
                    self.net_header_len,
                    NET_BUFFER_SIZE,
                ) {
                    self.post_receive_buffer(desc_id);
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::Io);
                    return Err(UserDriverError::Io);
                }

                let Some(frame_len) =
                    driver_logic::net_rx_frame_len(packet_len, self.net_header_len)
                else {
                    self.post_receive_buffer(desc_id);
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::Io);
                    return Err(UserDriverError::Io);
                };
                if !driver_logic::net_rx_output_len_valid(frame_len, out.len()) {
                    self.post_receive_buffer(desc_id);
                    self.dropped_packets = self.dropped_packets.saturating_add(1);
                    self.last_error = Some(UserDriverError::OutOfRange);
                    return Err(UserDriverError::OutOfRange);
                }

                out[..frame_len].copy_from_slice(
                    &RX_BUFFERS[desc_id]
                        [self.net_header_len..self.net_header_len.saturating_add(frame_len)],
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

pub fn bind_at(base: usize) -> Result<(), UserDriverError> {
    match driver().bind_at(base) {
        Ok(()) => {
            driver().last_error = None;
            Ok(())
        }
        Err(err) => {
            driver().last_error = Some(err);
            Err(err)
        }
    }
}

pub fn bind_pci() -> Result<(), UserDriverError> {
    match driver().bind_pci() {
        Ok(()) => {
            driver().last_error = None;
            Ok(())
        }
        Err(err) => {
            driver().last_error = Some(err);
            Err(err)
        }
    }
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
        driver.read_status()
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

fn pci_read_u8(addr: usize) -> u8 {
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

fn pci_read_u16(addr: usize) -> u16 {
    unsafe { core::ptr::read_volatile(addr as *const u16) }
}

fn pci_read_u32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn pci_write_u8(addr: usize, value: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

fn pci_write_u16(addr: usize, value: u16) {
    unsafe { core::ptr::write_volatile(addr as *mut u16, value) }
}

fn pci_write_u32(addr: usize, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

fn pci_write_u64(addr: usize, value: u64) {
    unsafe { core::ptr::write_volatile(addr as *mut u64, value) }
}

fn memory_barrier() {
    crate::kernel_lowlevel::cpu::mmio_barrier();
}
