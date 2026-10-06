/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_PRINTK_H
#define _LINUX_PRINTK_H

#include <linux/compiler.h>

#define KERN_SOH "\001"
#define KERN_SOH_ASCII '\001'
#define KERN_EMERG KERN_SOH "0"
#define KERN_ALERT KERN_SOH "1"
#define KERN_CRIT KERN_SOH "2"
#define KERN_ERR KERN_SOH "3"
#define KERN_WARNING KERN_SOH "4"
#define KERN_NOTICE KERN_SOH "5"
#define KERN_INFO KERN_SOH "6"
#define KERN_DEBUG KERN_SOH "7"
#define KERN_CONT KERN_SOH "c"

int printk(const char *fmt, ...) __printf(1, 2);

#define pr_fmt(fmt) fmt
#define pr_err(fmt, ...) printk(KERN_ERR pr_fmt(fmt), ##__VA_ARGS__)
#define pr_warn(fmt, ...) printk(KERN_WARNING pr_fmt(fmt), ##__VA_ARGS__)
#define pr_info(fmt, ...) printk(KERN_INFO pr_fmt(fmt), ##__VA_ARGS__)
#define pr_debug(fmt, ...) printk(KERN_DEBUG pr_fmt(fmt), ##__VA_ARGS__)
#define pr_cont(fmt, ...) printk(KERN_CONT pr_fmt(fmt), ##__VA_ARGS__)

#endif /* _LINUX_PRINTK_H */
