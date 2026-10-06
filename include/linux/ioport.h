/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_IOPORT_H
#define _LINUX_IOPORT_H

#include <linux/types.h>

#define IORESOURCE_IO 0x00000100ul
#define IORESOURCE_MEM 0x00000200ul
#define IORESOURCE_IRQ 0x00000400ul
#define IORESOURCE_PREFETCH 0x00002000ul

struct resource {
    resource_size_t start;
    resource_size_t end;
    const char *name;
    unsigned long flags;
};

#endif /* _LINUX_IOPORT_H */
