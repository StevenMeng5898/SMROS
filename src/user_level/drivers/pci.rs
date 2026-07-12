//! Minimal PCI config-space discovery for x86_64 direct-boot QEMU.
//!
//! The x86_64 PVH boot path does not run a PCI firmware/BIOS assignment pass,
//! so VirtIO-PCI BARs may still be unassigned when SMROS starts. This module
//! enumerates legacy PCI config space, sizes BARs, assigns them from fallback
//! x86 PCI IO/MMIO windows, and returns the modern VirtIO-PCI transport regions
//! used by the block and network drivers.

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtioPciTransport {
    pub common_base: usize,
    pub notify_base: usize,
    pub notify_multiplier: u32,
    pub isr_base: usize,
    pub device_config_base: usize,
}

#[cfg(target_arch = "x86_64")]
mod arch {
    use super::VirtioPciTransport;
    use core::sync::atomic::{AtomicU32, Ordering};

    const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
    const PCI_CONFIG_DATA: u16 = 0x0cfc;
    const PCI_VENDOR_INVALID: u16 = 0xffff;
    const PCI_BUSES: u16 = 256;
    const PCI_DEVICES: u8 = 32;
    const PCI_FUNCTIONS: u8 = 8;
    const PCI_HEADER_TYPE_MULTI_FUNCTION: u8 = 0x80;
    const PCI_STATUS_CAPABILITIES: u16 = 1 << 4;
    const PCI_COMMAND_IO: u16 = 1 << 0;
    const PCI_COMMAND_MEMORY: u16 = 1 << 1;
    const PCI_COMMAND_BUS_MASTER: u16 = 1 << 2;
    const PCI_BAR_COUNT: u8 = 6;
    const PCI_BAR0: u16 = 0x10;
    const PCI_COMMAND: u16 = 0x04;
    const PCI_STATUS: u16 = 0x06;
    const PCI_HEADER_TYPE: u16 = 0x0e;
    const PCI_CAP_PTR: u16 = 0x34;
    const PCI_SUBSYSTEM_ID: u16 = 0x2e;
    const PCI_CAP_ID_VENDOR_SPECIFIC: u8 = 0x09;
    const VIRTIO_VENDOR_ID: u16 = 0x1af4;
    const VIRTIO_NET_TRANSITIONAL_DEVICE_ID: u16 = 0x1000;
    const VIRTIO_NET_MODERN_DEVICE_ID: u16 = 0x1041;
    const VIRTIO_NET_SUBSYSTEM_ID: u16 = 1;
    const VIRTIO_BLOCK_TRANSITIONAL_DEVICE_ID: u16 = 0x1001;
    const VIRTIO_BLOCK_MODERN_DEVICE_ID: u16 = 0x1042;
    const VIRTIO_BLOCK_SUBSYSTEM_ID: u16 = 2;
    const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
    const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
    const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
    const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
    const PCI_MMIO32_ALLOC_BASE: u32 = 0xc000_0000;
    const PCI_MMIO32_ALLOC_LIMIT: u32 = 0xff00_0000;
    const PCI_IO_ALLOC_BASE: u32 = 0x1000;
    const PCI_IO_ALLOC_LIMIT: u32 = 0xf000;

    static NEXT_MMIO32: AtomicU32 = AtomicU32::new(PCI_MMIO32_ALLOC_BASE);
    static NEXT_IO: AtomicU32 = AtomicU32::new(PCI_IO_ALLOC_BASE);

    #[derive(Clone, Copy)]
    struct VirtioCap {
        cfg_type: u8,
        bar: u8,
        offset: u32,
        notify_multiplier: u32,
    }

    #[derive(Clone, Copy)]
    enum VirtioDeviceKind {
        Block,
        Net,
    }

    pub fn find_virtio_net_transport() -> Option<VirtioPciTransport> {
        find_virtio_transport(VirtioDeviceKind::Net)
    }

    pub fn find_virtio_block_transport() -> Option<VirtioPciTransport> {
        find_virtio_transport(VirtioDeviceKind::Block)
    }

