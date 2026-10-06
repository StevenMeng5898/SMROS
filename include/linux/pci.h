/* SPDX-License-Identifier: MIT
 *
 * Source-level Linux PCI driver API for the SMROS user-space DDK.
 * Layout is LP64 source-compatible for the fields Linux PCI drivers use;
 * it is not a Linux kernel binary ABI.
 */
#ifndef _LINUX_PCI_H
#define _LINUX_PCI_H

#include <linux/device.h>
#include <linux/errno.h>
#include <linux/ioport.h>
#include <linux/mod_devicetable.h>
#include <linux/types.h>

#define PCI_NUM_RESOURCES 6
#define DEVICE_COUNT_RESOURCE PCI_NUM_RESOURCES
#define PCI_DEVFN(slot, func) ((((slot)&0x1f) << 3) | ((func)&0x07))
#define PCI_SLOT(devfn) (((devfn) >> 3) & 0x1f)
#define PCI_FUNC(devfn) ((devfn)&0x07)

#define PCI_DEVICE(vend, dev) \
    .vendor = (vend), .device = (dev), .subvendor = PCI_ANY_ID, \
    .subdevice = PCI_ANY_ID

#define PCI_DEVICE_CLASS(dev_class, dev_class_mask) \
    .vendor = PCI_ANY_ID, .device = PCI_ANY_ID, .subvendor = PCI_ANY_ID, \
    .subdevice = PCI_ANY_ID, .class = (dev_class), .class_mask = (dev_class_mask)

struct pci_dev {
    struct device dev;
    unsigned int devfn;
    unsigned short vendor;
    unsigned short device;
    unsigned short subsystem_vendor;
    unsigned short subsystem_device;
    unsigned int class;
    u8 revision;
    unsigned int irq;
    struct resource resource[PCI_NUM_RESOURCES];
};

struct pci_driver {
    const char *name;
    const struct pci_device_id *id_table;
    int (*probe)(struct pci_dev *dev, const struct pci_device_id *id);
    void (*remove)(struct pci_dev *dev);
};

int pci_register_driver(struct pci_driver *drv);
void pci_unregister_driver(struct pci_driver *drv);
int pci_enable_device(struct pci_dev *dev);
void pci_disable_device(struct pci_dev *dev);
int pci_request_regions(struct pci_dev *dev, const char *res_name);
void pci_release_regions(struct pci_dev *dev);
void pci_set_master(struct pci_dev *dev);
void pci_clear_master(struct pci_dev *dev);
void __iomem *pci_iomap(struct pci_dev *dev, int bar, unsigned long maxlen);
void pci_iounmap(struct pci_dev *dev, void __iomem *addr);
int pci_read_config_byte(const struct pci_dev *dev, int where, u8 *val);
int pci_read_config_word(const struct pci_dev *dev, int where, u16 *val);
int pci_read_config_dword(const struct pci_dev *dev, int where, u32 *val);
int pci_write_config_byte(struct pci_dev *dev, int where, u8 val);
int pci_write_config_word(struct pci_dev *dev, int where, u16 val);
int pci_write_config_dword(struct pci_dev *dev, int where, u32 val);

static inline void pci_set_drvdata(struct pci_dev *pdev, void *data)
{
    dev_set_drvdata(&pdev->dev, data);
}

static inline void *pci_get_drvdata(struct pci_dev *pdev)
{
    return dev_get_drvdata(&pdev->dev);
}

static inline resource_size_t pci_resource_start(const struct pci_dev *dev, int bar)
{
    return dev->resource[bar].start;
}

static inline resource_size_t pci_resource_end(const struct pci_dev *dev, int bar)
{
    return dev->resource[bar].end;
}

static inline unsigned long pci_resource_flags(const struct pci_dev *dev, int bar)
{
    return dev->resource[bar].flags;
}

static inline resource_size_t pci_resource_len(const struct pci_dev *dev, int bar)
{
    if (dev->resource[bar].start == 0 && dev->resource[bar].end == 0)
        return 0;
    return dev->resource[bar].end - dev->resource[bar].start + 1;
}

#define module_pci_driver(__pci_driver) \
    module_driver(__pci_driver, pci_register_driver, pci_unregister_driver)

#endif /* _LINUX_PCI_H */
