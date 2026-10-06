/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_KERNEL_H
#define _LINUX_KERNEL_H

#include <linux/compiler.h>
#include <linux/stddef.h>
#include <linux/types.h>
#include <linux/printk.h>
#include <linux/err.h>
#include <linux/errno.h>

#define ARRAY_SIZE(arr) (sizeof(arr) / sizeof((arr)[0]))

#define min(x, y) \
    ({ \
        __typeof__(x) _x = (x); \
        __typeof__(y) _y = (y); \
        _x < _y ? _x : _y; \
    })
#define max(x, y) \
    ({ \
        __typeof__(x) _x = (x); \
        __typeof__(y) _y = (y); \
        _x > _y ? _x : _y; \
    })

#define container_of(ptr, type, member) \
    ({ \
        const __typeof__(((type *)0)->member) *__mptr = (ptr); \
        (type *)((char *)__mptr - offsetof(type, member)); \
    })

#define ALIGN(x, a) (((x) + ((a) - 1)) & ~((a) - 1))
#define DIV_ROUND_UP(n, d) (((n) + (d) - 1) / (d))

#endif /* _LINUX_KERNEL_H */