    fn find_virtio_transport(kind: VirtioDeviceKind) -> Option<VirtioPciTransport> {
        for bus in 0..PCI_BUSES {
            for device in 0..PCI_DEVICES {
                let bus = bus as u8;
                if config_read_u16(bus, device, 0, 0) == PCI_VENDOR_INVALID {
                    continue;
                }
                let header = config_read_u8(bus, device, 0, PCI_HEADER_TYPE);
                let functions = if header & PCI_HEADER_TYPE_MULTI_FUNCTION != 0 {
                    PCI_FUNCTIONS
                } else {
                    1
                };

                for function in 0..functions {
                    if config_read_u16(bus, device, function, 0) == PCI_VENDOR_INVALID {
                        continue;
                    }
                    if !is_virtio_kind(bus, device, function, kind) {
                        continue;
                    }
                    assign_bars(bus, device, function);
                    enable_device(bus, device, function);
                    if let Some(transport) = virtio_transport(bus, device, function) {
                        return Some(transport);
                    }
                }
            }
        }
        None
    }

    fn is_virtio_kind(bus: u8, device: u8, function: u8, kind: VirtioDeviceKind) -> bool {
        let vendor = config_read_u16(bus, device, function, 0);
        if vendor != VIRTIO_VENDOR_ID {
            return false;
        }
        let device_id = config_read_u16(bus, device, function, 2);
        let subsystem_id = config_read_u16(bus, device, function, PCI_SUBSYSTEM_ID);
        match kind {
            VirtioDeviceKind::Block => {
                device_id == VIRTIO_BLOCK_MODERN_DEVICE_ID
                    || (device_id == VIRTIO_BLOCK_TRANSITIONAL_DEVICE_ID
                        && subsystem_id == VIRTIO_BLOCK_SUBSYSTEM_ID)
            }
            VirtioDeviceKind::Net => {
                device_id == VIRTIO_NET_MODERN_DEVICE_ID
                    || (device_id == VIRTIO_NET_TRANSITIONAL_DEVICE_ID
                        && subsystem_id == VIRTIO_NET_SUBSYSTEM_ID)
            }
        }
    }

    fn virtio_transport(bus: u8, device: u8, function: u8) -> Option<VirtioPciTransport> {
        if config_read_u16(bus, device, function, PCI_STATUS) & PCI_STATUS_CAPABILITIES == 0 {
            return None;
        }

        let mut common = None;
        let mut notify = None;
        let mut isr = None;
        let mut device_config = None;
        let mut cap_ptr = config_read_u8(bus, device, function, PCI_CAP_PTR) & !0x3;
        let mut visited = 0;

        while cap_ptr >= 0x40 && visited < 48 {
            let cap_id = config_read_u8(bus, device, function, cap_ptr as u16);
            let next = config_read_u8(bus, device, function, cap_ptr as u16 + 1) & !0x3;
            if cap_id == PCI_CAP_ID_VENDOR_SPECIFIC {
                if let Some(cap) = read_virtio_cap(bus, device, function, cap_ptr as u16) {
                    match cap.cfg_type {
                        VIRTIO_PCI_CAP_COMMON_CFG => {
                            common = region_base(bus, device, function, cap)
                        }
                        VIRTIO_PCI_CAP_NOTIFY_CFG => {
                            notify = region_base(bus, device, function, cap)
                        }
                        VIRTIO_PCI_CAP_ISR_CFG => isr = region_base(bus, device, function, cap),
                        VIRTIO_PCI_CAP_DEVICE_CFG => {
                            device_config = region_base(bus, device, function, cap)
                        }
                        _ => {}
                    }
                }
            }
            if next == 0 || next == cap_ptr {
                break;
            }
            cap_ptr = next;
            visited += 1;
        }

        Some(VirtioPciTransport {
            common_base: common?,
            notify_base: notify?,
            notify_multiplier: notify_multiplier(bus, device, function)?,
            isr_base: isr?,
            device_config_base: device_config?,
        })
    }

    fn read_virtio_cap(bus: u8, device: u8, function: u8, ptr: u16) -> Option<VirtioCap> {
        let cap_len = config_read_u8(bus, device, function, ptr + 2);
        if cap_len < 16 {
            return None;
        }
        Some(VirtioCap {
            cfg_type: config_read_u8(bus, device, function, ptr + 3),
            bar: config_read_u8(bus, device, function, ptr + 4),
            offset: config_read_u32(bus, device, function, ptr + 8),
            notify_multiplier: if cap_len >= 20 {
                config_read_u32(bus, device, function, ptr + 16)
            } else {
                0
            },
        })
    }

