/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_TYPES_H
#define _LINUX_TYPES_H

typedef __UINT8_TYPE__ u8;
typedef __UINT16_TYPE__ u16;
typedef __UINT32_TYPE__ u32;
typedef __UINT64_TYPE__ u64;
typedef __INT8_TYPE__ s8;
typedef __INT16_TYPE__ s16;
typedef __INT32_TYPE__ s32;
typedef __INT64_TYPE__ s64;

typedef u8 __u8;
typedef u16 __u16;
typedef u32 __u32;
typedef u64 __u64;

typedef __SIZE_TYPE__ size_t;
typedef __INTPTR_TYPE__ ssize_t;
typedef __INTPTR_TYPE__ intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;
typedef long long loff_t;
typedef unsigned long resource_size_t;
typedef unsigned long kernel_ulong_t;
typedef unsigned int gfp_t;
typedef unsigned int __poll_t;

typedef _Bool bool;
#define true 1
#define false 0

#ifndef NULL
#define NULL ((void *)0)
#endif

#define __bitwise
typedef u32 __bitwise __be32;
typedef u16 __bitwise __be16;
typedef u32 __bitwise __le32;
typedef u16 __bitwise __le16;

#endif /* _LINUX_TYPES_H */
