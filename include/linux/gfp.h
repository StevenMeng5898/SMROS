/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_GFP_H
#define _LINUX_GFP_H

#include <linux/types.h>

#define ___GFP_ZERO 0x100u
#define GFP_KERNEL ((gfp_t)0)
#define GFP_ATOMIC ((gfp_t)0)
#define __GFP_ZERO ((gfp_t)___GFP_ZERO)

#endif /* _LINUX_GFP_H */
