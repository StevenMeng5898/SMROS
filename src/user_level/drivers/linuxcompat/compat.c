/* SPDX-License-Identifier: MIT
 *
 * Freestanding C runtime for Linux-shaped SMROS DDK drivers.
 * Linux PCI drivers include Linux kernel headers; this file is not part of those
 * drivers and may call SMROS ABI helpers.
 */

#include <linux/compiler.h>
#include <linux/dma-mapping.h>
#include <linux/gfp.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/kernel.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/types.h>

typedef __builtin_va_list va_list;
#define va_start(v, l) __builtin_va_start(v, l)
#define va_end(v) __builtin_va_end(v)
#define va_arg(v, t) __builtin_va_arg(v, t)

void smros_ddk_puts(const char *s);
void *smros_ddk_kmalloc(unsigned long size, unsigned int gfp);
void smros_ddk_kfree(void *ptr);
void *smros_ddk_ioremap(u64 phys, unsigned long size);
void smros_ddk_iounmap(void *addr, unsigned long size);
int smros_ddk_request_irq(unsigned int irq, void (*handler)(int, void *),
                          unsigned long flags, const char *name, void *dev);
void smros_ddk_free_irq(unsigned int irq, void *dev);
void *smros_ddk_dma_alloc_coherent(unsigned long size, u64 *dma_handle);
void smros_ddk_dma_free_coherent(unsigned long size, void *cpu_addr, u64 dma_handle);

static void emit_char(char *buf, unsigned long size, unsigned long *used, char ch)
{
    if (*used + 1 < size)
        buf[*used] = ch;
    *used += 1;
}

static void emit_str(char *buf, unsigned long size, unsigned long *used, const char *s)
{
    if (!s)
        s = "(null)";
    while (*s)
        emit_char(buf, size, used, *s++);
}

static void emit_uint(char *buf, unsigned long size, unsigned long *used,
                      unsigned long long value, unsigned base, int width, int zero_pad,
                      int upper)
{
    char tmp[32];
    const char *digits = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    int n = 0;
    int i;

    if (base < 2 || base > 16)
        base = 10;
    do {
        tmp[n++] = digits[value % (unsigned)base];
        value /= (unsigned)base;
    } while (value && n < (int)sizeof(tmp));

    while (width > n) {
        emit_char(buf, size, used, zero_pad ? '0' : ' ');
        width--;
    }
    for (i = n - 1; i >= 0; i--)
        emit_char(buf, size, used, tmp[i]);
}

static int vscnprintf(char *buf, unsigned long size, const char *fmt, va_list ap)
{
    unsigned long used = 0;
    const char *p = fmt;

    if (!buf || !fmt || size == 0)
        return 0;

    if ((unsigned char)p[0] == KERN_SOH_ASCII && p[1] >= '0' && p[1] <= '9')
        p += 2;
    else if ((unsigned char)p[0] == KERN_SOH_ASCII && p[1] == 'c')
        p += 2;

    while (*p) {
        int width = 0;
        int zero_pad = 0;
        int long_count = 0;

        if (*p != '%') {
            emit_char(buf, size, &used, *p++);
            continue;
        }
        p++;
        if (*p == '%') {
            emit_char(buf, size, &used, '%');
            p++;
            continue;
        }
        if (*p == '0') {
            zero_pad = 1;
            p++;
        }
        while (*p >= '0' && *p <= '9') {
            width = width * 10 + (*p - '0');
            p++;
        }
        while (*p == 'l' || *p == 'z' || *p == 't' || *p == 'h') {
            if (*p == 'l')
                long_count++;
            p++;
        }
        switch (*p) {
        case 's':
            emit_str(buf, size, &used, va_arg(ap, const char *));
            break;
        case 'c':
            emit_char(buf, size, &used, (char)va_arg(ap, int));
            break;
        case 'd':
        case 'i': {
            long long value = long_count >= 2 ? va_arg(ap, long long)
                                              : (long_count == 1 ? va_arg(ap, long)
                                                                 : va_arg(ap, int));
            if (value < 0) {
                emit_char(buf, size, &used, '-');
                if (width)
                    width--;
                emit_uint(buf, size, &used, (unsigned long long)(-value), 10, width, zero_pad,
                          0);
            } else {
                emit_uint(buf, size, &used, (unsigned long long)value, 10, width, zero_pad, 0);
            }
            break;
        }
        case 'u': {
            unsigned long long value =
                long_count >= 2 ? va_arg(ap, unsigned long long)
                                : (long_count == 1 ? va_arg(ap, unsigned long)
                                                   : va_arg(ap, unsigned int));
            emit_uint(buf, size, &used, value, 10, width, zero_pad, 0);
            break;
        }
        case 'x':
        case 'X': {
            unsigned long long value =
                long_count >= 2 ? va_arg(ap, unsigned long long)
                                : (long_count == 1 ? va_arg(ap, unsigned long)
                                                   : va_arg(ap, unsigned int));
            emit_uint(buf, size, &used, value, 16, width, zero_pad, *p == 'X');
            break;
        }
        case 'p': {
            unsigned long long value = (unsigned long long)(uintptr_t)va_arg(ap, void *);
            emit_str(buf, size, &used, "0x");
            emit_uint(buf, size, &used, value, 16, width > 2 ? width - 2 : 0, 1, 0);
            break;
        }
        default:
            emit_char(buf, size, &used, '%');
            if (*p)
                emit_char(buf, size, &used, *p);
            break;
        }
        if (*p)
            p++;
    }

    if (size) {
        unsigned long term = used < size ? used : size - 1;
        buf[term] = 0;
    }
    return used < size ? (int)used : (int)(size - 1);
}

int printk(const char *fmt, ...)
{
    char buf[192];
    va_list ap;
    int n;

    va_start(ap, fmt);
    n = vscnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    smros_ddk_puts(buf);
    return n;
}

void *kmalloc(size_t size, gfp_t flags)
{
    void *ptr = smros_ddk_kmalloc((unsigned long)size, (unsigned int)flags);
    if (ptr && (flags & __GFP_ZERO))
        memset(ptr, 0, size);
    return ptr;
}

void *kzalloc(size_t size, gfp_t flags)
{
    return kmalloc(size, flags | __GFP_ZERO);
}

void kfree(const void *ptr)
{
    smros_ddk_kfree((void *)ptr);
}

void __iomem *ioremap(resource_size_t offset, unsigned long size)
{
    return smros_ddk_ioremap((u64)offset, size);
}

void iounmap(volatile void __iomem *addr)
{
    smros_ddk_iounmap((void *)addr, 0);
}

int request_irq(unsigned int irq, irq_handler_t handler, unsigned long flags,
                const char *name, void *dev)
{
    return smros_ddk_request_irq(irq, (void (*)(int, void *))handler, flags, name, dev);
}

void free_irq(unsigned int irq, void *dev)
{
    smros_ddk_free_irq(irq, dev);
}

void *dma_alloc_coherent(struct device *dev, size_t size, dma_addr_t *dma_handle, gfp_t gfp)
{
    (void)dev;
    (void)gfp;
    return smros_ddk_dma_alloc_coherent((unsigned long)size, dma_handle);
}

void dma_free_coherent(struct device *dev, size_t size, void *cpu_addr, dma_addr_t dma_handle)
{
    (void)dev;
    smros_ddk_dma_free_coherent((unsigned long)size, cpu_addr, dma_handle);
}
