//! SMROS user-space Linux Driver Development Kit.
//!
//! The DDK is a Linux-shaped driver model that runs in the SMROS user-level
//! layer. Linux driver demos register through `register_module` and bind with
//! `platform_driver` / `virtio_driver` / `miscdevice` APIs.
//!
//! C drivers may use `include/smros/ddk.h`. Linux PCI drivers include
//! `<linux/pci.h>` and keep stock Linux C, as in
//! `src/user_level/drivers/linuxcompat/edu.c`.
//!
//! Live examples:
//! - `virtio_blk` and `virtio_net` (moved onto the virtio bus)
//! - `hello` misc character driver
//! - `dummy` platform driver
//! - `edu` QEMU EDU PCI driver (Linux C)
//!
//! See `docs/DDK.md` for the API reference.

#![allow(unused_imports)]

pub use super::linux::{
    attach, buses, chrdev_ioctl, chrdev_open, chrdev_read, chrdev_write, chrdevs,
    dma_alloc_coherent, dma_free_coherent, free_irq, intern_name, ioremap, iounmap, misc_register,
    modules, readb, readl, readw, register_module, register_pci_device, register_pci_driver,
    register_platform_device, register_platform_driver, register_virtio_driver,
    register_virtio_mmio_device, register_virtio_pci_device, request_irq, stats,
    virtio_mmio_device_id, writeb, writel, writew, BusInfo, BusKind, ChrdevInfo, FileOperations,
    LinuxDdkStats, LinuxModule, MiscDevice, ModuleInfo, PciBar, PciDevice, PciDeviceId, PciDriver,
    PlatformDevice, PlatformDriver, VirtioDevice, VirtioDeviceId, VirtioDriver, VirtioTransport,
    BUS_PCI, BUS_PLATFORM, BUS_VIRTIO, CHRDEV_MINOR_MAX, DDK_API_VERSION, DMA_ALLOC_MAX,
    IOREMAP_MAX, IORESOURCE_IO, IORESOURCE_MEM, IRQF_SHARED, IRQ_MAX, MISC_DYNAMIC_MINOR,
    PCI_ANY_ID, VIRTIO_ID_ANY, VIRTIO_ID_BLOCK, VIRTIO_ID_NET,
};