    fn notify_multiplier(bus: u8, device: u8, function: u8) -> Option<u32> {
        let mut cap_ptr = config_read_u8(bus, device, function, PCI_CAP_PTR) & !0x3;
        let mut visited = 0;
        while cap_ptr >= 0x40 && visited < 48 {
            if config_read_u8(bus, device, function, cap_ptr as u16) == PCI_CAP_ID_VENDOR_SPECIFIC {
                let cap = read_virtio_cap(bus, device, function, cap_ptr as u16)?;
                if cap.cfg_type == VIRTIO_PCI_CAP_NOTIFY_CFG {
                    return Some(cap.notify_multiplier);
                }
            }
            let next = config_read_u8(bus, device, function, cap_ptr as u16 + 1) & !0x3;
            if next == 0 || next == cap_ptr {
                break;
            }
            cap_ptr = next;
            visited += 1;
        }
        None
    }

    fn region_base(bus: u8, device: u8, function: u8, cap: VirtioCap) -> Option<usize> {
        memory_cap_bar_base(bus, device, function, cap.bar)
            .and_then(|base| base.checked_add(cap.offset as usize))
    }

    fn assign_bars(bus: u8, device: u8, function: u8) {
        let mut bar = 0u8;
        while bar < PCI_BAR_COUNT {
            let offset = PCI_BAR0 + (bar as u16 * 4);
            let original = config_read_u32(bus, device, function, offset);
            config_write_u32(bus, device, function, offset, 0xffff_ffff);
            let mask = config_read_u32(bus, device, function, offset);
            config_write_u32(bus, device, function, offset, original);

            if mask == 0 || mask == 0xffff_ffff {
                bar += 1;
                continue;
            }

            if mask & 1 != 0 {
                if io_bar_base(original) == 0 {
                    let size = (!(mask & !0x3)).wrapping_add(1);
                    if let Some(base) = allocate_io(size) {
                        config_write_u32(bus, device, function, offset, base | (mask & 0x3));
                    }
                }
                bar += 1;
                continue;
            }

            let flags = mask & 0xf;
            let is_64 = mask & 0x6 == 0x4;
            if is_64 && bar + 1 < PCI_BAR_COUNT {
                let high_offset = offset + 4;
                let original_high = config_read_u32(bus, device, function, high_offset);
                config_write_u32(bus, device, function, offset, 0xffff_ffff);
                config_write_u32(bus, device, function, high_offset, 0xffff_ffff);
                let low_mask = config_read_u32(bus, device, function, offset);
                let high_mask = config_read_u32(bus, device, function, high_offset);
                config_write_u32(bus, device, function, offset, original);
                config_write_u32(bus, device, function, high_offset, original_high);

                if memory_bar_base(original, Some(original_high)) == 0 {
                    let mask64 = ((high_mask as u64) << 32) | ((low_mask & !0xf) as u64);
                    let size = (!mask64).wrapping_add(1);
                    if let Some(base) = allocate_mmio32(size) {
                        config_write_u32(bus, device, function, offset, (base as u32) | flags);
                        config_write_u32(bus, device, function, high_offset, 0);
                    }
                }
                bar += 2;
            } else {
                if memory_bar_base(original, None) == 0 {
                    let size = (!(mask & !0xf)).wrapping_add(1) as u64;
                    if let Some(base) = allocate_mmio32(size) {
                        config_write_u32(bus, device, function, offset, (base as u32) | flags);
                    }
                }
                bar += 1;
            }
        }
    }

    fn enable_device(bus: u8, device: u8, function: u8) {
        let command = config_read_u16(bus, device, function, PCI_COMMAND);
        config_write_u16(
            bus,
            device,
            function,
            PCI_COMMAND,
            command | PCI_COMMAND_IO | PCI_COMMAND_MEMORY | PCI_COMMAND_BUS_MASTER,
        );
    }

    fn memory_cap_bar_base(bus: u8, device: u8, function: u8, bar: u8) -> Option<usize> {
        if bar >= PCI_BAR_COUNT {
            return None;
        }
        let offset = PCI_BAR0 + (bar as u16 * 4);
        let raw = config_read_u32(bus, device, function, offset);
        if raw & 1 != 0 {
            return None;
        }
        let base = if raw & 0x6 == 0x4 && bar + 1 < PCI_BAR_COUNT {
            let high = config_read_u32(bus, device, function, offset + 4);
            memory_bar_base(raw, Some(high))
        } else {
            memory_bar_base(raw, None)
        };
        if base == 0 {
            None
        } else {
            Some(base as usize)
        }
    }

