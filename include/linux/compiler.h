/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_COMPILER_H
#define _LINUX_COMPILER_H

#define __user
#define __kernel
#define __safe
#define __force
#define __nocast
#define __iomem
#define __percpu
#define __rcu
#define __must_check
#define __always_inline inline __attribute__((always_inline))
#define __gnu_inline inline
#define __pure
#define __aligned(x) __attribute__((aligned(x)))
#define __packed __attribute__((packed))
#define __maybe_unused __attribute__((unused))
#define __always_unused __attribute__((unused))
#define __used __attribute__((used))
#define __weak __attribute__((weak))
#define __noreturn __attribute__((noreturn))
#define __printf(a, b) __attribute__((format(printf, a, b)))
#define __scanf(a, b) __attribute__((format(scanf, a, b)))

#define likely(x) __builtin_expect(!!(x), 1)
#define unlikely(x) __builtin_expect(!!(x), 0)
#define barrier() __asm__ __volatile__("" ::: "memory")

#define WRITE_ONCE(x, val) \
    do { \
        barrier(); \
        (x) = (val); \
        barrier(); \
    } while (0)
#define READ_ONCE(x) \
    ({ \
        barrier(); \
        __typeof__(x) __val = (x); \
        barrier(); \
        __val; \
    })

#endif /* _LINUX_COMPILER_H */
