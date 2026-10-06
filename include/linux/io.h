/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_IO_H
#define _LINUX_IO_H

#include <asm/io.h>

#define ioread8(addr) readb(addr)
#define ioread16(addr) readw(addr)
#define ioread32(addr) readl(addr)
#define iowrite8(v, addr) writeb((v), (addr))
#define iowrite16(v, addr) writew((v), (addr))
#define iowrite32(v, addr) writel((v), (addr))

void __iomem *ioremap(resource_size_t offset, unsigned long size);
void iounmap(volatile void __iomem *addr);

#endif /* _LINUX_IO_H */
