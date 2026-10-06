/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_DMA_MAPPING_H
#define _LINUX_DMA_MAPPING_H

#include <linux/device.h>
#include <linux/types.h>

typedef u64 dma_addr_t;

void *dma_alloc_coherent(struct device *dev, size_t size, dma_addr_t *dma_handle,
                         gfp_t gfp);
void dma_free_coherent(struct device *dev, size_t size, void *cpu_addr,
                       dma_addr_t dma_handle);

#endif /* _LINUX_DMA_MAPPING_H */
