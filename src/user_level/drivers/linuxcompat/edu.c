// SPDX-License-Identifier: GPL-2.0
/*
 * QEMU EDU PCI driver
 *
 * Matches the QEMU educational PCI device (hw/misc/edu.c):
 *   vendor 0x1234, device 0x11e8, ident 0xRRrr00ED
 *
 * This file is ordinary Linux PCI driver C. It includes only Linux kernel
 * headers and uses module_pci_driver / pci_* / ioread32 / kzalloc.
 */

#include <linux/module.h>
#include <linux/pci.h>
#include <linux/io.h>
#include <linux/slab.h>

#define EDU_VENDOR_ID 0x1234
#define EDU_DEVICE_ID 0x11e8
#define EDU_IDENT_MASK 0xffff
#define EDU_IDENT_MAGIC 0x00ed
#define EDU_REG_IDENT 0x00
#define EDU_REG_LIVE 0x04

struct edu_priv {
    void __iomem *mmio;
    u32 ident;
};

static int edu_probe(struct pci_dev *pdev, const struct pci_device_id *id)
{
    struct edu_priv *priv;
    u32 ident;
    int err;

    err = pci_enable_device(pdev);
    if (err)
        return err;

    err = pci_request_regions(pdev, "edu");
    if (err)
        goto err_disable;

    pci_set_master(pdev);

    priv = kzalloc(sizeof(*priv), GFP_KERNEL);
    if (!priv) {
        err = -ENOMEM;
        goto err_release;
    }

    priv->mmio = pci_iomap(pdev, 0, 0);
    if (!priv->mmio) {
        err = -ENOMEM;
        goto err_free;
    }

    ident = ioread32(priv->mmio + EDU_REG_IDENT);
    if ((ident & EDU_IDENT_MASK) != EDU_IDENT_MAGIC) {
        err = -ENODEV;
        goto err_unmap;
    }

    /* Liveness register: writing X reads back as ~X on real EDU hardware. */
    iowrite32(0xa5a5a5a5, priv->mmio + EDU_REG_LIVE);

    priv->ident = ident;
    pci_set_drvdata(pdev, priv);
    dev_info(&pdev->dev, "edu: ident=0x%08x bar0=%p\n", ident, priv->mmio);
    return 0;

err_unmap:
    pci_iounmap(pdev, priv->mmio);
err_free:
    kfree(priv);
err_release:
    pci_release_regions(pdev);
err_disable:
    pci_disable_device(pdev);
    return err;
}

static void edu_remove(struct pci_dev *pdev)
{
    struct edu_priv *priv = pci_get_drvdata(pdev);

    if (!priv)
        return;
    pci_iounmap(pdev, priv->mmio);
    kfree(priv);
    pci_release_regions(pdev);
    pci_disable_device(pdev);
}

static const struct pci_device_id edu_ids[] = {
    { PCI_DEVICE(EDU_VENDOR_ID, EDU_DEVICE_ID) },
    { 0, }
};
MODULE_DEVICE_TABLE(pci, edu_ids);

static struct pci_driver edu_driver = {
    .name = "edu",
    .id_table = edu_ids,
    .probe = edu_probe,
    .remove = edu_remove,
};

module_pci_driver(edu_driver);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("SMROS");
MODULE_DESCRIPTION("QEMU EDU PCI driver");
