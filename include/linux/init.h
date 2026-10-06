/* SPDX-License-Identifier: MIT */
#ifndef _LINUX_INIT_H
#define _LINUX_INIT_H

#define __init
#define __exit
#define __initdata
#define __exitdata
#define __devinit
#define __devexit

#define module_init(fn) \
    int smros_linux_compat_init(void) \
    { \
        return (fn)(); \
    }

#define module_exit(fn) \
    void smros_linux_compat_exit(void) \
    { \
        (fn)(); \
    }

#endif /* _LINUX_INIT_H */
