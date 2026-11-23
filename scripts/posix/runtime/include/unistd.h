/* POSIX feature declarations used by the SMROS test ABI. */
#ifndef SMROS_POSIX_UNISTD_H
#define SMROS_POSIX_UNISTD_H 1

/* glibc supplies these option macros from bits/posix_opt.h.  Include its
 * declarations first, then advertise the optional scheduler groups that the
 * SMROS runtime implements. */
#if defined(__clang__)
# pragma clang diagnostic push
# pragma clang diagnostic ignored "-Winclude-next-absolute-path"
#endif
#include_next <unistd.h>
#if defined(__clang__)
# pragma clang diagnostic pop
#endif

#ifdef _POSIX_SPORADIC_SERVER
# undef _POSIX_SPORADIC_SERVER
#endif
#define _POSIX_SPORADIC_SERVER 200809L

#ifdef _POSIX_THREAD_SPORADIC_SERVER
# undef _POSIX_THREAD_SPORADIC_SERVER
#endif
#define _POSIX_THREAD_SPORADIC_SERVER 200809L

#endif /* SMROS_POSIX_UNISTD_H */
