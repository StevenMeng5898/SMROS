/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_MODULE_H
#define _LINUX_MODULE_H

#include <linux/init.h>
#include <linux/kernel.h>
#include <linux/moduleparam.h>

#define THIS_MODULE ((struct module *)0)

struct module;

#define MODULE_LICENSE(x)
#define MODULE_AUTHOR(x)
#define MODULE_DESCRIPTION(x)
#define MODULE_VERSION(x)
#define MODULE_ALIAS(x)
#define MODULE_DEVICE_TABLE(type, name)
#define MODULE_INFO(tag, info)

#define module_driver(__driver, __register, __unregister, ...) \
    static int __init __driver##_init(void) \
    { \
        return __register(&(__driver), ##__VA_ARGS__); \
    } \
    static void __exit __driver##_exit(void) \
    { \
        __unregister(&(__driver), ##__VA_ARGS__); \
    } \
    module_init(__driver##_init); \
    module_exit(__driver##_exit)

#endif /* _LINUX_MODULE_H */
