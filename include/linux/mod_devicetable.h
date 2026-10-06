/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_MOD_DEVICETABLE_H
#define _LINUX_MOD_DEVICETABLE_H

#include <linux/types.h>

#define PCI_ANY_ID (~0u)

struct pci_device_id {
    u32 vendor;
    u32 device;
    u32 subvendor;
    u32 subdevice;
    u32 class;
    u32 class_mask;
    kernel_ulong_t driver_data;
};

#endif /* _LINUX_MOD_DEVICETABLE_H */
