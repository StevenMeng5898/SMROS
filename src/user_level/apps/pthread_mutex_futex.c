/*
 * pthread / futex mutex demo for SMROS.
 *
 * This is an AArch64 dynamic PIE. SMROS has no strace, so the demo uses the
 * same 0/1/2 futex word as glibc's default pthread_mutex and prints each
 * kernel entry itself:
 *
 *   uncontended -> userspace cmpxchg only, no futex() lines
 *   contended   -> FUTEX_WAIT_PRIVATE / FUTEX_WAKE_PRIVATE
 *
 * Build into /shared (rebuild the kernel after this so host_shared is embedded):
 *   make -C src/user_level/apps smros
 *
 * On SMROS:
 *   run pthread_mutex_futex.elf uncontended
 *   run pthread_mutex_futex.elf contended
 */

#define _GNU_SOURCE

#include <errno.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#ifndef SYS_futex
#ifdef __aarch64__
#define SYS_futex 98
#else
#define SYS_futex 202
#endif
#endif

#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX_WAIT_PRIVATE (FUTEX_WAIT | FUTEX_PRIVATE_FLAG)
#define FUTEX_WAKE_PRIVATE (FUTEX_WAKE | FUTEX_PRIVATE_FLAG)

/* 0 = free, 1 = locked, 2 = locked with waiters (glibc pthread_mutex). */
static atomic_uint mutex_word;
static unsigned long counter;
static atomic_uint wait_calls;
static atomic_uint wake_calls;

static void die(const char *msg)
{
    perror(msg);
    _exit(1);
}

static void write_trace(const char *text, size_t len)
{
    ssize_t wrote = write(1, text, len);
    (void)wrote;
}

static void trace_futex(const char *op, const void *addr, unsigned val)
{
    char line[160];
    int n = snprintf(line,
                     sizeof(line),
                     "futex(%p, %s, %u)\n",
                     addr,
                     op,
                     val);
    if (n > 0) {
        write_trace(line, (size_t)n);
    }
}

static long futex_wait(atomic_uint *word, unsigned expected)
{
    atomic_fetch_add_explicit(&wait_calls, 1, memory_order_relaxed);
    trace_futex("FUTEX_WAIT_PRIVATE", word, expected);
    return syscall(SYS_futex, word, FUTEX_WAIT_PRIVATE, expected, NULL, NULL, 0);
}

static long futex_wake(atomic_uint *word, unsigned count)
{
    atomic_fetch_add_explicit(&wake_calls, 1, memory_order_relaxed);
    trace_futex("FUTEX_WAKE_PRIVATE", word, count);
    return syscall(SYS_futex, word, FUTEX_WAKE_PRIVATE, count, NULL, NULL, 0);
}

static void mutex_lock(void)
{
    unsigned expected = 0;
    if (atomic_compare_exchange_strong_explicit(
            &mutex_word, &expected, 1, memory_order_acquire, memory_order_relaxed)) {
        return;
    }
    while (atomic_exchange_explicit(&mutex_word, 2, memory_order_acquire) != 0) {
        if (futex_wait(&mutex_word, 2) < 0 && errno != EAGAIN && errno != EINTR) {
            die("futex wait");
        }
    }
}

static void mutex_unlock(void)
{
    unsigned prev = atomic_exchange_explicit(&mutex_word, 0, memory_order_release);
    if (prev == 2) {
        if (futex_wake(&mutex_word, 1) < 0) {
            die("futex wake");
        }
    }
}

static void hold_lock_briefly(void)
{
    struct timespec ts = {
        .tv_sec = 0,
        .tv_nsec = 1000 * 1000, /* 1 ms; SMROS ticks are coarser, still yields */
    };
    while (nanosleep(&ts, &ts) < 0) {
        if (errno != EINTR) {
            die("nanosleep");
        }
    }
}

static void uncontended(unsigned long iters)
{
    unsigned long i;
    printf("mode=uncontended threads=1 iters=%lu\n", iters);
    printf("expect: no futex() lines (userspace cmpxchg fast path)\n");
    fflush(stdout);

    for (i = 0; i < iters; i++) {
        mutex_lock();
        counter++;
        mutex_unlock();
    }

    printf("counter=%lu wait=%u wake=%u\n",
           counter,
           atomic_load(&wait_calls),
           atomic_load(&wake_calls));
}

struct worker_arg {
    unsigned long iters;
};

static void *worker(void *arg)
{
    struct worker_arg *a = arg;
    unsigned long i;
    for (i = 0; i < a->iters; i++) {
        mutex_lock();
        counter++;
        hold_lock_briefly();
        mutex_unlock();
        sched_yield();
    }
    return NULL;
}

static void contended(unsigned int threads, unsigned long iters)
{
    pthread_t *tids;
    struct worker_arg arg = { .iters = iters };
    unsigned int i;

    printf("mode=contended threads=%u iters_per_thread=%lu\n", threads, iters);
    printf("expect: FUTEX_WAIT_PRIVATE / FUTEX_WAKE_PRIVATE\n");
    fflush(stdout);

    tids = calloc(threads, sizeof(*tids));
    if (tids == NULL) {
        die("calloc");
    }
    for (i = 0; i < threads; i++) {
        int rc = pthread_create(&tids[i], NULL, worker, &arg);
        if (rc != 0) {
            errno = rc;
            die("pthread_create");
        }
    }
    for (i = 0; i < threads; i++) {
        int rc = pthread_join(tids[i], NULL);
        if (rc != 0) {
            errno = rc;
            die("pthread_join");
        }
    }
    free(tids);

    printf("counter=%lu wait=%u wake=%u\n",
           counter,
           atomic_load(&wait_calls),
           atomic_load(&wake_calls));
    if (atomic_load(&wait_calls) == 0 || atomic_load(&wake_calls) == 0) {
        printf("warning: no contended futex path observed\n");
    }
}

static void usage(const char *argv0)
{
    fprintf(stderr,
            "usage: %s uncontended [iters]\n"
            "       %s contended [threads] [iters_per_thread]\n\n"
            "SMROS:\n"
            "  run pthread_mutex_futex.elf uncontended\n"
            "  run pthread_mutex_futex.elf contended\n",
            argv0,
            argv0);
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        usage(argv[0]);
        return 2;
    }

    if (strcmp(argv[1], "uncontended") == 0) {
        unsigned long iters = 200000;
        if (argc >= 3) {
            iters = strtoul(argv[2], NULL, 10);
        }
        if (iters == 0) {
            usage(argv[0]);
            return 2;
        }
        uncontended(iters);
        return 0;
    }

    if (strcmp(argv[1], "contended") == 0) {
        unsigned int threads = 4;
        unsigned long iters = 8;
        if (argc >= 3) {
            threads = (unsigned int)strtoul(argv[2], NULL, 10);
        }
        if (argc >= 4) {
            iters = strtoul(argv[3], NULL, 10);
        }
        if (threads < 2 || iters == 0) {
            fprintf(stderr, "contended mode needs >= 2 threads and > 0 iters\n");
            return 2;
        }
        contended(threads, iters);
        return 0;
    }

    usage(argv[0]);
    return 2;
}
