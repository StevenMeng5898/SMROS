/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_INTERRUPT_H
#define _LINUX_INTERRUPT_H

#include <linux/types.h>

#define IRQF_SHARED 0x00000080u
#define IRQF_TRIGGER_NONE 0
#define IRQF_TRIGGER_RISING 0x00000001u
#define IRQF_TRIGGER_FALLING 0x00000002u
#define IRQF_TRIGGER_HIGH 0x00000004u
#define IRQF_TRIGGER_LOW 0x00000008u

enum irqreturn {
    IRQ_NONE = 0,
    IRQ_HANDLED = 1,
    IRQ_WAKE_THREAD = 2,
};

typedef enum irqreturn irqreturn_t;
typedef irqreturn_t (*irq_handler_t)(int, void *);

int request_irq(unsigned int irq, irq_handler_t handler, unsigned long flags,
                const char *name, void *dev);
void free_irq(unsigned int irq, void *dev);

#endif /* _LINUX_INTERRUPT_H */
