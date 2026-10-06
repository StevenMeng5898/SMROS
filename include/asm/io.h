/* SPDX-License-Identifier: MIT */
#ifndef _ASM_IO_H
#define _ASM_IO_H

#include <linux/compiler.h>
#include <linux/types.h>

static inline u8 readb(const volatile void __iomem *addr)
{
    return *(const volatile u8 *)addr;
}

static inline u16 readw(const volatile void __iomem *addr)
{
    return *(const volatile u16 *)addr;
}

static inline u32 readl(const volatile void __iomem *addr)
{
    return *(const volatile u32 *)addr;
}

static inline void writeb(u8 value, volatile void __iomem *addr)
{
    *(volatile u8 *)addr = value;
}

static inline void writew(u16 value, volatile void __iomem *addr)
{
    *(volatile u16 *)addr = value;
}

static inline void writel(u32 value, volatile void __iomem *addr)
{
    *(volatile u32 *)addr = value;
}

#endif /* _ASM_IO_H */
