/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_DEVICE_H
#define _LINUX_DEVICE_H

#include <linux/compiler.h>
#include <linux/types.h>
#include <linux/printk.h>

struct device {
    void *driver_data;
    const char *init_name;
};

static inline void dev_set_drvdata(struct device *dev, void *data)
{
    dev->driver_data = data;
}

static inline void *dev_get_drvdata(const struct device *dev)
{
    return dev->driver_data;
}

#define dev_err(dev, fmt, ...) printk(KERN_ERR fmt, ##__VA_ARGS__)
#define dev_info(dev, fmt, ...) printk(KERN_INFO fmt, ##__VA_ARGS__)
#define dev_dbg(dev, fmt, ...) printk(KERN_DEBUG fmt, ##__VA_ARGS__)
#define dev_warn(dev, fmt, ...) printk(KERN_WARNING fmt, ##__VA_ARGS__)

#endif /* _LINUX_DEVICE_H */
