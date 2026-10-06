/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_STDDEF_H
#define _LINUX_STDDEF_H

#include <linux/types.h>

#undef offsetof
#define offsetof(TYPE, MEMBER) __builtin_offsetof(TYPE, MEMBER)

#endif /* _LINUX_STDDEF_H */