    fn io_bar_base(raw: u32) -> u32 {
        raw & !0x3
    }

    fn memory_bar_base(low: u32, high: Option<u32>) -> u64 {
        ((high.unwrap_or(0) as u64) << 32) | ((low & !0xf) as u64)
    }

    fn allocate_io(size: u32) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let align = core::cmp::max(size.next_power_of_two(), 4);
        loop {
            let current = NEXT_IO.load(Ordering::Acquire);
            let aligned = align_up_u32(current, align)?;
            let next = aligned.checked_add(size)?;
            if next > PCI_IO_ALLOC_LIMIT {
                return None;
            }
            if NEXT_IO
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(aligned);
            }
        }
    }

    fn allocate_mmio32(size: u64) -> Option<u64> {
        if size == 0 || size > u32::MAX as u64 {
            return None;
        }
        let align = core::cmp::max(size.next_power_of_two(), 0x1000);
        loop {
            let current = NEXT_MMIO32.load(Ordering::Acquire) as u64;
            let aligned = align_up_u64(current, align)?;
            let next = aligned.checked_add(size)?;
            if next > PCI_MMIO32_ALLOC_LIMIT as u64 {
                return None;
            }
            if NEXT_MMIO32
                .compare_exchange(
                    current as u32,
                    next as u32,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Some(aligned);
            }
        }
    }

    fn align_up_u32(value: u32, align: u32) -> Option<u32> {
        if align == 0 || !align.is_power_of_two() {
            return None;
        }
        value.checked_add(align - 1).map(|v| v & !(align - 1))
    }

    fn align_up_u64(value: u64, align: u64) -> Option<u64> {
        if align == 0 || !align.is_power_of_two() {
            return None;
        }
        value.checked_add(align - 1).map(|v| v & !(align - 1))
    }

    fn config_read_u8(bus: u8, device: u8, function: u8, offset: u16) -> u8 {
        let value = config_read_u32(bus, device, function, offset & !0x3);
        let shift = ((offset & 0x3) * 8) as u32;
        ((value >> shift) & 0xff) as u8
    }

    fn config_read_u16(bus: u8, device: u8, function: u8, offset: u16) -> u16 {
        let value = config_read_u32(bus, device, function, offset & !0x3);
        let shift = ((offset & 0x2) * 8) as u32;
        ((value >> shift) & 0xffff) as u16
    }

    fn config_read_u32(bus: u8, device: u8, function: u8, offset: u16) -> u32 {
        unsafe {
            outl(
                PCI_CONFIG_ADDRESS,
                config_address(bus, device, function, offset),
            );
            inl(PCI_CONFIG_DATA)
        }
    }

    fn config_write_u16(bus: u8, device: u8, function: u8, offset: u16, value: u16) {
        let aligned = offset & !0x3;
        let shift = ((offset & 0x2) * 8) as u32;
        let old = config_read_u32(bus, device, function, aligned);
        let new = (old & !(0xffff << shift)) | ((value as u32) << shift);
        config_write_u32(bus, device, function, aligned, new);
    }

    fn config_write_u32(bus: u8, device: u8, function: u8, offset: u16, value: u32) {
        unsafe {
            outl(
                PCI_CONFIG_ADDRESS,
                config_address(bus, device, function, offset),
            );
            outl(PCI_CONFIG_DATA, value);
        }
    }

    fn config_address(bus: u8, device: u8, function: u8, offset: u16) -> u32 {
        0x8000_0000
            | ((bus as u32) << 16)
            | ((device as u32) << 11)
            | ((function as u32) << 8)
            | ((offset as u32) & 0xfc)
    }

    unsafe fn inl(port: u16) -> u32 {
        let value: u32;
        core::arch::asm!(
            "in eax, dx",
            in("dx") port,
            out("eax") value,
            options(nomem, nostack, preserves_flags),
        );
        value
    }

    unsafe fn outl(port: u16, value: u32) {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod arch {
    use super::VirtioPciTransport;

    pub fn find_virtio_net_transport() -> Option<VirtioPciTransport> {
        None
    }

    pub fn find_virtio_block_transport() -> Option<VirtioPciTransport> {
        None
    }
}

pub fn find_virtio_net_transport() -> Option<VirtioPciTransport> {
    arch::find_virtio_net_transport()
}

pub fn find_virtio_block_transport() -> Option<VirtioPciTransport> {
    arch::find_virtio_block_transport()
}
