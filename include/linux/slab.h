/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_SLAB_H
#define _LINUX_SLAB_H

#include <linux/gfp.h>
#include <linux/types.h>

void *kmalloc(size_t size, gfp_t flags);
void *kzalloc(size_t size, gfp_t flags);
void kfree(const void *ptr);

static inline void *kmalloc_array(size_t n, size_t size, gfp_t flags)
{
    return kmalloc(n * size, flags);
}

static inline void *kcalloc(size_t n, size_t size, gfp_t flags)
{
    return kzalloc(n * size, flags);
}

#endif /* _LINUX_SLAB_H */
