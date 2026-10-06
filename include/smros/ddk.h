/* SPDX-License-Identifier: MIT
 *
 * SMROS user-space Linux Driver Development Kit.
 *
 * This header is the C DDK surface for Linux-shaped drivers that run in the
 * SMROS user-level driver framework. The live runtime is implemented in
 * `src/user_level/drivers/linux.rs`. Rust drivers use
 * `crate::user_level::drivers::ddk`.
 *
 * The model follows Linux's device/driver/bus split:
 *   module_init -> register_*_driver -> probe(device)
 * plus misc/char, ioremap, request_irq, and dma_alloc_coherent helpers.
 *
 * Linux PCI drivers should include <linux/pci.h> instead of this header and
 * keep stock Linux C (see src/user_level/drivers/linuxcompat/edu.c).
 */

#ifndef SMROS_DDK_H
#define SMROS_DDK_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>

#define SMROS_DDK_API_VERSION 1u

#define SMROS_DDK_SUCCESS 0
#define SMROS_DDK_ERR_NOENT (-2)
#define SMROS_DDK_ERR_IO (-5)
#define SMROS_DDK_ERR_NOMEM (-12)
#define SMROS_DDK_ERR_BUSY (-16)
#define SMROS_DDK_ERR_EXIST (-17)
#define SMROS_DDK_ERR_NODEV (-19)
#define SMROS_DDK_ERR_INVAL (-22)
#define SMROS_DDK_ERR_TIMEDOUT (-110)

#define SMROS_DDK_BUS_PLATFORM 1u
#define SMROS_DDK_BUS_VIRTIO 2u
#define SMROS_DDK_BUS_PCI 3u

#define SMROS_DDK_PCI_ANY_ID 0xffffffffu
#define SMROS_DDK_VIRTIO_ID_ANY 0xffffffffu
#define SMROS_DDK_VIRTIO_ID_NET 1u
#define SMROS_DDK_VIRTIO_ID_BLOCK 2u

#define SMROS_DDK_MISC_DYNAMIC_MINOR 255
#define SMROS_DDK_IRQF_SHARED 0x80u

#define SMROS_DDK_HELLO_IOCTL_MAGIC 0x534d5244u /* 'SMRD' */

typedef void (*smros_ddk_irq_handler_t)(int irq, void *dev);

unsigned int smros_ddk_api_version(void);
unsigned int smros_ddk_module_count(void);
unsigned int smros_ddk_device_count(void);
int smros_ddk_bus_kind_valid(unsigned int kind);
int smros_ddk_probe_status_ok(int code);
int smros_ddk_id_match(unsigned int id, unsigned int table_id, unsigned int any_id);

void *smros_ddk_ioremap(uint64_t phys, unsigned long size);
void smros_ddk_iounmap(void *addr, unsigned long size);
uint8_t smros_ddk_readb(const volatile void *addr);
void smros_ddk_writeb(uint8_t value, volatile void *addr);
uint16_t smros_ddk_readw(const volatile void *addr);
void smros_ddk_writew(uint16_t value, volatile void *addr);
uint32_t smros_ddk_readl(const volatile void *addr);
void smros_ddk_writel(uint32_t value, volatile void *addr);

int smros_ddk_request_irq(unsigned int irq, smros_ddk_irq_handler_t handler,
                          unsigned long flags, const char *name, void *dev);
void smros_ddk_free_irq(unsigned int irq, void *dev);

void *smros_ddk_dma_alloc_coherent(unsigned long size, uint64_t *dma_handle);
void smros_ddk_dma_free_coherent(unsigned long size, void *cpu_addr,
                                 uint64_t dma_handle);

long smros_ddk_chrdev_read(const char *name, char *buf, unsigned long count,
                           long long *ppos);
long smros_ddk_chrdev_write(const char *name, const char *buf,
                            unsigned long count, long long *ppos);

#ifdef __cplusplus
}
#endif

#endif /* SMROS_DDK_H */
