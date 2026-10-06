/* SPDX-License-Identifier: MIT
 *
 * Linux pci_* entry points. Linux PCI driver source calls these names
 * unchanged; the bodies forward to the SMROS DDK PCI ABI.
 */

#include <linux/pci.h>
#include <linux/types.h>

int smros_ddk_pci_register_driver(struct pci_driver *drv);
void smros_ddk_pci_unregister_driver(struct pci_driver *drv);
int smros_ddk_pci_enable_device(struct pci_dev *dev);
void smros_ddk_pci_disable_device(struct pci_dev *dev);
int smros_ddk_pci_request_regions(struct pci_dev *dev, const char *res_name);
void smros_ddk_pci_release_regions(struct pci_dev *dev);
void smros_ddk_pci_set_master(struct pci_dev *dev, int enable);
void __iomem *smros_ddk_pci_iomap(struct pci_dev *dev, int bar, unsigned long maxlen);
void smros_ddk_pci_iounmap(struct pci_dev *dev, void __iomem *addr);
int smros_ddk_pci_read_config(const struct pci_dev *dev, int where, int size, u32 *val);
int smros_ddk_pci_write_config(struct pci_dev *dev, int where, int size, u32 val);

int pci_register_driver(struct pci_driver *drv)
{
    return smros_ddk_pci_register_driver(drv);
}

void pci_unregister_driver(struct pci_driver *drv)
{
    smros_ddk_pci_unregister_driver(drv);
}

int pci_enable_device(struct pci_dev *dev)
{
    return smros_ddk_pci_enable_device(dev);
}

void pci_disable_device(struct pci_dev *dev)
{
    smros_ddk_pci_disable_device(dev);
}

int pci_request_regions(struct pci_dev *dev, const char *res_name)
{
    return smros_ddk_pci_request_regions(dev, res_name);
}

void pci_release_regions(struct pci_dev *dev)
{
    smros_ddk_pci_release_regions(dev);
}

void pci_set_master(struct pci_dev *dev)
{
    smros_ddk_pci_set_master(dev, 1);
}

void pci_clear_master(struct pci_dev *dev)
{
    smros_ddk_pci_set_master(dev, 0);
}

void __iomem *pci_iomap(struct pci_dev *dev, int bar, unsigned long maxlen)
{
    return smros_ddk_pci_iomap(dev, bar, maxlen);
}

void pci_iounmap(struct pci_dev *dev, void __iomem *addr)
{
    smros_ddk_pci_iounmap(dev, addr);
}

int pci_read_config_byte(const struct pci_dev *dev, int where, u8 *val)
{
    u32 tmp = 0;
    int err;

    if (!val)
        return -EINVAL;
    err = smros_ddk_pci_read_config(dev, where, 1, &tmp);
    if (!err)
        *val = (u8)tmp;
    return err;
}

int pci_read_config_word(const struct pci_dev *dev, int where, u16 *val)
{
    u32 tmp = 0;
    int err;

    if (!val)
        return -EINVAL;
    err = smros_ddk_pci_read_config(dev, where, 2, &tmp);
    if (!err)
        *val = (u16)tmp;
    return err;
}

int pci_read_config_dword(const struct pci_dev *dev, int where, u32 *val)
{
    if (!val)
        return -EINVAL;
    return smros_ddk_pci_read_config(dev, where, 4, val);
}

int pci_write_config_byte(struct pci_dev *dev, int where, u8 val)
{
    return smros_ddk_pci_write_config(dev, where, 1, val);
}

int pci_write_config_word(struct pci_dev *dev, int where, u16 val)
{
    return smros_ddk_pci_write_config(dev, where, 2, val);
}

int pci_write_config_dword(struct pci_dev *dev, int where, u32 val)
{
    return smros_ddk_pci_write_config(dev, where, 4, val);
}
