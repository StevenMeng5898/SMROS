/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_STRING_H
#define _LINUX_STRING_H

#include <linux/types.h>

static inline void *memcpy(void *dest, const void *src, size_t n)
{
    return __builtin_memcpy(dest, src, n);
}

static inline void *memset(void *s, int c, size_t n)
{
    return __builtin_memset(s, c, n);
}

static inline int memcmp(const void *s1, const void *s2, size_t n)
{
    return __builtin_memcmp(s1, s2, n);
}

static inline size_t strlen(const char *s)
{
    return __builtin_strlen(s);
}

static inline int strcmp(const char *s1, const char *s2)
{
    return __builtin_strcmp(s1, s2);
}

#endif /* _LINUX_STRING_H */
