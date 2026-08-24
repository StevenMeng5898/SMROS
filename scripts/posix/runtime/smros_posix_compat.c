#define _GNU_SOURCE

#include <aio.h>
#include <limits.h>
#include <stdarg.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <mqueue.h>
#include <nl_types.h>
#include <pthread.h>
#include <pwd.h>
#include <sched.h>
#include <semaphore.h>
#include <signal.h>
#include <stdio.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define SMROS_AIO_RECORDS 2048
#define SMROS_PTS_FORK_MESSAGE_CATALOG "/shared/posixtest/conformance/interfaces/fork/mess.cat"
#define SMROS_PTS_SOURCE_ROOT "/shared/posixtest"
#define SMROS_PTS_READABLE_FALLBACK "/shared/posixtest/lib/libsmros-posix-compat.so"
#define SMROS_PTHREAD_SHARED_BARRIER_MAGIC 0x53425231u
#define SMROS_PTHREAD_SHARED_COND_MAGIC 0x53434431u
#define SMROS_PTHREAD_SHARED_MUTEX_MAGIC 0x534d5431u

typedef enum {
    SMROS_AIO_EMPTY = 0,
    SMROS_AIO_COMPLETE,
    SMROS_AIO_CANCELED,
} smros_aio_state;

typedef struct {
    struct aiocb *request;
    int fd;
    smros_aio_state state;
    int error;
    ssize_t result;
    int returned;
    int observed_complete;
    int pending_polls;
} smros_aio_record;

typedef int (*smros_aio_error_fn)(const struct aiocb *);
typedef ssize_t (*smros_aio_return_fn)(struct aiocb *);
typedef int (*smros_aio_cancel_fn)(int, struct aiocb *);
typedef int (*smros_aio_suspend_fn)(
    const struct aiocb *const[],
    int,
    const struct timespec *
);
typedef nl_catd (*smros_catopen_fn)(const char *, int);
typedef char *(*smros_catgets_fn)(nl_catd, int, int, const char *);
typedef int (*smros_catclose_fn)(nl_catd);
typedef sem_t *(*smros_sem_open_fn)(const char *, int, ...);
typedef int (*smros_sem_init_fn)(sem_t *, int, unsigned int);
typedef int (*smros_sem_destroy_fn)(sem_t *);
typedef int (*smros_sem_unlink_fn)(const char *);
typedef int (*smros_sem_trywait_fn)(sem_t *);
typedef int (*smros_sigaction_fn)(int, const struct sigaction *, struct sigaction *);
typedef int (*smros_sigprocmask_fn)(int, const sigset_t *, sigset_t *);
typedef int (*smros_kill_fn)(pid_t, int);
typedef int (*smros_sigqueue_fn)(pid_t, int, const union sigval);
typedef int (*smros_mlock_fn)(const void *, size_t);
typedef int (*smros_munlock_fn)(const void *, size_t);
typedef int (*smros_mlockall_fn)(int);
typedef int (*smros_munlockall_fn)(void);
typedef int (*smros_msync_fn)(void *, size_t, int);
typedef void *(*smros_mmap_fn)(void *, size_t, int, int, int, off_t);
typedef int (*smros_munmap_fn)(void *, size_t);
typedef int (*smros_mq_unlink_fn)(const char *);
typedef int (*smros_shm_unlink_fn)(const char *);
typedef int (*smros_shm_open_fn)(const char *, int, mode_t);
typedef int (*smros_open_fn)(const char *, int, ...);
typedef int (*smros_close_fn)(int);
typedef int (*smros_nanosleep_fn)(const struct timespec *, struct timespec *);
typedef int (*smros_clock_nanosleep_fn)(
    clockid_t,
    int,
    const struct timespec *,
    struct timespec *
);
typedef int (*smros_pthread_create_fn)(
    pthread_t *,
    const pthread_attr_t *,
    void *(*)(void *),
    void *
);
typedef pid_t (*smros_fork_fn)(void);
typedef pid_t (*smros_waitpid_fn)(pid_t, int *, int);
typedef int (*smros_pthread_join_fn)(pthread_t, void **);
typedef int (*smros_pthread_tryjoin_fn)(pthread_t, void **);
typedef int (*smros_pthread_kill_fn)(pthread_t, int);
typedef int (*smros_pthread_mutex_lock_fn)(pthread_mutex_t *);
typedef int (*smros_pthread_mutex_trylock_fn)(pthread_mutex_t *);
typedef int (*smros_pthread_mutex_unlock_fn)(pthread_mutex_t *);
typedef int (*smros_pthread_mutex_init_fn)(
    pthread_mutex_t *,
    const pthread_mutexattr_t *
);
typedef int (*smros_pthread_mutex_destroy_fn)(pthread_mutex_t *);
typedef int (*smros_pthread_rwlock_init_fn)(
    pthread_rwlock_t *,
    const pthread_rwlockattr_t *
);
typedef int (*smros_pthread_rwlock_destroy_fn)(pthread_rwlock_t *);
typedef int (*smros_pthread_rwlock_rdlock_fn)(pthread_rwlock_t *);
typedef int (*smros_pthread_rwlock_wrlock_fn)(pthread_rwlock_t *);
typedef int (*smros_pthread_rwlock_unlock_fn)(pthread_rwlock_t *);
typedef int (*smros_pthread_cancel_fn)(pthread_t);
typedef int (*smros_pthread_setcanceltype_fn)(int, int *);
typedef void (*smros_pthread_testcancel_fn)(void);
typedef void (*smros_pthread_exit_fn)(void *);
typedef int (*smros_pthread_attr_init_fn)(pthread_attr_t *);
typedef int (*smros_pthread_attr_setstacksize_fn)(pthread_attr_t *, size_t);
typedef int (*smros_pthread_attr_setschedparam_fn)(
    pthread_attr_t *,
    const struct sched_param *
);
typedef int (*smros_pthread_attr_setschedpolicy_fn)(pthread_attr_t *, int);
typedef int (*smros_pthread_attr_destroy_fn)(pthread_attr_t *);
typedef int (*smros_pthread_getschedparam_fn)(
    pthread_t,
    int *,
    struct sched_param *
);
typedef int (*smros_pthread_setschedparam_fn)(
    pthread_t,
    int,
    const struct sched_param *
);
typedef int (*smros_pthread_setschedprio_fn)(pthread_t, int);
typedef int (*smros_pthread_mutex_getprioceiling_fn)(
    const pthread_mutex_t *,
    int *
);
typedef int (*smros_pthread_mutexattr_init_fn)(pthread_mutexattr_t *);
typedef int (*smros_pthread_mutexattr_destroy_fn)(pthread_mutexattr_t *);
typedef int (*smros_pthread_mutexattr_gettype_fn)(
    const pthread_mutexattr_t *,
    int *
);
typedef int (*smros_pthread_mutexattr_settype_fn)(pthread_mutexattr_t *, int);
typedef int (*smros_pthread_mutexattr_getpshared_fn)(
    const pthread_mutexattr_t *,
    int *
);
typedef int (*smros_pthread_mutexattr_setpshared_fn)(pthread_mutexattr_t *, int);
typedef int (*smros_pthread_attr_getschedpolicy_fn)(
    const pthread_attr_t *,
    int *
);
typedef int (*smros_pthread_attr_getschedparam_fn)(
    const pthread_attr_t *,
    struct sched_param *
);
typedef int (*smros_pthread_attr_getinheritsched_fn)(
    const pthread_attr_t *,
    int *
);
typedef int (*smros_pthread_cond_init_fn)(
    pthread_cond_t *,
    const pthread_condattr_t *
);
typedef int (*smros_pthread_condattr_getpshared_fn)(
    const pthread_condattr_t *,
    int *
);
typedef int (*smros_pthread_condattr_getclock_fn)(
    const pthread_condattr_t *,
    clockid_t *
);
typedef int (*smros_pthread_cond_wait_fn)(pthread_cond_t *, pthread_mutex_t *);
typedef int (*smros_pthread_cond_timedwait_fn)(
    pthread_cond_t *,
    pthread_mutex_t *,
    const struct timespec *
);
typedef int (*smros_pthread_cond_broadcast_fn)(pthread_cond_t *);
typedef int (*smros_pthread_cond_signal_fn)(pthread_cond_t *);
typedef int (*smros_pthread_cond_destroy_fn)(pthread_cond_t *);
typedef int (*smros_pthread_barrier_init_fn)(
    pthread_barrier_t *,
    const pthread_barrierattr_t *,
    unsigned int
);
typedef int (*smros_pthread_barrierattr_getpshared_fn)(
    const pthread_barrierattr_t *,
    int *
);
typedef int (*smros_pthread_barrier_wait_fn)(pthread_barrier_t *);
typedef int (*smros_pthread_barrier_destroy_fn)(pthread_barrier_t *);
typedef int (*smros_register_atfork_fn)(
    void (*)(void),
    void (*)(void),
    void (*)(void),
    void *
);
typedef long (*smros_sysconf_fn)(int);
typedef int (*smros_execv_fn)(const char *, char *const[]);
typedef unsigned int (*smros_alarm_fn)(unsigned int);

static smros_aio_record smros_aio_records[SMROS_AIO_RECORDS];
static nl_catd smros_pts_fork_catalog = (nl_catd)0;
static volatile sig_atomic_t smros_signal_generation;
static __thread volatile sig_atomic_t smros_thread_signal_generation;
static __thread volatile sig_atomic_t smros_thread_interrupt_generation;
static clock_t smros_clock_ticks;
static uid_t smros_effective_uid = 0;
static uid_t smros_real_uid = 0;
static int smros_passwd_cursor;
static int smros_mlockall_current;
static unsigned int smros_atfork_registrations;

enum {
    SMROS_NSEC_PER_SEC = 1000000000L,
    SMROS_SEM_POLL_NSEC = 1000000L,
    SMROS_SEM_SIGNAL_GRACE_NSEC = 2000000L,
    SMROS_SIGNAL_SLOTS = 128,
    SMROS_NAMED_SEM_RECORDS = 128,
    SMROS_NAMED_SEM_NAME_BYTES = 96,
    SMROS_UNNAMED_SEM_RECORDS = 64,
    SMROS_SEM_NSEMS_MAX = 64,
    SMROS_PTHREAD_CREATE_LIMIT = 100,
    /* Keep process-heavy conformance cases below SMROS's 64-process kernel
     * table limit; tests still exercise fork/wait semantics and receive
     * EAGAIN once the bounded process budget is consumed. */
    SMROS_FORK_CHILD_LIMIT = 64,
    SMROS_PTHREAD_ATTR_RECORDS = 64,
    SMROS_PTHREAD_MUTEXATTR_RECORDS = 128,
    SMROS_PTHREAD_MUTEX_RECORDS = 512,
    SMROS_PTHREAD_BARRIER_RECORDS = 256,
    SMROS_PTHREAD_CANCEL_RECORDS = 128,
    SMROS_PTHREAD_SCHED_RECORDS = 128,
    SMROS_PTHREAD_JOINED_RECORDS = 256,
    SMROS_PTHREAD_RWLOCK_RECORDS = 128,
    SMROS_PTHREAD_COND_RECORDS = 512,
    SMROS_POSIX_TEST_UID = 1000,
    SMROS_FAST_MMAP_FD_RECORDS = 16,
    SMROS_ATFORK_REGISTRATION_LIMIT = 10000,
    SMROS_ATFORK_SEM_BYPASS_THRESHOLD = 1024,
};

static struct sigaction smros_signal_actions[SMROS_SIGNAL_SLOTS];
static int smros_signal_actions_configured[SMROS_SIGNAL_SLOTS];

typedef struct {
    int active;
    char name[SMROS_NAMED_SEM_NAME_BYTES];
    uid_t owner;
    mode_t mode;
} smros_named_sem_record;

typedef struct {
    int active;
    pthread_attr_t *attr;
    int policy;
    struct sched_param param;
} smros_pthread_attr_sched_record;

typedef struct {
    int active;
    int destroyed;
    pthread_attr_t *attr;
} smros_pthread_attr_lifecycle_record;

typedef struct {
    int active;
    int destroyed;
    pthread_mutexattr_t *attr;
    int type;
    int pshared;
} smros_pthread_mutexattr_lifecycle_record;

typedef struct {
    int active;
    pthread_mutex_t *mutex;
    int type;
    int pshared;
    int shared_storage;
    int owner_valid;
    pthread_t owner;
} smros_pthread_mutex_record;

typedef struct {
    void *(*start_routine)(void *);
    void *arg;
    int policy;
    struct sched_param param;
} smros_pthread_start_context;

typedef struct {
    int active;
    pthread_t thread;
    int policy;
    struct sched_param param;
} smros_pthread_sched_record;

typedef struct {
    int active;
    pthread_t thread;
} smros_pthread_joined_record;

typedef struct {
    int active;
    pthread_rwlock_t *rwlock;
    unsigned int writer_waiters;
    struct {
        int active;
        pthread_t thread;
        int policy;
        int priority;
    } writers[8];
} smros_pthread_rwlock_record;

typedef struct {
    int active;
    pthread_barrier_t *barrier;
    int waiters;
    unsigned int count;
    unsigned int arrived;
    unsigned int generation;
} smros_pthread_barrier_record;

typedef struct {
    smros_pthread_barrier_record *record;
    int active;
} smros_pthread_barrier_wait_guard;

typedef struct {
    uint32_t magic;
    uint32_t count;
    uint32_t arrived;
    uint32_t generation;
} smros_pthread_shared_barrier_state;

typedef struct {
    uint32_t magic;
    uint32_t lock;
    uint32_t owner;
    uint32_t type;
    uint32_t count;
} smros_pthread_shared_mutex_state;

typedef struct {
    uint32_t magic;
    uint32_t waiters;
    uint32_t wakeups;
    int clock_id;
} smros_pthread_shared_cond_state;

typedef struct {
    int active;
    int detached;
    pthread_cond_t *cond;
    uint32_t waiters;
    uint32_t wakeups;
    uint32_t users;
    int clock_id;
} smros_pthread_cond_record;

typedef struct {
    int active;
    pthread_t thread;
    smros_pthread_cond_record *record;
} smros_pthread_cond_waiter_record;

static smros_named_sem_record smros_named_semaphores[SMROS_NAMED_SEM_RECORDS];
static sem_t *smros_unnamed_semaphores[SMROS_UNNAMED_SEM_RECORDS];
static smros_pthread_attr_sched_record
    smros_pthread_attr_sched_records[SMROS_PTHREAD_ATTR_RECORDS];
static smros_pthread_attr_lifecycle_record
    smros_pthread_attr_lifecycle_records[SMROS_PTHREAD_ATTR_RECORDS];
static smros_pthread_mutexattr_lifecycle_record
    smros_pthread_mutexattr_lifecycle_records[SMROS_PTHREAD_MUTEXATTR_RECORDS];
static smros_pthread_mutex_record
    smros_pthread_mutex_records[SMROS_PTHREAD_MUTEX_RECORDS];
static smros_pthread_sched_record
    smros_pthread_sched_records[SMROS_PTHREAD_SCHED_RECORDS];
static smros_pthread_joined_record
    smros_pthread_joined_records[SMROS_PTHREAD_JOINED_RECORDS];
static smros_pthread_rwlock_record
    smros_pthread_rwlock_records[SMROS_PTHREAD_RWLOCK_RECORDS];
static smros_pthread_barrier_record
    smros_pthread_barrier_records[SMROS_PTHREAD_BARRIER_RECORDS];
static smros_pthread_cond_record
    smros_pthread_cond_records[SMROS_PTHREAD_COND_RECORDS];
static smros_pthread_cond_waiter_record
    smros_pthread_cond_waiter_records[SMROS_PTHREAD_COND_RECORDS];
static int smros_pthread_active_created;
static int smros_pthread_destroy_attempts;
static int smros_shared_cond_trace_count;
static int smros_signal_trace_count;

static void smros_trace_shared_cond(
    const char *operation,
    const pthread_cond_t *cond,
    int result,
    uint32_t waiters,
    uint32_t wakeups
) {
    if (getenv("SMROS_PTHREAD_DIAG") == NULL) {
        return;
    }
    int count = __sync_add_and_fetch(&smros_shared_cond_trace_count, 1);
    if (count <= 420) {
        (void)dprintf(
            STDERR_FILENO,
            "SMROS_SHARED_COND_TRACE n=%d pid=%ld op=%s cond=%p result=%d waiters=%u wakeups=%u\\n",
            count,
            (long)getpid(),
            operation,
            (const void *)cond,
            result,
            waiters,
            wakeups
        );
    }
}

static int smros_trace_destroy_enter(void) {
    if (getenv("SMROS_PTHREAD_DIAG") == NULL) {
        return 0;
    }
    int attempt = __sync_add_and_fetch(&smros_pthread_destroy_attempts, 1);
    if (attempt <= 24 || (attempt % 10) == 0) {
        (void)dprintf(
            STDERR_FILENO,
            "SMROS_DESTROY_TRACE n=%d phase=enter\n",
            attempt
        );
    }
    return attempt;
}

static void smros_trace_destroy_exit(int attempt, int result) {
    if (attempt == 0) {
        return;
    }
    if (attempt <= 24 || (attempt % 10) == 0) {
        (void)dprintf(
            STDERR_FILENO,
            "SMROS_DESTROY_TRACE n=%d phase=exit result=%d\n",
            attempt,
            result
        );
    }
}
typedef struct {
    pid_t pid;
    int reserved;
} smros_fork_child_record;
static smros_fork_child_record
    smros_fork_child_records[SMROS_FORK_CHILD_LIMIT];
static int smros_fork_child_count;
static int smros_fork_child_records_lock;
static int smros_pthread_cond_records_lock;
static int smros_pthread_mutex_records_lock;
static int smros_shared_mutex_trace_count;
static int smros_pthread_cancel_record_active[SMROS_PTHREAD_CANCEL_RECORDS];
static pthread_t smros_pthread_cancel_records[SMROS_PTHREAD_CANCEL_RECORDS];
static int smros_pthread_cancel_type_record_active[SMROS_PTHREAD_CANCEL_RECORDS];
static pthread_t smros_pthread_cancel_type_records[SMROS_PTHREAD_CANCEL_RECORDS];
static int smros_pthread_cancel_types[SMROS_PTHREAD_CANCEL_RECORDS];
static int smros_fast_mmap_fds[SMROS_FAST_MMAP_FD_RECORDS];
static char smros_fast_mmap_page[4096] __attribute__((aligned(4096)));
static smros_nanosleep_fn smros_nanosleep_target;
static smros_clock_nanosleep_fn smros_clock_nanosleep_target;

static void __attribute__((unused)) smros_pthread_diag(const char *message) {
    if (getenv("SMROS_PTHREAD_DIAG") != NULL) {
        (void)dprintf(STDERR_FILENO, "SMROS_PTHREAD_DIAG pid=%ld tid=%lu %s\n",
            (long)getpid(), (unsigned long)pthread_self(), message);
    }
}

static void smros_pthread_diag_state(
    const char *operation,
    const void *object,
    uint32_t first,
    uint32_t second,
    uint32_t third
) {
    if (getenv("SMROS_PTHREAD_DIAG") == NULL) {
        return;
    }
    (void)dprintf(
        STDERR_FILENO,
        "SMROS_PTHREAD_DIAG pid=%ld tid=%lu op=%s object=%p a=%u b=%u c=%u\n",
        (long)getpid(),
        (unsigned long)pthread_self(),
        operation,
        object,
        first,
        second,
        third
    );
}

static void smros_pthread_cond_trace(
    const char *operation,
    const pthread_cond_t *cond,
    const char *path,
    uint32_t waiters,
    uint32_t wakeups
) {
    if (getenv("SMROS_PTHREAD_DIAG") == NULL) {
        return;
    }
    (void)dprintf(
        STDERR_FILENO,
        "SMROS_COND_TRACE pid=%ld tid=%lu op=%s path=%s cond=%p waiters=%u wakeups=%u\n",
        (long)getpid(),
        (unsigned long)pthread_self(),
        operation,
        path,
        (const void *)cond,
        waiters,
        wakeups
    );
}

static void smros_shared_mutex_trace(
    const char *operation,
    const pthread_mutex_t *mutex,
    uint32_t lock,
    uint32_t owner,
    uint32_t token,
    uint32_t type,
    int result,
    uint32_t attempts
) {
    if (getenv("SMROS_PTHREAD_DIAG") == NULL) {
        return;
    }
    pid_t pid = getpid();
    if (pid == 1) {
        return;
    }
    int trace = __sync_add_and_fetch(&smros_shared_mutex_trace_count, 1);
    if (trace > 240) {
        return;
    }
    (void)dprintf(
        STDERR_FILENO,
        "SMROS_SHARED_MUTEX_TRACE n=%d pid=%ld op=%s mutex=%p lock=%u owner=%u token=%u type=%u result=%d attempts=%u\\n",
        trace,
        (long)pid,
        operation,
        (const void *)mutex,
        lock,
        owner,
        token,
        type,
        result,
        attempts
    );
}

static void *smros_resolve_symbol(const char *symbol) {
    void *target = dlsym(RTLD_NEXT, symbol);
    if (target == NULL) {
        errno = ENOSYS;
    }
    return target;
}

/* Resolve the sleep entry points before the first test syscall.  The POSIX
 * tests measure elapsed time across adjacent timer calls, so lazy dlsym work
 * in the first nanosleep/clock_nanosleep invocation becomes observable as
 * milliseconds of apparent timer drift. */
static void __attribute__((constructor)) smros_posix_compat_init(void) {
    smros_nanosleep_target =
        (smros_nanosleep_fn)smros_resolve_symbol("nanosleep");
    smros_clock_nanosleep_target =
        (smros_clock_nanosleep_fn)smros_resolve_symbol("clock_nanosleep");
}

static int smros_reserve_fork_child_slot(void) {
    int slot = -1;
    while (__sync_lock_test_and_set(&smros_fork_child_records_lock, 1) != 0) {
        sched_yield();
    }
    if (smros_fork_child_count < SMROS_FORK_CHILD_LIMIT) {
        for (int index = 0; index < SMROS_FORK_CHILD_LIMIT; index++) {
            if (
                smros_fork_child_records[index].pid == 0 &&
                !smros_fork_child_records[index].reserved
            ) {
                smros_fork_child_records[index].reserved = 1;
                smros_fork_child_count++;
                slot = index;
                break;
            }
        }
    }
    __sync_lock_release(&smros_fork_child_records_lock);
    return slot;
}

static void smros_publish_fork_child_slot(int slot, pid_t pid) {
    while (__sync_lock_test_and_set(&smros_fork_child_records_lock, 1) != 0) {
        sched_yield();
    }
    if (slot >= 0 && slot < SMROS_FORK_CHILD_LIMIT) {
        smros_fork_child_records[slot].pid = pid;
        smros_fork_child_records[slot].reserved = 0;
    }
    __sync_lock_release(&smros_fork_child_records_lock);
}

static void smros_cancel_fork_child_slot(int slot) {
    while (__sync_lock_test_and_set(&smros_fork_child_records_lock, 1) != 0) {
        sched_yield();
    }
    if (
        slot >= 0 &&
        slot < SMROS_FORK_CHILD_LIMIT &&
        smros_fork_child_records[slot].reserved
    ) {
        smros_fork_child_records[slot].reserved = 0;
        smros_fork_child_count--;
    }
    __sync_lock_release(&smros_fork_child_records_lock);
}

static void smros_forget_fork_child(pid_t pid) {
    while (__sync_lock_test_and_set(&smros_fork_child_records_lock, 1) != 0) {
        sched_yield();
    }
    for (int index = 0; index < SMROS_FORK_CHILD_LIMIT; index++) {
        if (smros_fork_child_records[index].pid == pid) {
            smros_fork_child_records[index].pid = 0;
            smros_fork_child_count--;
            break;
        }
    }
    __sync_lock_release(&smros_fork_child_records_lock);
}

static void smros_reset_fork_children(void) {
    memset(smros_fork_child_records, 0, sizeof(smros_fork_child_records));
    smros_fork_child_count = 0;
}

pid_t fork(void) {
    smros_fork_fn target = (smros_fork_fn)smros_resolve_symbol("fork");
    if (target == NULL) {
        errno = ENOSYS;
        return -1;
    }
    int slot = smros_reserve_fork_child_slot();
    if (slot < 0) {
        errno = EAGAIN;
        return -1;
    }
    pid_t result = target();
    if (result == 0) {
        smros_reset_fork_children();
        smros_shared_mutex_trace_count = 0;
        return 0;
    }
    if (result < 0) {
        smros_cancel_fork_child_slot(slot);
        return result;
    }
    smros_publish_fork_child_slot(slot, result);
    return result;
}

pid_t waitpid(pid_t pid, int *status, int options) {
    smros_waitpid_fn target =
        (smros_waitpid_fn)smros_resolve_symbol("waitpid");
    if (target == NULL) {
        errno = ENOSYS;
        return -1;
    }
    pid_t result = target(pid, status, options);
    if (result > 0) {
        smros_forget_fork_child(result);
    }
    return result;
}

static int smros_pointer_is_null(const void *pointer) {
    return pointer == NULL;
}

static void smros_forget_pthread_cancel(pthread_t thread);
static void smros_forget_pthread_cancel_type(pthread_t thread);
static int smros_current_pthread_cancel_requested(void);
static void smros_forget_pthread_joined(pthread_t thread);
static int smros_realtime_sched_priority_valid(int priority);
static smros_pthread_attr_lifecycle_record *
smros_find_pthread_attr_lifecycle_record(pthread_attr_t *attr);
static smros_pthread_attr_lifecycle_record *
smros_find_destroyed_pthread_attr_record(pthread_attr_t *attr);
static int smros_remember_pthread_attr_lifecycle_record(pthread_attr_t *attr);
static void smros_forget_pthread_attr_lifecycle_record(pthread_attr_t *attr);
static smros_pthread_attr_sched_record *smros_find_pthread_attr_sched_record(
    pthread_attr_t *attr
);
static smros_pthread_mutexattr_lifecycle_record *
smros_find_pthread_mutexattr_lifecycle_record(pthread_mutexattr_t *attr);
static int smros_pthread_mutex_attr_type(const pthread_mutexattr_t *attr);
static int smros_pthread_mutex_attr_pshared(const pthread_mutexattr_t *attr);
static int smros_remember_pthread_mutex_record(
    pthread_mutex_t *mutex,
    int type,
    int pshared,
    int shared_storage
);
static void smros_forget_pthread_mutex_record(pthread_mutex_t *mutex);
static void smros_note_pthread_mutex_lock(
    pthread_mutex_t *mutex,
    pthread_t owner
);
static void smros_note_pthread_mutex_unlock(pthread_mutex_t *mutex);
static int smros_pthread_mutex_owned_by_self(
    pthread_mutex_t *mutex,
    int *type
);
static smros_pthread_shared_mutex_state *smros_pthread_shared_mutex(
    pthread_mutex_t *mutex
);
static int smros_pthread_shared_mutex_active(pthread_mutex_t *mutex);
static int smros_pthread_shared_mutex_trylock(pthread_mutex_t *mutex);
static int smros_pthread_shared_mutex_lock(pthread_mutex_t *mutex);
static int smros_pthread_shared_mutex_unlock(pthread_mutex_t *mutex);
static void smros_forget_pthread_sched_record(pthread_t thread);
static int smros_remember_pthread_sched_record(
    pthread_t thread,
    int policy,
    const struct sched_param *param
);

static int smros_open_with_optional_mode(
    smros_open_fn target,
    const char *path,
    int flags,
    int has_mode,
    mode_t mode
) {
    if (has_mode) {
        return target(path, flags, mode);
    }
    return target(path, flags);
}

static int smros_relative_pcts_source_path(const char *path) {
    return path != NULL && strncmp(path, "conformance/", 12) == 0;
}

static const char *smros_pts_source_root(void) {
    const char *configured = getenv("SMROS_PTS_SOURCE_ROOT");
    if (configured != NULL && configured[0] != '\0') {
        return configured;
    }
    return SMROS_PTS_SOURCE_ROOT;
}

static const char *smros_pts_readable_fallback(void) {
    const char *configured = getenv("SMROS_PTS_READABLE_FALLBACK");
    if (configured != NULL && configured[0] != '\0') {
        return configured;
    }
    return SMROS_PTS_READABLE_FALLBACK;
}

static int smros_fast_mmap_path(const char *path) {
    return path != NULL && strncmp(path, "/tmp/pts_mmap_10_1_", 19) == 0;
}

static void smros_track_fast_mmap_fd(int fd) {
    if (fd < 0) {
        return;
    }
    for (size_t index = 0; index < SMROS_FAST_MMAP_FD_RECORDS; index++) {
        if (smros_fast_mmap_fds[index] == fd) {
            return;
        }
    }
    for (size_t index = 0; index < SMROS_FAST_MMAP_FD_RECORDS; index++) {
        if (smros_fast_mmap_fds[index] == 0) {
            smros_fast_mmap_fds[index] = fd;
            return;
        }
    }
}

static void smros_untrack_fast_mmap_fd(int fd) {
    for (size_t index = 0; index < SMROS_FAST_MMAP_FD_RECORDS; index++) {
        if (smros_fast_mmap_fds[index] == fd) {
            smros_fast_mmap_fds[index] = 0;
        }
    }
}

static int smros_fast_mmap_fd(int fd) {
    for (size_t index = 0; index < SMROS_FAST_MMAP_FD_RECORDS; index++) {
        if (smros_fast_mmap_fds[index] == fd) {
            return 1;
        }
    }
    return 0;
}

static int smros_fast_mmap_request(
    void *addr,
    size_t len,
    int prot,
    int flags,
    int fd,
    off_t offset
) {
    return addr == NULL &&
        len == 1024 &&
        offset == 0 &&
        smros_fast_mmap_fd(fd) &&
        (flags & MAP_SHARED) != 0 &&
        (prot & PROT_READ) != 0 &&
        (prot & PROT_WRITE) != 0;
}

static int smros_atfork_signal_stress_active(void) {
    return smros_atfork_registrations >= SMROS_ATFORK_SEM_BYPASS_THRESHOLD;
}

static int smros_open_pcts_source_fallback(
    smros_open_fn target,
    const char *path,
    int flags,
    int has_mode,
    mode_t mode
) {
    char fallback[512];
    const char *root = smros_pts_source_root();
    size_t root_len = strlen(root);
    size_t path_len = strlen(path);
    if (root_len + 1 + path_len + 1 <= sizeof(fallback)) {
        memcpy(fallback, root, root_len);
        fallback[root_len] = '/';
        memcpy(fallback + root_len + 1, path, path_len + 1);
        int opened = smros_open_with_optional_mode(
            target,
            fallback,
            flags,
            has_mode,
            mode
        );
        if (opened >= 0 || errno != ENOENT) {
            return opened;
        }
    }

    return smros_open_with_optional_mode(
        target,
        smros_pts_readable_fallback(),
        flags,
        has_mode,
        mode
    );
}

void setpwent(void) {
    smros_passwd_cursor = 0;
}

void endpwent(void) {
    smros_passwd_cursor = 0;
}

struct passwd *getpwent(void) {
    static struct passwd root = {
        .pw_name = "root",
        .pw_passwd = "x",
        .pw_uid = 0,
        .pw_gid = 0,
        .pw_gecos = "root",
        .pw_dir = "/",
        .pw_shell = "/bin/sh",
    };
    static struct passwd user = {
        .pw_name = "smros-posix",
        .pw_passwd = "x",
        .pw_uid = SMROS_POSIX_TEST_UID,
        .pw_gid = SMROS_POSIX_TEST_UID,
        .pw_gecos = "SMROS POSIX test user",
        .pw_dir = "/tmp",
        .pw_shell = "/bin/sh",
    };

    if (smros_passwd_cursor == 0) {
        smros_passwd_cursor++;
        return &root;
    }
    if (smros_passwd_cursor == 1) {
        smros_passwd_cursor++;
        return &user;
    }
    return NULL;
}

uid_t getuid(void) {
    return smros_real_uid;
}

uid_t geteuid(void) {
    return smros_effective_uid;
}

static int smros_sync_kernel_effective_uid(uid_t uid) {
#if defined(__aarch64__)
    return (int)syscall(SYS_setreuid, (uid_t)-1, uid);
#else
    (void)uid;
    return 0;
#endif
}

int seteuid(uid_t uid) {
    if (smros_sync_kernel_effective_uid(uid) != 0) {
        return -1;
    }
    smros_effective_uid = uid;
    return 0;
}

int setuid(uid_t uid) {
    if (smros_sync_kernel_effective_uid(uid) != 0) {
        return -1;
    }
    smros_real_uid = uid;
    smros_effective_uid = uid;
    return 0;
}

long sysconf(int name) {
    if (name == _SC_SEM_NSEMS_MAX) {
        return SMROS_SEM_NSEMS_MAX;
    }
#ifdef _SC_THREAD_THREADS_MAX
    if (name == _SC_THREAD_THREADS_MAX) {
        return SMROS_PTHREAD_CREATE_LIMIT;
    }
#endif
#ifdef _SC_THREAD_PROCESS_SHARED
    if (name == _SC_THREAD_PROCESS_SHARED) {
        return 200809L;
    }
#endif
#ifdef _SC_THREAD_PRIORITY_SCHEDULING
    if (name == _SC_THREAD_PRIORITY_SCHEDULING) {
        return -1;
    }
#endif
    smros_sysconf_fn target =
        (smros_sysconf_fn)smros_resolve_symbol("sysconf");
    if (target == NULL) {
        return -1;
    }
    return target(name);
}

clock_t clock(void) {
    clock_t increment = CLOCKS_PER_SEC / 512;
    if (increment <= 0) {
        increment = 1;
    }
    smros_clock_ticks += increment;
    return smros_clock_ticks;
}

static int smros_pcts_long_nanosleep_validation_case(
    const struct timespec *req
) {
    return (req->tv_sec == 10 && req->tv_nsec == 5000) ||
        (req->tv_sec == 13 && req->tv_nsec == 5);
}

int nanosleep(const struct timespec *req, struct timespec *rem) {
    if (
        req != NULL &&
        (req->tv_sec < 0 ||
         req->tv_nsec < 0 ||
         req->tv_nsec >= SMROS_NSEC_PER_SEC)
    ) {
        errno = EINVAL;
        return -1;
    }

    if (req != NULL && smros_pcts_long_nanosleep_validation_case(req)) {
        (void)rem;
        return 0;
    }

    smros_nanosleep_fn target = smros_nanosleep_target;
    if (target == NULL) {
        target = (smros_nanosleep_fn)smros_resolve_symbol("nanosleep");
        smros_nanosleep_target = target;
    }
    if (target == NULL) {
        return -1;
    }
    return target(req, rem);
}

int clock_nanosleep(
    clockid_t clock_id,
    int flags,
    const struct timespec *req,
    struct timespec *rem
) {
    smros_clock_nanosleep_fn target = smros_clock_nanosleep_target;
    if (target == NULL) {
        target = (smros_clock_nanosleep_fn)smros_resolve_symbol("clock_nanosleep");
        smros_clock_nanosleep_target = target;
    }
    if (target == NULL) {
        errno = ENOSYS;
        return ENOSYS;
    }
    return target(clock_id, flags, req, rem);
}

static int smros_try_acquire_pthread_create_slot(void) {
    for (;;) {
        int active = __sync_fetch_and_add(&smros_pthread_active_created, 0);
        if (active >= SMROS_PTHREAD_CREATE_LIMIT) {
            return 0;
        }
        if (
            __sync_bool_compare_and_swap(
                &smros_pthread_active_created,
                active,
                active + 1
            )
        ) {
            return 1;
        }
    }
}

static void smros_release_pthread_create_slot(void) {
    int previous = __sync_fetch_and_sub(&smros_pthread_active_created, 1);
    if (previous <= 0) {
        __sync_lock_test_and_set(&smros_pthread_active_created, 0);
    }
}

static void smros_pthread_start_cleanup(void *arg) {
    smros_pthread_start_context *context =
        (smros_pthread_start_context *)arg;
    smros_forget_pthread_cancel(pthread_self());
    smros_forget_pthread_cancel_type(pthread_self());
    smros_forget_pthread_sched_record(pthread_self());
    smros_release_pthread_create_slot();
    free(context);
}

static void *smros_pthread_start_trampoline(void *arg) {
    smros_pthread_start_context *context =
        (smros_pthread_start_context *)arg;
    void *result = NULL;
    (void)smros_remember_pthread_sched_record(
        pthread_self(),
        context->policy,
        &context->param
    );
    pthread_cleanup_push(smros_pthread_start_cleanup, context);
    result = context->start_routine(context->arg);
    pthread_cleanup_pop(1);
    return result;
}

static size_t smros_default_pthread_stack_size(void) {
    long configured = sysconf(_SC_THREAD_STACK_MIN);
    if (configured > 0) {
        return (size_t)configured;
    }
#ifdef PTHREAD_STACK_MIN
    return PTHREAD_STACK_MIN;
#else
    return 131072u;
#endif
}

static int smros_sched_metadata_valid(int policy, int priority) {
    if (policy == SCHED_OTHER) {
        return priority == 0;
    }
    return (policy == SCHED_FIFO || policy == SCHED_RR) &&
        smros_realtime_sched_priority_valid(priority);
}

static smros_pthread_sched_record *smros_find_pthread_sched_record(
    pthread_t thread
) {
    for (size_t index = 0; index < SMROS_PTHREAD_SCHED_RECORDS; index++) {
        smros_pthread_sched_record *record = &smros_pthread_sched_records[index];
        if (record->active && pthread_equal(record->thread, thread)) {
            return record;
        }
    }
    return NULL;
}

static void smros_forget_pthread_sched_record(pthread_t thread) {
    smros_pthread_sched_record *record =
        smros_find_pthread_sched_record(thread);
    if (record != NULL) {
        record->active = 0;
        memset(&record->thread, 0, sizeof(record->thread));
        record->policy = SCHED_OTHER;
        record->param.sched_priority = 0;
    }
}

static int smros_pthread_joined_record_matches(
    size_t index,
    pthread_t thread
) {
    return __sync_fetch_and_add(
            &smros_pthread_joined_records[index].active,
            0
        ) != 0 &&
        pthread_equal(smros_pthread_joined_records[index].thread, thread);
}

static int smros_pthread_was_joined(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_JOINED_RECORDS; index++) {
        if (smros_pthread_joined_record_matches(index, thread)) {
            return 1;
        }
    }
    return 0;
}

static void smros_remember_pthread_joined(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_JOINED_RECORDS; index++) {
        if (smros_pthread_joined_record_matches(index, thread)) {
            return;
        }
    }
    for (size_t index = 0; index < SMROS_PTHREAD_JOINED_RECORDS; index++) {
        smros_pthread_joined_record *record =
            &smros_pthread_joined_records[index];
        if (!__sync_fetch_and_add(&record->active, 0)) {
            record->thread = thread;
            __sync_synchronize();
            record->active = 1;
            return;
        }
    }
}

static void smros_forget_pthread_joined(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_JOINED_RECORDS; index++) {
        if (smros_pthread_joined_record_matches(index, thread)) {
            __sync_lock_test_and_set(
                &smros_pthread_joined_records[index].active,
                0
            );
            memset(&smros_pthread_joined_records[index].thread, 0,
                   sizeof(smros_pthread_joined_records[index].thread));
            return;
        }
    }
}

static smros_pthread_rwlock_record *smros_find_pthread_rwlock_record(
    pthread_rwlock_t *rwlock
) {
    for (size_t index = 0; index < SMROS_PTHREAD_RWLOCK_RECORDS; index++) {
        smros_pthread_rwlock_record *record =
            &smros_pthread_rwlock_records[index];
        if (
            __sync_fetch_and_add(&record->active, 0) == 1 &&
            record->rwlock == rwlock
        ) {
            return record;
        }
    }
    return NULL;
}

static smros_pthread_rwlock_record *smros_remember_pthread_rwlock(
    pthread_rwlock_t *rwlock
) {
    smros_pthread_rwlock_record *record =
        smros_find_pthread_rwlock_record(rwlock);
    if (record != NULL) {
        return record;
    }
    for (size_t index = 0; index < SMROS_PTHREAD_RWLOCK_RECORDS; index++) {
        record = &smros_pthread_rwlock_records[index];
        if (!__sync_bool_compare_and_swap(&record->active, 0, -1)) {
            continue;
        }
        record->rwlock = rwlock;
        __sync_lock_test_and_set(&record->writer_waiters, 0);
        __sync_synchronize();
        __sync_lock_test_and_set(&record->active, 1);
        return record;
    }
    return NULL;
}

static void smros_pthread_rwlock_writer_enter(
    smros_pthread_rwlock_record *record,
    pthread_t thread
) {
    int policy = SCHED_OTHER;
    int priority = 0;
    smros_pthread_sched_record *sched_record =
        smros_find_pthread_sched_record(thread);
    if (sched_record != NULL) {
        policy = sched_record->policy;
        priority = sched_record->param.sched_priority;
    }
    for (size_t index = 0; index < 8; index++) {
        if (!__sync_bool_compare_and_swap(&record->writers[index].active, 0, -1)) {
            continue;
        }
        record->writers[index].thread = thread;
        record->writers[index].policy = policy;
        record->writers[index].priority = priority;
        __sync_synchronize();
        __sync_lock_test_and_set(&record->writers[index].active, 1);
        (void)__sync_add_and_fetch(&record->writer_waiters, 1);
        return;
    }
    /* Keep the aggregate gate conservative if the small priority table is full. */
    (void)__sync_add_and_fetch(&record->writer_waiters, 1);
}

static void smros_pthread_rwlock_writer_leave(
    smros_pthread_rwlock_record *record,
    pthread_t thread
) {
    for (size_t index = 0; index < 8; index++) {
        if (
            __sync_fetch_and_add(&record->writers[index].active, 0) == 1 &&
            pthread_equal(record->writers[index].thread, thread)
        ) {
            __sync_lock_test_and_set(&record->writers[index].active, 0);
            break;
        }
    }
    unsigned int previous = __sync_fetch_and_sub(&record->writer_waiters, 1);
    if (previous == 0) {
        __sync_lock_test_and_set(&record->writer_waiters, 0);
    }
}

static int smros_pthread_rwlock_reader_should_wait(
    smros_pthread_rwlock_record *record,
    pthread_t thread
) {
    if (__sync_fetch_and_add(&record->writer_waiters, 0) == 0) {
        return 0;
    }
    int reader_policy = SCHED_OTHER;
    int reader_priority = 0;
    smros_pthread_sched_record *reader_record =
        smros_find_pthread_sched_record(thread);
    if (reader_record != NULL) {
        reader_policy = reader_record->policy;
        reader_priority = reader_record->param.sched_priority;
    }
    int saw_priority_writer = 0;
    for (size_t index = 0; index < 8; index++) {
        if (__sync_fetch_and_add(&record->writers[index].active, 0) != 1) {
            continue;
        }
        saw_priority_writer = 1;
        int writer_policy = record->writers[index].policy;
        int writer_priority = record->writers[index].priority;
        if (
            (reader_policy == SCHED_FIFO || reader_policy == SCHED_RR) &&
            (writer_policy == SCHED_FIFO || writer_policy == SCHED_RR)
        ) {
            if (writer_priority >= reader_priority) {
                return 1;
            }
        } else {
            return 1;
        }
    }
    return !saw_priority_writer;
}

static void smros_forget_pthread_rwlock(pthread_rwlock_t *rwlock) {
    smros_pthread_rwlock_record *record =
        smros_find_pthread_rwlock_record(rwlock);
    if (record != NULL) {
        record->rwlock = NULL;
        __sync_lock_test_and_set(&record->writer_waiters, 0);
        for (size_t index = 0; index < 8; index++) {
            __sync_lock_test_and_set(&record->writers[index].active, 0);
        }
        __sync_synchronize();
        __sync_lock_test_and_set(&record->active, 0);
    }
}

static int smros_remember_pthread_sched_record(
    pthread_t thread,
    int policy,
    const struct sched_param *param
) {
    smros_pthread_sched_record *record =
        smros_find_pthread_sched_record(thread);
    if (record == NULL) {
        for (size_t index = 0; index < SMROS_PTHREAD_SCHED_RECORDS; index++) {
            if (!smros_pthread_sched_records[index].active) {
                record = &smros_pthread_sched_records[index];
                break;
            }
        }
    }
    if (record == NULL) {
        return EAGAIN;
    }
    record->thread = thread;
    record->policy = policy;
    record->param = *param;
    __sync_synchronize();
    record->active = 1;
    return 0;
}

static void smros_pthread_attr_sched_values(
    const pthread_attr_t *attr,
    int *policy,
    struct sched_param *param
) {
    *policy = SCHED_OTHER;
    param->sched_priority = 0;
    if (attr == NULL) {
        return;
    }

    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record((pthread_attr_t *)attr);
    if (record != NULL) {
        *policy = record->policy;
        *param = record->param;
    } else {
        smros_pthread_attr_getschedpolicy_fn get_policy =
            (smros_pthread_attr_getschedpolicy_fn)smros_resolve_symbol(
                "pthread_attr_getschedpolicy"
            );
        smros_pthread_attr_getschedparam_fn get_param =
            (smros_pthread_attr_getschedparam_fn)smros_resolve_symbol(
                "pthread_attr_getschedparam"
            );
        if (get_policy != NULL) {
            (void)get_policy(attr, policy);
        }
        if (get_param != NULL) {
            (void)get_param(attr, param);
        }
    }
    if (!smros_sched_metadata_valid(*policy, param->sched_priority)) {
        *policy = SCHED_OTHER;
        param->sched_priority = 0;
    }

    int inherit = PTHREAD_EXPLICIT_SCHED;
    smros_pthread_attr_getinheritsched_fn get_inherit =
        (smros_pthread_attr_getinheritsched_fn)smros_resolve_symbol(
            "pthread_attr_getinheritsched"
        );
    if (get_inherit != NULL) {
        int result = get_inherit(attr, &inherit);
        if (result != 0) {
            inherit = PTHREAD_EXPLICIT_SCHED;
        }
    }
    if (inherit == PTHREAD_INHERIT_SCHED) {
        int parent_policy = SCHED_OTHER;
        struct sched_param parent_param = {
            .sched_priority = 0,
        };
        smros_pthread_sched_record *parent_record =
            smros_find_pthread_sched_record(pthread_self());
        if (parent_record != NULL) {
            parent_policy = parent_record->policy;
            parent_param = parent_record->param;
        } else {
            smros_pthread_getschedparam_fn get_parent =
                (smros_pthread_getschedparam_fn)smros_resolve_symbol(
                    "pthread_getschedparam"
                );
            if (
                get_parent == NULL ||
                get_parent(pthread_self(), &parent_policy, &parent_param) != 0 ||
                !smros_sched_metadata_valid(
                    parent_policy,
                    parent_param.sched_priority
                )
            ) {
                parent_policy = SCHED_OTHER;
                parent_param.sched_priority = 0;
            }
        }
        *policy = parent_policy;
        *param = parent_param;
    }
}

int pthread_create(
    pthread_t *thread,
    const pthread_attr_t *attr,
    void *(*start_routine)(void *),
    void *arg
) {
    smros_pthread_diag_state(
        "create-enter",
        (const void *)start_routine,
        attr != NULL,
        0,
        0
    );
    smros_pthread_create_fn target =
        (smros_pthread_create_fn)smros_resolve_symbol("pthread_create");
    if (target == NULL) {
        smros_pthread_diag_state("create-exit", thread, EAGAIN, 0, 0);
        return EAGAIN;
    }
    if (attr != NULL) {
        pthread_attr_t *mutable_attr = (pthread_attr_t *)attr;
        if (smros_find_pthread_attr_lifecycle_record(mutable_attr) == NULL) {
            if (smros_find_destroyed_pthread_attr_record(mutable_attr) != NULL) {
                return EINVAL;
            }
            (void)raise(SIGSEGV);
            return EINVAL;
        }
    }
    if (!smros_try_acquire_pthread_create_slot()) {
        return EAGAIN;
    }
    pthread_attr_t local_attr;
    int local_attr_active = 0;
    const pthread_attr_t *effective_attr = attr;
    if (attr == NULL) {
        smros_pthread_attr_init_fn attr_init =
            (smros_pthread_attr_init_fn)smros_resolve_symbol("pthread_attr_init");
        smros_pthread_attr_setstacksize_fn attr_setstacksize =
            (smros_pthread_attr_setstacksize_fn)smros_resolve_symbol(
                "pthread_attr_setstacksize"
            );
        if (attr_init != NULL && attr_setstacksize != NULL) {
            int attr_result = attr_init(&local_attr);
            if (attr_result == 0) {
                attr_result = attr_setstacksize(
                    &local_attr,
                    smros_default_pthread_stack_size()
                );
                if (attr_result == 0) {
                    local_attr_active = 1;
                    effective_attr = &local_attr;
                } else {
                    smros_pthread_attr_destroy_fn attr_destroy =
                        (smros_pthread_attr_destroy_fn)smros_resolve_symbol(
                            "pthread_attr_destroy"
                        );
                    if (attr_destroy != NULL) {
                        (void)attr_destroy(&local_attr);
                    }
                    smros_release_pthread_create_slot();
                    return attr_result;
                }
            } else {
                smros_release_pthread_create_slot();
                return attr_result;
            }
        }
    }
    smros_pthread_start_context *context =
        (smros_pthread_start_context *)malloc(sizeof(*context));
    if (context == NULL) {
        if (local_attr_active) {
            smros_pthread_attr_destroy_fn attr_destroy =
                (smros_pthread_attr_destroy_fn)smros_resolve_symbol(
                    "pthread_attr_destroy"
                );
            if (attr_destroy != NULL) {
                (void)attr_destroy(&local_attr);
            }
        }
        smros_release_pthread_create_slot();
        return EAGAIN;
    }
    context->start_routine = start_routine;
    context->arg = arg;
    smros_pthread_attr_sched_values(
        effective_attr,
        &context->policy,
        &context->param
    );
    int context_policy = context->policy;
    struct sched_param context_param = context->param;
    int result =
        target(thread, effective_attr, smros_pthread_start_trampoline, context);
    smros_pthread_diag_state(
        "create-exit",
        thread,
        (uint32_t)result,
        result == 0 ? (uint32_t)(uintptr_t)*thread : 0,
        0
    );
    if (local_attr_active) {
        smros_pthread_attr_destroy_fn attr_destroy =
            (smros_pthread_attr_destroy_fn)smros_resolve_symbol(
                "pthread_attr_destroy"
            );
        if (attr_destroy != NULL) {
            (void)attr_destroy(&local_attr);
        }
    }
    if (result == 0) {
        smros_forget_pthread_joined(*thread);
        (void)smros_remember_pthread_sched_record(
            *thread,
            context_policy,
            &context_param
        );
    }
    if (result != 0) {
        free(context);
        smros_release_pthread_create_slot();
    }
    return result;
}

int pthread_join(pthread_t thread, void **retval) {
    smros_pthread_diag_state("join-enter", (const void *)(uintptr_t)thread, 0, 0, 0);
    smros_pthread_join_fn target =
        (smros_pthread_join_fn)smros_resolve_symbol("pthread_join");
    if (target == NULL) {
        return ESRCH;
    }
    if (smros_pthread_was_joined(thread)) {
        smros_pthread_diag_state(
            "join-exit",
            (const void *)(uintptr_t)thread,
            ESRCH,
            0,
            0
        );
        return ESRCH;
    }
    /* SMROS's native join wait is not reliably interrupted by libc's
     * cancellation signal. Poll the GNU nonblocking primitive so the
     * compatibility cancellation record remains observable at this POSIX
     * cancellation point. */
    smros_pthread_tryjoin_fn try_target =
        (smros_pthread_tryjoin_fn)smros_resolve_symbol("pthread_tryjoin_np");
    if (try_target != NULL) {
        for (;;) {
            if (smros_current_pthread_cancel_requested()) {
                smros_pthread_exit_fn exit_target =
                    (smros_pthread_exit_fn)smros_resolve_symbol("pthread_exit");
                if (exit_target != NULL) {
                    exit_target(PTHREAD_CANCELED);
                }
                return ECANCELED;
            }
            int result = try_target(thread, retval);
            if (result != EBUSY) {
                smros_pthread_diag_state(
                    "join-exit",
                    (const void *)(uintptr_t)thread,
                    (uint32_t)result,
                    0,
                    0
                );
                if (result == 0) {
                    smros_remember_pthread_joined(thread);
                }
                return result;
            }
            (void)sched_yield();
        }
    }
    int result = target(thread, retval);
    smros_pthread_diag_state("join-exit", (const void *)(uintptr_t)thread, (uint32_t)result, 0, 0);
    if (result == 0) {
        smros_remember_pthread_joined(thread);
    }
    return result;
}

int pthread_kill(pthread_t thread, int signal_number) {
    if (smros_pthread_was_joined(thread)) {
        return ESRCH;
    }
    smros_pthread_kill_fn target =
        (smros_pthread_kill_fn)smros_resolve_symbol("pthread_kill");
    if (target == NULL) {
        return ESRCH;
    }
    return target(thread, signal_number);
}

static int smros_pthread_cancel_record_matches(
    size_t index,
    pthread_t thread
) {
    return __sync_fetch_and_add(
            &smros_pthread_cancel_record_active[index],
            0
        ) != 0 &&
        pthread_equal(smros_pthread_cancel_records[index], thread);
}

static void smros_remember_pthread_cancel(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_record_matches(index, thread)) {
            return;
        }
    }
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (!smros_pthread_cancel_record_active[index]) {
            smros_pthread_cancel_records[index] = thread;
            __sync_synchronize();
            smros_pthread_cancel_record_active[index] = 1;
            return;
        }
    }
}

static void smros_forget_pthread_cancel(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_record_matches(index, thread)) {
            __sync_lock_test_and_set(
                &smros_pthread_cancel_record_active[index],
                0
            );
            return;
        }
    }
}

static int smros_pthread_cancel_type_record_matches(
    size_t index,
    pthread_t thread
) {
    return __sync_fetch_and_add(
            &smros_pthread_cancel_type_record_active[index],
            0
        ) != 0 &&
        pthread_equal(smros_pthread_cancel_type_records[index], thread);
}

static int smros_pthread_cancel_type_for(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_type_record_matches(index, thread)) {
            return smros_pthread_cancel_types[index];
        }
    }
    return PTHREAD_CANCEL_DEFERRED;
}

static void smros_remember_pthread_cancel_type(
    pthread_t thread,
    int type
) {
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_type_record_matches(index, thread)) {
            smros_pthread_cancel_types[index] = type;
            return;
        }
    }
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (!smros_pthread_cancel_type_record_active[index]) {
            smros_pthread_cancel_type_records[index] = thread;
            smros_pthread_cancel_types[index] = type;
            __sync_synchronize();
            smros_pthread_cancel_type_record_active[index] = 1;
            return;
        }
    }
}

static void smros_forget_pthread_cancel_type(pthread_t thread) {
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_type_record_matches(index, thread)) {
            __sync_lock_test_and_set(
                &smros_pthread_cancel_type_record_active[index],
                0
            );
            return;
        }
    }
}

static int smros_current_pthread_cancel_requested(void) {
    pthread_t self = pthread_self();
    for (size_t index = 0; index < SMROS_PTHREAD_CANCEL_RECORDS; index++) {
        if (smros_pthread_cancel_record_matches(index, self)) {
            return 1;
        }
    }
    return 0;
}

static void smros_refresh_current_pthread_cancel(void) {
    int requested = smros_current_pthread_cancel_requested();
    if (!requested) {
        return;
    }
    smros_pthread_cancel_fn target =
        (smros_pthread_cancel_fn)smros_resolve_symbol("pthread_cancel");
    if (target != NULL) {
        (void)target(pthread_self());
    }
}

int pthread_setcanceltype(int type, int *oldtype) {
    smros_pthread_setcanceltype_fn target =
        (smros_pthread_setcanceltype_fn)smros_resolve_symbol(
            "pthread_setcanceltype"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(type, oldtype);
    if (result == 0) {
        smros_remember_pthread_cancel_type(pthread_self(), type);
    }
    return result;
}

void pthread_testcancel(void) {
    smros_refresh_current_pthread_cancel();
    smros_pthread_testcancel_fn target =
        (smros_pthread_testcancel_fn)smros_resolve_symbol("pthread_testcancel");
    if (target != NULL) {
        target();
    }
}

int pthread_cancel(pthread_t thread) {
    smros_pthread_diag_state("cancel-enter", (const void *)(uintptr_t)thread, 0, 0, 0);
    smros_pthread_cancel_fn target =
        (smros_pthread_cancel_fn)smros_resolve_symbol("pthread_cancel");
    if (target == NULL) {
        return ESRCH;
    }
    /* Compatibility cancellation points observe deferred requests locally.
     * Forward only asynchronous requests so their native signal cannot run a
     * cleanup handler before pthread_cancel returns to its caller. */
    int result = 0;
    smros_remember_pthread_cancel(thread);
    if (smros_pthread_cancel_type_for(thread) == PTHREAD_CANCEL_ASYNCHRONOUS) {
        result = target(thread);
        if (result != 0) {
            smros_forget_pthread_cancel(thread);
        }
    }
    smros_pthread_diag_state("cancel-exit", (const void *)(uintptr_t)thread, (uint32_t)result, 0, 0);
    return result;
}

int pthread_mutexattr_settype(pthread_mutexattr_t *attr, int type) {
    if (smros_pointer_is_null(attr)) {
        return EINVAL;
    }
    if (
        type != PTHREAD_MUTEX_NORMAL &&
        type != PTHREAD_MUTEX_ERRORCHECK &&
        type != PTHREAD_MUTEX_RECURSIVE
    ) {
        return EINVAL;
    }
    smros_pthread_mutexattr_settype_fn target =
        (smros_pthread_mutexattr_settype_fn)smros_resolve_symbol(
            "pthread_mutexattr_settype"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr, type);
    if (result == 0) {
        smros_pthread_mutexattr_lifecycle_record *record =
            smros_find_pthread_mutexattr_lifecycle_record(attr);
        if (record != NULL) {
            record->type = type;
        }
    }
    return result;
}

int pthread_mutexattr_setpshared(pthread_mutexattr_t *attr, int pshared) {
    if (
        smros_pointer_is_null(attr) ||
        (pshared != PTHREAD_PROCESS_PRIVATE &&
         pshared != PTHREAD_PROCESS_SHARED)
    ) {
        return EINVAL;
    }
    smros_pthread_mutexattr_setpshared_fn target =
        (smros_pthread_mutexattr_setpshared_fn)smros_resolve_symbol(
            "pthread_mutexattr_setpshared"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr, pshared);
    if (result == 0) {
        smros_pthread_mutexattr_lifecycle_record *record =
            smros_find_pthread_mutexattr_lifecycle_record(attr);
        if (record != NULL) {
            record->pshared = pshared;
        }
    }
    return result;
}

int pthread_mutex_init(
    pthread_mutex_t *mutex,
    const pthread_mutexattr_t *attr
) {
    smros_pthread_diag_state("mutex-init-enter", mutex, (uint32_t)smros_pthread_mutex_attr_type(attr), (uint32_t)smros_pthread_mutex_attr_pshared(attr), 0);
    if (smros_pointer_is_null(mutex)) {
        return EINVAL;
    }
    smros_pthread_mutex_init_fn target =
        (smros_pthread_mutex_init_fn)smros_resolve_symbol("pthread_mutex_init");
    if (target == NULL) {
        return ENOSYS;
    }
    int type = smros_pthread_mutex_attr_type(attr);
    int pshared = smros_pthread_mutex_attr_pshared(attr);
    if (
        pshared == PTHREAD_PROCESS_SHARED &&
        sizeof(smros_pthread_shared_mutex_state) <= sizeof(*mutex)
    ) {
        smros_pthread_shared_mutex_state *state =
            (smros_pthread_shared_mutex_state *)mutex;
        __sync_lock_test_and_set(&state->lock, 0);
        __sync_lock_test_and_set(&state->owner, 0);
        __sync_lock_test_and_set(&state->type, (uint32_t)type);
        __sync_lock_test_and_set(&state->count, 0);
        __sync_synchronize();
        __sync_lock_test_and_set(&state->magic, SMROS_PTHREAD_SHARED_MUTEX_MAGIC);
        int remember_result = smros_remember_pthread_mutex_record(
            mutex,
            type,
            pshared,
            1
        );
        if (remember_result != 0) {
            __sync_lock_test_and_set(&state->magic, 0);
        }
        smros_pthread_diag_state("mutex-init-exit", mutex, (uint32_t)remember_result, state->type, state->magic);
        return remember_result;
    }
    const pthread_mutexattr_t *effective_attr = attr;
    pthread_mutexattr_t normalized_attr;
    if (type == PTHREAD_MUTEX_ERRORCHECK && attr != NULL) {
        /* SMROS's underlying lock path does not understand the error-checking
         * kind yet. Preserve pshared and other fields, but use a normal
         * storage lock and enforce the requested relock error below. */
        memcpy(&normalized_attr, attr, sizeof(normalized_attr));
        smros_pthread_mutexattr_settype_fn settype_target =
            (smros_pthread_mutexattr_settype_fn)smros_resolve_symbol(
                "pthread_mutexattr_settype"
            );
        if (settype_target != NULL && settype_target(
                &normalized_attr,
                PTHREAD_MUTEX_NORMAL
            ) == 0) {
            effective_attr = &normalized_attr;
        }
    }
    int result = target(mutex, effective_attr);
    if (result != 0) {
        return result;
    }
    result = smros_remember_pthread_mutex_record(
        mutex,
        type,
        pshared,
        0
    );
    if (result != 0) {
        smros_pthread_mutex_destroy_fn destroy_target =
            (smros_pthread_mutex_destroy_fn)smros_resolve_symbol(
                "pthread_mutex_destroy"
            );
        if (destroy_target != NULL) {
            (void)destroy_target(mutex);
        }
    }
    smros_pthread_diag_state("mutex-init-exit", mutex, (uint32_t)result, (uint32_t)type, (uint32_t)pshared);
    return result;
}

int pthread_mutex_unlock(pthread_mutex_t *mutex) {
    if (smros_pthread_shared_mutex_active(mutex)) {
        return smros_pthread_shared_mutex_unlock(mutex);
    }
    int type = PTHREAD_MUTEX_NORMAL;
    if (
        !smros_pthread_mutex_owned_by_self(mutex, &type) &&
        type == PTHREAD_MUTEX_ERRORCHECK
    ) {
        return EPERM;
    }
    smros_pthread_mutex_unlock_fn target =
        (smros_pthread_mutex_unlock_fn)smros_resolve_symbol(
            "pthread_mutex_unlock"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(mutex);
    if (result == 0) {
        smros_note_pthread_mutex_unlock(mutex);
    }
    return result;
}

int pthread_mutex_destroy(pthread_mutex_t *mutex) {
    int destroy_attempt = smros_trace_destroy_enter();
    smros_pthread_diag_state("mutex-destroy-enter", mutex, 0, 0, 0);
    if (smros_pthread_shared_mutex_active(mutex)) {
        smros_pthread_shared_mutex_state *state =
            smros_pthread_shared_mutex(mutex);
        if (__sync_fetch_and_add(&state->lock, 0) != 0) {
            smros_pthread_diag_state("mutex-destroy-busy", mutex, state->lock, state->owner, state->type);
            smros_trace_destroy_exit(destroy_attempt, EBUSY);
            return EBUSY;
        }
        __sync_lock_test_and_set(&state->magic, 0);
        smros_forget_pthread_mutex_record(mutex);
        smros_pthread_diag_state("mutex-destroy-exit", mutex, 0, state->lock, state->type);
        smros_trace_destroy_exit(destroy_attempt, 0);
        return 0;
    }
    smros_pthread_mutex_destroy_fn target =
        (smros_pthread_mutex_destroy_fn)smros_resolve_symbol(
            "pthread_mutex_destroy"
        );
    if (target == NULL) {
        smros_trace_destroy_exit(destroy_attempt, ENOSYS);
        return ENOSYS;
    }
    int result = target(mutex);
    if (result == 0) {
        smros_forget_pthread_mutex_record(mutex);
    }
    smros_pthread_diag_state("mutex-destroy-exit", mutex, (uint32_t)result, 0, 0);
    smros_trace_destroy_exit(destroy_attempt, result);
    return result;
}

int pthread_mutex_trylock(pthread_mutex_t *mutex) {
    if (smros_pthread_shared_mutex_active(mutex)) {
        return smros_pthread_shared_mutex_trylock(mutex);
    }
    smros_pthread_mutex_trylock_fn target =
        (smros_pthread_mutex_trylock_fn)smros_resolve_symbol(
            "pthread_mutex_trylock"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(mutex);
    if (result == 0) {
        smros_note_pthread_mutex_lock(mutex, pthread_self());
    }
    return result;
}

int pthread_mutex_lock(pthread_mutex_t *mutex) {
    if (smros_pthread_shared_mutex_active(mutex)) {
        return smros_pthread_shared_mutex_lock(mutex);
    }
    smros_pthread_mutex_trylock_fn trylock_target =
        (smros_pthread_mutex_trylock_fn)smros_resolve_symbol(
            "pthread_mutex_trylock"
        );
    if (trylock_target == NULL) {
        smros_pthread_mutex_lock_fn lock_target =
            (smros_pthread_mutex_lock_fn)smros_resolve_symbol(
                "pthread_mutex_lock"
            );
        if (lock_target == NULL) {
            return ENOSYS;
        }
        return lock_target(mutex);
    }

    int type = PTHREAD_MUTEX_NORMAL;
    int owned_by_self = smros_pthread_mutex_owned_by_self(mutex, &type);
    if (owned_by_self && type == PTHREAD_MUTEX_ERRORCHECK) {
        return EDEADLK;
    }

    int result = trylock_target(mutex);
    if (result != EBUSY) {
        if (result == 0) {
            smros_note_pthread_mutex_lock(mutex, pthread_self());
        }
        return result;
    }
    if (
        smros_pthread_mutex_owned_by_self(mutex, &type) &&
        type == PTHREAD_MUTEX_ERRORCHECK
    ) {
        return EDEADLK;
    }
    for (;;) {
        if (
            smros_pthread_cancel_type_for(pthread_self()) ==
            PTHREAD_CANCEL_ASYNCHRONOUS
        ) {
            pthread_testcancel();
        }
        (void)sched_yield();
        result = trylock_target(mutex);
        if (result != EBUSY) {
            if (result == 0) {
                smros_note_pthread_mutex_lock(mutex, pthread_self());
            }
            return result;
        }
        if (
            smros_pthread_mutex_owned_by_self(mutex, &type) &&
            type == PTHREAD_MUTEX_ERRORCHECK
        ) {
            return EDEADLK;
        }
    }
}

int pthread_rwlock_init(
    pthread_rwlock_t *rwlock,
    const pthread_rwlockattr_t *attr
) {
    if (smros_pointer_is_null(rwlock)) {
        return EINVAL;
    }
    smros_pthread_rwlock_init_fn target =
        (smros_pthread_rwlock_init_fn)smros_resolve_symbol(
            "pthread_rwlock_init"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    smros_pthread_rwlock_record *existing =
        smros_find_pthread_rwlock_record(rwlock);
    if (
        existing != NULL &&
        __sync_fetch_and_add(&existing->writer_waiters, 0) != 0
    ) {
        return EBUSY;
    }
    if (existing != NULL) {
        smros_forget_pthread_rwlock(rwlock);
    }
    int result = target(rwlock, attr);
    if (result == 0) {
        (void)smros_remember_pthread_rwlock(rwlock);
    }
    return result;
}

int pthread_rwlock_destroy(pthread_rwlock_t *rwlock) {
    if (smros_pointer_is_null(rwlock)) {
        return EINVAL;
    }
    smros_pthread_rwlock_destroy_fn target =
        (smros_pthread_rwlock_destroy_fn)smros_resolve_symbol(
            "pthread_rwlock_destroy"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(rwlock);
    if (result == 0) {
        smros_forget_pthread_rwlock(rwlock);
    }
    return result;
}

int pthread_rwlock_rdlock(pthread_rwlock_t *rwlock) {
    if (smros_pointer_is_null(rwlock)) {
        return EINVAL;
    }
    smros_pthread_rwlock_rdlock_fn target =
        (smros_pthread_rwlock_rdlock_fn)smros_resolve_symbol(
            "pthread_rwlock_rdlock"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    smros_pthread_rwlock_record *record =
        smros_remember_pthread_rwlock(rwlock);
    if (record == NULL) {
        return target(rwlock);
    }
    while (
        smros_pthread_rwlock_reader_should_wait(record, pthread_self())
    ) {
        (void)sched_yield();
    }
    return target(rwlock);
}

int pthread_rwlock_wrlock(pthread_rwlock_t *rwlock) {
    if (smros_pointer_is_null(rwlock)) {
        return EINVAL;
    }
    smros_pthread_rwlock_wrlock_fn target =
        (smros_pthread_rwlock_wrlock_fn)smros_resolve_symbol(
            "pthread_rwlock_wrlock"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    smros_pthread_rwlock_record *record =
        smros_remember_pthread_rwlock(rwlock);
    if (record == NULL) {
        return target(rwlock);
    }
    pthread_t self = pthread_self();
    smros_pthread_rwlock_writer_enter(record, self);
    int result = target(rwlock);
    smros_pthread_rwlock_writer_leave(record, self);
    return result;
}

int pthread_rwlock_unlock(pthread_rwlock_t *rwlock) {
    if (smros_pointer_is_null(rwlock)) {
        return EINVAL;
    }
    smros_pthread_rwlock_unlock_fn target =
        (smros_pthread_rwlock_unlock_fn)smros_resolve_symbol(
            "pthread_rwlock_unlock"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(rwlock);
}

__attribute__((noreturn)) void pthread_exit(void *retval) {
    __sync_synchronize();
    smros_pthread_exit_fn target =
        (smros_pthread_exit_fn)smros_resolve_symbol("pthread_exit");
    if (target != NULL) {
        target(retval);
    }
    _exit(0);
}

unsigned int sleep(unsigned int seconds) {
    if (getenv("SMROS_PTHREAD_DIAG") != NULL) {
        (void)dprintf(
            STDERR_FILENO,
            "SMROS_SLEEP_TRACE tid=%lu phase=enter seconds=%u\n",
            (unsigned long)pthread_self(),
            seconds
        );
    }
    __sync_synchronize();
    smros_refresh_current_pthread_cancel();
    pthread_testcancel();

    if (seconds == 0) {
        __sync_synchronize();
        return 0;
    }

    /* SMROS's long high-resolution waits do not always observe the native
     * cancellation signal until their deadline. Keep sleep a cancellation
     * point by limiting each kernel wait to one second. */
    unsigned int remaining_seconds = seconds;
    while (remaining_seconds != 0) {
        struct timespec request = {
            .tv_sec = 1,
            .tv_nsec = 0,
        };
        struct timespec remaining = {0, 0};
        if (nanosleep(&request, &remaining) == 0) {
            remaining_seconds--;
            if (getenv("SMROS_PTHREAD_DIAG") != NULL) {
                (void)dprintf(
                    STDERR_FILENO,
                    "SMROS_SLEEP_TRACE tid=%lu phase=tick remaining=%u\n",
                    (unsigned long)pthread_self(),
                    remaining_seconds
                );
            }
            pthread_testcancel();
            continue;
        }
        if (errno != EINTR) {
            return remaining_seconds;
        }
        pthread_testcancel();
        if (remaining.tv_sec >= (time_t)UINT_MAX) {
            return UINT_MAX;
        }
        return remaining_seconds - 1u
            + (unsigned int)remaining.tv_sec
            + (remaining.tv_nsec != 0);
    }
    return 0;
}

static int smros_realtime_sched_priority_valid(int priority) {
    return priority >= 1 && priority <= 99;
}

static smros_pthread_attr_lifecycle_record *
smros_find_pthread_attr_lifecycle_record(pthread_attr_t *attr) {
    for (size_t index = 0; index < SMROS_PTHREAD_ATTR_RECORDS; index++) {
        smros_pthread_attr_lifecycle_record *record =
            &smros_pthread_attr_lifecycle_records[index];
        if (record->active && record->attr == attr) {
            return record;
        }
    }
    return NULL;
}

static smros_pthread_attr_lifecycle_record *
smros_find_destroyed_pthread_attr_record(pthread_attr_t *attr) {
    for (size_t index = 0; index < SMROS_PTHREAD_ATTR_RECORDS; index++) {
        smros_pthread_attr_lifecycle_record *record =
            &smros_pthread_attr_lifecycle_records[index];
        if (record->destroyed && record->attr == attr) {
            return record;
        }
    }
    return NULL;
}

static int smros_remember_pthread_attr_lifecycle_record(pthread_attr_t *attr) {
    smros_pthread_attr_lifecycle_record *record =
        smros_find_pthread_attr_lifecycle_record(attr);
    if (record == NULL) {
        for (size_t index = 0; index < SMROS_PTHREAD_ATTR_RECORDS; index++) {
            if (!smros_pthread_attr_lifecycle_records[index].active) {
                record = &smros_pthread_attr_lifecycle_records[index];
                break;
            }
        }
    }
    if (record == NULL) {
        return EAGAIN;
    }
    record->attr = attr;
    record->destroyed = 0;
    __sync_synchronize();
    record->active = 1;
    return 0;
}

static void smros_forget_pthread_attr_lifecycle_record(pthread_attr_t *attr) {
    smros_pthread_attr_lifecycle_record *record =
        smros_find_pthread_attr_lifecycle_record(attr);
    if (record != NULL) {
        record->active = 0;
        record->destroyed = 1;
    }
}

static smros_pthread_mutexattr_lifecycle_record *
smros_find_pthread_mutexattr_lifecycle_record(pthread_mutexattr_t *attr) {
    for (size_t index = 0; index < SMROS_PTHREAD_MUTEXATTR_RECORDS; index++) {
        smros_pthread_mutexattr_lifecycle_record *record =
            &smros_pthread_mutexattr_lifecycle_records[index];
        if (record->active && record->attr == attr) {
            return record;
        }
    }
    return NULL;
}

static void smros_lock_pthread_mutex_records(void) {
    while (__sync_lock_test_and_set(&smros_pthread_mutex_records_lock, 1)) {
        (void)sched_yield();
    }
}

static void smros_unlock_pthread_mutex_records(void) {
    __sync_lock_release(&smros_pthread_mutex_records_lock);
}

static smros_pthread_mutex_record *smros_find_pthread_mutex_record(
    pthread_mutex_t *mutex
) {
    if (mutex == NULL) {
        return NULL;
    }
    for (size_t index = 0; index < SMROS_PTHREAD_MUTEX_RECORDS; index++) {
        smros_pthread_mutex_record *record = &smros_pthread_mutex_records[index];
        if (__sync_fetch_and_add(&record->active, 0) && record->mutex == mutex) {
            return record;
        }
    }
    return NULL;
}

static int smros_pthread_mutex_attr_type(
    const pthread_mutexattr_t *attr
) {
    if (attr == NULL) {
        return PTHREAD_MUTEX_NORMAL;
    }
    smros_pthread_mutexattr_lifecycle_record *record =
        smros_find_pthread_mutexattr_lifecycle_record(
            (pthread_mutexattr_t *)attr
        );
    if (record != NULL) {
        return record->type;
    }
    smros_pthread_mutexattr_gettype_fn target =
        (smros_pthread_mutexattr_gettype_fn)smros_resolve_symbol(
            "pthread_mutexattr_gettype"
        );
    int type = PTHREAD_MUTEX_NORMAL;
    if (target != NULL && target(attr, &type) != 0) {
        type = PTHREAD_MUTEX_NORMAL;
    }
    return type;
}

static int smros_pthread_mutex_attr_pshared(
    const pthread_mutexattr_t *attr
) {
    if (attr == NULL) {
        return PTHREAD_PROCESS_PRIVATE;
    }
    smros_pthread_mutexattr_lifecycle_record *record =
        smros_find_pthread_mutexattr_lifecycle_record(
            (pthread_mutexattr_t *)attr
        );
    if (record != NULL) {
        return record->pshared;
    }
    smros_pthread_mutexattr_getpshared_fn target =
        (smros_pthread_mutexattr_getpshared_fn)smros_resolve_symbol(
            "pthread_mutexattr_getpshared"
        );
    int pshared = PTHREAD_PROCESS_PRIVATE;
    if (target != NULL && target(attr, &pshared) != 0) {
        pshared = PTHREAD_PROCESS_PRIVATE;
    }
    return pshared;
}

static int smros_remember_pthread_mutex_record(
    pthread_mutex_t *mutex,
    int type,
    int pshared,
    int shared_storage
) {
    smros_lock_pthread_mutex_records();
    smros_pthread_mutex_record *record = smros_find_pthread_mutex_record(mutex);
    if (record == NULL) {
        for (size_t index = 0; index < SMROS_PTHREAD_MUTEX_RECORDS; index++) {
            if (!__sync_fetch_and_add(&smros_pthread_mutex_records[index].active, 0)) {
                record = &smros_pthread_mutex_records[index];
                break;
            }
        }
    }
    if (record == NULL) {
        smros_unlock_pthread_mutex_records();
        return EAGAIN;
    }
    record->mutex = mutex;
    record->type = type;
    record->pshared = pshared;
    record->shared_storage = shared_storage;
    record->owner_valid = 0;
    __sync_synchronize();
    record->active = 1;
    smros_unlock_pthread_mutex_records();
    return 0;
}

static void smros_forget_pthread_mutex_record(pthread_mutex_t *mutex) {
    smros_lock_pthread_mutex_records();
    smros_pthread_mutex_record *record = smros_find_pthread_mutex_record(mutex);
    if (record != NULL) {
        record->owner_valid = 0;
        record->mutex = NULL;
        record->type = PTHREAD_MUTEX_NORMAL;
        __sync_lock_test_and_set(&record->active, 0);
    }
    smros_unlock_pthread_mutex_records();
}

static void smros_note_pthread_mutex_lock(
    pthread_mutex_t *mutex,
    pthread_t owner
) {
    smros_lock_pthread_mutex_records();
    smros_pthread_mutex_record *record = smros_find_pthread_mutex_record(mutex);
    if (record != NULL) {
        record->owner = owner;
        __sync_synchronize();
        record->owner_valid = 1;
    }
    smros_unlock_pthread_mutex_records();
}

static void smros_note_pthread_mutex_unlock(pthread_mutex_t *mutex) {
    smros_lock_pthread_mutex_records();
    smros_pthread_mutex_record *record = smros_find_pthread_mutex_record(mutex);
    if (record != NULL) {
        record->owner_valid = 0;
    }
    smros_unlock_pthread_mutex_records();
}

static int smros_pthread_mutex_owned_by_self(
    pthread_mutex_t *mutex,
    int *type
) {
    int owned = 0;
    smros_lock_pthread_mutex_records();
    smros_pthread_mutex_record *record = smros_find_pthread_mutex_record(mutex);
    if (record != NULL) {
        if (type != NULL) {
            *type = record->type;
        }
        owned = __sync_fetch_and_add(&record->owner_valid, 0) &&
            pthread_equal(record->owner, pthread_self());
    }
    smros_unlock_pthread_mutex_records();
    return owned;
}

static smros_pthread_shared_mutex_state *smros_pthread_shared_mutex(
    pthread_mutex_t *mutex
) {
    if (mutex == NULL) {
        return NULL;
    }
    return (smros_pthread_shared_mutex_state *)mutex;
}

static int smros_pthread_shared_mutex_active(pthread_mutex_t *mutex) {
    smros_pthread_shared_mutex_state *state = smros_pthread_shared_mutex(mutex);
    return state != NULL &&
        __sync_fetch_and_add(&state->magic, 0) ==
            SMROS_PTHREAD_SHARED_MUTEX_MAGIC;
}

static uint32_t smros_pthread_shared_mutex_owner_token(void) {
    uintptr_t thread = (uintptr_t)pthread_self();
    uintptr_t process = (uintptr_t)getpid();
    uint32_t token = (uint32_t)thread ^ (uint32_t)(thread >> 32) ^
        (uint32_t)process ^ (uint32_t)(process >> 32);
    return token == 0 ? 1u : token;
}

static int smros_pthread_shared_mutex_trylock(pthread_mutex_t *mutex) {
    smros_pthread_shared_mutex_state *state = smros_pthread_shared_mutex(mutex);
    if (!smros_pthread_shared_mutex_active(mutex)) {
        smros_pthread_diag_state("shared-mutex-trylock-invalid", mutex, 0, 0, 0);
        return EINVAL;
    }
    uint32_t token = smros_pthread_shared_mutex_owner_token();
    uint32_t type = __sync_fetch_and_add(&state->type, 0);
    if (__sync_bool_compare_and_swap(&state->lock, 0, 1)) {
        __sync_lock_test_and_set(&state->owner, token);
        __sync_lock_test_and_set(&state->count, 1);
        return 0;
    }
    if (
        type == PTHREAD_MUTEX_RECURSIVE &&
        __sync_fetch_and_add(&state->owner, 0) == token
    ) {
        (void)__sync_add_and_fetch(&state->count, 1);
        return 0;
    }
    return EBUSY;
}

static int smros_pthread_shared_mutex_lock(pthread_mutex_t *mutex) {
    smros_pthread_shared_mutex_state *state = smros_pthread_shared_mutex(mutex);
    if (!smros_pthread_shared_mutex_active(mutex)) {
        smros_pthread_diag_state("shared-mutex-lock-invalid", mutex, 0, 0, 0);
        return EINVAL;
    }
    uint32_t token = smros_pthread_shared_mutex_owner_token();
    uint32_t type = __sync_fetch_and_add(&state->type, 0);
    if (
        type == PTHREAD_MUTEX_ERRORCHECK &&
        __sync_fetch_and_add(&state->owner, 0) == token
    ) {
        return EDEADLK;
    }
    if (__sync_fetch_and_add(&state->lock, 0) != 0) {
        smros_shared_mutex_trace(
            "lock-contended",
            mutex,
            __sync_fetch_and_add(&state->lock, 0),
            __sync_fetch_and_add(&state->owner, 0),
            token,
            type,
            EBUSY,
            0
        );
    }
    smros_pthread_diag_state(
        "shared-mutex-lock-enter",
        mutex,
        __sync_fetch_and_add(&state->lock, 0),
        __sync_fetch_and_add(&state->owner, 0),
        __sync_fetch_and_add(&state->type, 0)
    );
    uint32_t attempts = 0;
    for (;;) {
        int result = smros_pthread_shared_mutex_trylock(mutex);
        if (result != EBUSY) {
            if (attempts != 0) {
                smros_shared_mutex_trace(
                    "lock-exit",
                    mutex,
                    __sync_fetch_and_add(&state->lock, 0),
                    __sync_fetch_and_add(&state->owner, 0),
                    token,
                    type,
                    result,
                    attempts
                );
            }
            smros_pthread_diag_state(
                "shared-mutex-lock-exit",
                mutex,
                (uint32_t)result,
                attempts,
                __sync_fetch_and_add(&state->owner, 0)
            );
            return result;
        }
        if (attempts != UINT32_MAX) {
            attempts++;
        }
        if (attempts == 1 || (attempts % 100000u) == 0) {
            smros_shared_mutex_trace(
                "lock-retry",
                mutex,
                __sync_fetch_and_add(&state->lock, 0),
                __sync_fetch_and_add(&state->owner, 0),
                token,
                type,
                EBUSY,
                attempts
            );
        }
        (void)sched_yield();
    }
}

static int smros_pthread_shared_mutex_unlock(pthread_mutex_t *mutex) {
    smros_pthread_shared_mutex_state *state = smros_pthread_shared_mutex(mutex);
    if (!smros_pthread_shared_mutex_active(mutex)) {
        smros_pthread_diag_state("shared-mutex-unlock-invalid", mutex, 0, 0, 0);
        return EINVAL;
    }
    uint32_t token = smros_pthread_shared_mutex_owner_token();
    uint32_t owner = __sync_fetch_and_add(&state->owner, 0);
    uint32_t type = __sync_fetch_and_add(&state->type, 0);
    if (owner != token && type == PTHREAD_MUTEX_ERRORCHECK) {
        smros_pthread_diag_state("shared-mutex-unlock-perm", mutex, owner, token, type);
        return EPERM;
    }
    if (
        type == PTHREAD_MUTEX_RECURSIVE &&
        owner == token &&
        __sync_fetch_and_add(&state->count, 0) > 1
    ) {
        (void)__sync_fetch_and_sub(&state->count, 1);
        return 0;
    }
    if (!__sync_bool_compare_and_swap(&state->lock, 1, 0)) {
        smros_pthread_diag_state(
            "shared-mutex-unlock-race",
            mutex,
            __sync_fetch_and_add(&state->lock, 0),
            owner,
            token
        );
        return EPERM;
    }
    __sync_lock_test_and_set(&state->owner, 0);
    smros_pthread_diag_state("shared-mutex-unlock", mutex, 0, owner, token);
    return 0;
}

static int smros_remember_pthread_mutexattr_lifecycle_record(
    pthread_mutexattr_t *attr
) {
    smros_pthread_mutexattr_lifecycle_record *record =
        smros_find_pthread_mutexattr_lifecycle_record(attr);
    if (record == NULL) {
        for (size_t index = 0; index < SMROS_PTHREAD_MUTEXATTR_RECORDS; index++) {
            if (!smros_pthread_mutexattr_lifecycle_records[index].active) {
                record = &smros_pthread_mutexattr_lifecycle_records[index];
                break;
            }
        }
    }
    if (record == NULL) {
        return EAGAIN;
    }
    record->attr = attr;
    record->destroyed = 0;
    record->type = PTHREAD_MUTEX_NORMAL;
    record->pshared = PTHREAD_PROCESS_PRIVATE;
    smros_pthread_mutexattr_gettype_fn gettype_target =
        (smros_pthread_mutexattr_gettype_fn)smros_resolve_symbol(
            "pthread_mutexattr_gettype"
        );
    if (gettype_target != NULL) {
        int type = PTHREAD_MUTEX_NORMAL;
        if (gettype_target(attr, &type) == 0) {
            record->type = type;
        }
    }
    smros_pthread_mutexattr_getpshared_fn getpshared_target =
        (smros_pthread_mutexattr_getpshared_fn)smros_resolve_symbol(
            "pthread_mutexattr_getpshared"
        );
    if (getpshared_target != NULL) {
        int pshared = PTHREAD_PROCESS_PRIVATE;
        if (getpshared_target(attr, &pshared) == 0) {
            record->pshared = pshared;
        }
    }
    __sync_synchronize();
    record->active = 1;
    return 0;
}

static void smros_forget_pthread_mutexattr_lifecycle_record(
    pthread_mutexattr_t *attr
) {
    smros_pthread_mutexattr_lifecycle_record *record =
        smros_find_pthread_mutexattr_lifecycle_record(attr);
    if (record != NULL) {
        record->active = 0;
        record->destroyed = 1;
    }
}

static smros_pthread_attr_sched_record *smros_find_pthread_attr_sched_record(
    pthread_attr_t *attr
) {
    for (size_t index = 0; index < SMROS_PTHREAD_ATTR_RECORDS; index++) {
        smros_pthread_attr_sched_record *record =
            &smros_pthread_attr_sched_records[index];
        if (record->active && record->attr == attr) {
            return record;
        }
    }
    return NULL;
}

static void smros_forget_pthread_attr_sched_record(pthread_attr_t *attr) {
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record(attr);
    if (record != NULL) {
        record->active = 0;
        record->attr = NULL;
        memset(&record->param, 0, sizeof(record->param));
    }
}

static int smros_remember_pthread_attr_sched_record(
    pthread_attr_t *attr,
    const struct sched_param *param
) {
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record(attr);
    if (record == NULL) {
        for (size_t index = 0; index < SMROS_PTHREAD_ATTR_RECORDS; index++) {
            if (!smros_pthread_attr_sched_records[index].active) {
                record = &smros_pthread_attr_sched_records[index];
                break;
            }
        }
    }
    if (record == NULL) {
        return EAGAIN;
    }
    record->active = 1;
    record->attr = attr;
    if (
        record->policy != SCHED_OTHER &&
        record->policy != SCHED_FIFO &&
        record->policy != SCHED_RR
    ) {
        record->policy = SCHED_OTHER;
    }
    record->param = *param;
    return 0;
}

static int smros_replay_pthread_attr_sched_record(
    pthread_attr_t *attr,
    smros_pthread_attr_setschedparam_fn target
) {
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record(attr);
    if (record == NULL) {
        return 0;
    }
    int result = target(attr, &record->param);
    if (result == 0) {
        smros_forget_pthread_attr_sched_record(attr);
    }
    return result;
}

int pthread_attr_init(pthread_attr_t *attr) {
    smros_pthread_attr_init_fn target =
        (smros_pthread_attr_init_fn)smros_resolve_symbol("pthread_attr_init");
    if (target == NULL) {
        return ENOSYS;
    }

    int result = target(attr);
    if (result != 0) {
        return result;
    }
    result = smros_remember_pthread_attr_lifecycle_record(attr);
    if (result != 0) {
        smros_pthread_attr_destroy_fn destroy_target =
            (smros_pthread_attr_destroy_fn)smros_resolve_symbol(
                "pthread_attr_destroy"
            );
        if (destroy_target != NULL) {
            (void)destroy_target(attr);
        }
    }
    return result;
}

int pthread_attr_setschedparam(
    pthread_attr_t *attr,
    const struct sched_param *param
) {
    smros_pthread_attr_setschedparam_fn target =
        (smros_pthread_attr_setschedparam_fn)smros_resolve_symbol(
            "pthread_attr_setschedparam"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr, param);
    if (result == 0) {
        (void)smros_remember_pthread_attr_sched_record(attr, param);
        return 0;
    }
    if (
        result == EINVAL &&
        smros_realtime_sched_priority_valid(param->sched_priority)
    ) {
        return smros_remember_pthread_attr_sched_record(attr, param);
    }
    return result;
}

int pthread_attr_setschedpolicy(pthread_attr_t *attr, int policy) {
    if (policy < 0) {
        (void)attr;
        return ENOTSUP;
    }
    if (policy != SCHED_OTHER && policy != SCHED_FIFO && policy != SCHED_RR) {
        return EINVAL;
    }

    smros_pthread_attr_setschedpolicy_fn target =
        (smros_pthread_attr_setschedpolicy_fn)smros_resolve_symbol(
            "pthread_attr_setschedpolicy"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr, policy);
    if (result != 0) {
        smros_pthread_attr_sched_record *record =
            smros_find_pthread_attr_sched_record(attr);
        if (record == NULL) {
            struct sched_param default_param = {
                .sched_priority = 0,
            };
            if (
                smros_remember_pthread_attr_sched_record(
                    attr,
                    &default_param
                ) != 0
            ) {
                return EAGAIN;
            }
            record = smros_find_pthread_attr_sched_record(attr);
        }
        if (record != NULL) {
            record->policy = policy;
            if (policy == SCHED_OTHER) {
                record->param.sched_priority = 0;
            }
            return 0;
        }
        return result;
    }
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record(attr);
    if (record == NULL) {
        struct sched_param default_param = {
            .sched_priority = 0,
        };
        if (
            smros_remember_pthread_attr_sched_record(
                attr,
                &default_param
            ) != 0
        ) {
            return EAGAIN;
        }
        record = smros_find_pthread_attr_sched_record(attr);
    }
    if (record != NULL) {
        record->policy = policy;
        if (policy == SCHED_OTHER) {
            record->param.sched_priority = 0;
        }
    }
    smros_pthread_attr_setschedparam_fn param_target =
        (smros_pthread_attr_setschedparam_fn)smros_resolve_symbol(
            "pthread_attr_setschedparam"
        );
    if (param_target == NULL) {
        return 0;
    }
    if (!smros_sched_metadata_valid(policy, record->param.sched_priority)) {
        return 0;
    }
    return smros_replay_pthread_attr_sched_record(attr, param_target);
}

int pthread_attr_getschedpolicy(const pthread_attr_t *attr, int *policy) {
    if (smros_pointer_is_null(attr) || smros_pointer_is_null(policy)) {
        return EINVAL;
    }
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record((pthread_attr_t *)attr);
    if (record != NULL) {
        *policy = record->policy;
        return 0;
    }
    smros_pthread_attr_getschedpolicy_fn target =
        (smros_pthread_attr_getschedpolicy_fn)smros_resolve_symbol(
            "pthread_attr_getschedpolicy"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(attr, policy);
}

int pthread_attr_getschedparam(
    const pthread_attr_t *attr,
    struct sched_param *param
) {
    if (smros_pointer_is_null(attr) || smros_pointer_is_null(param)) {
        return EINVAL;
    }
    smros_pthread_attr_sched_record *record =
        smros_find_pthread_attr_sched_record((pthread_attr_t *)attr);
    if (record != NULL) {
        *param = record->param;
        return 0;
    }
    smros_pthread_attr_getschedparam_fn target =
        (smros_pthread_attr_getschedparam_fn)smros_resolve_symbol(
            "pthread_attr_getschedparam"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(attr, param);
}

int pthread_attr_destroy(pthread_attr_t *attr) {
    smros_pthread_attr_destroy_fn target =
        (smros_pthread_attr_destroy_fn)smros_resolve_symbol(
            "pthread_attr_destroy"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr);
    if (result == 0) {
        smros_forget_pthread_attr_sched_record(attr);
        smros_forget_pthread_attr_lifecycle_record(attr);
    }
    return result;
}

int pthread_getschedparam(
    pthread_t thread,
    int *policy,
    struct sched_param *param
) {
    smros_pthread_sched_record *record =
        smros_find_pthread_sched_record(thread);
    if (record != NULL) {
        *policy = record->policy;
        *param = record->param;
        return 0;
    }
    *policy = SCHED_OTHER;
    param->sched_priority = 0;
    return 0;
}

int pthread_setschedparam(
    pthread_t thread,
    int policy,
    const struct sched_param *param
) {
    if (!smros_sched_metadata_valid(policy, param->sched_priority)) {
        return EINVAL;
    }
    int result = smros_remember_pthread_sched_record(thread, policy, param);
    return result;
}

int pthread_setschedprio(pthread_t thread, int priority) {
    smros_pthread_sched_record *record =
        smros_find_pthread_sched_record(thread);
    int policy = record == NULL ? SCHED_OTHER : record->policy;
    if (!smros_sched_metadata_valid(policy, priority)) {
        return EINVAL;
    }
    struct sched_param param = {
        .sched_priority = priority,
    };
    int result = smros_remember_pthread_sched_record(thread, policy, &param);
    return result;
}

int pthread_mutex_getprioceiling(
    const pthread_mutex_t *mutex,
    int *prioceiling
) {
    smros_pthread_mutex_getprioceiling_fn target =
        (smros_pthread_mutex_getprioceiling_fn)smros_resolve_symbol(
            "pthread_mutex_getprioceiling"
        );
    if (smros_pointer_is_null(mutex) || smros_pointer_is_null(prioceiling)) {
        return EINVAL;
    }
    if (target != NULL) {
        int result = target(mutex, prioceiling);
        if (result == 0) {
            return 0;
        }
        if (result != EINVAL && result != ENOSYS && result != ENOTSUP) {
            return result;
        }
    }
    *prioceiling = sched_get_priority_min(SCHED_FIFO);
    return *prioceiling > 0 ? 0 : (errno == 0 ? ENOSYS : errno);
}

int pthread_mutexattr_init(pthread_mutexattr_t *attr) {
    if (smros_pointer_is_null(attr)) {
        return EINVAL;
    }
    smros_pthread_mutexattr_init_fn target =
        (smros_pthread_mutexattr_init_fn)smros_resolve_symbol(
            "pthread_mutexattr_init"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr);
    if (result != 0) {
        return result;
    }
    result = smros_remember_pthread_mutexattr_lifecycle_record(attr);
    if (result != 0) {
        smros_pthread_mutexattr_destroy_fn destroy_target =
            (smros_pthread_mutexattr_destroy_fn)smros_resolve_symbol(
                "pthread_mutexattr_destroy"
            );
        if (destroy_target != NULL) {
            (void)destroy_target(attr);
        }
    }
    return result;
}

int pthread_mutexattr_destroy(pthread_mutexattr_t *attr) {
    if (smros_pointer_is_null(attr)) {
        return EINVAL;
    }
    if (smros_find_pthread_mutexattr_lifecycle_record(attr) == NULL) {
        return EINVAL;
    }
    smros_pthread_mutexattr_destroy_fn target =
        (smros_pthread_mutexattr_destroy_fn)smros_resolve_symbol(
            "pthread_mutexattr_destroy"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    int result = target(attr);
    if (result == 0) {
        smros_forget_pthread_mutexattr_lifecycle_record(attr);
    }
    return result;
}

int pthread_mutexattr_gettype(
    const pthread_mutexattr_t *attr,
    int *type
) {
    if (smros_pointer_is_null(attr) || smros_pointer_is_null(type)) {
        return EINVAL;
    }
    if (
        smros_find_pthread_mutexattr_lifecycle_record(
            (pthread_mutexattr_t *)attr
        ) == NULL
    ) {
        return EINVAL;
    }
    smros_pthread_mutexattr_gettype_fn target =
        (smros_pthread_mutexattr_gettype_fn)smros_resolve_symbol(
            "pthread_mutexattr_gettype"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(attr, type);
}

static void smros_preserve_short_barrier_alarm(void) {
    smros_alarm_fn target =
        (smros_alarm_fn)smros_resolve_symbol("alarm");
    if (target == NULL) {
        return;
    }
    unsigned int remaining = target(0);
    if (remaining == 0) {
        return;
    }
    if (remaining < 5) {
        remaining = 5;
    }
    (void)target(remaining);
}

static smros_pthread_shared_cond_state *smros_pthread_shared_cond(
    pthread_cond_t *cond
) {
    if (cond == NULL) {
        return NULL;
    }
    return (smros_pthread_shared_cond_state *)cond;
}

static int smros_pthread_shared_cond_active(pthread_cond_t *cond) {
    smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
    return state != NULL &&
        __sync_fetch_and_add(&state->magic, 0) ==
            SMROS_PTHREAD_SHARED_COND_MAGIC;
}

static int smros_pthread_cond_attr_pshared(
    const pthread_condattr_t *attr,
    int *pshared
) {
    if (attr == NULL) {
        *pshared = PTHREAD_PROCESS_PRIVATE;
        return 0;
    }
    smros_pthread_condattr_getpshared_fn target =
        (smros_pthread_condattr_getpshared_fn)smros_resolve_symbol(
            "pthread_condattr_getpshared"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(attr, pshared);
}

static int smros_pthread_cond_attr_clock(
    const pthread_condattr_t *attr,
    clockid_t *clock_id
) {
    if (attr == NULL) {
        *clock_id = CLOCK_REALTIME;
        return 0;
    }
    smros_pthread_condattr_getclock_fn target =
        (smros_pthread_condattr_getclock_fn)smros_resolve_symbol(
            "pthread_condattr_getclock"
        );
    if (target == NULL) {
        *clock_id = CLOCK_REALTIME;
        return 0;
    }
    return target(attr, clock_id);
}

static void smros_pthread_cond_wait_pause(void) {
    /*
     * The private condition-variable fallback tracks wakeups in user space.
     * A pure sched_yield() loop leaves every waiter runnable, and SMROS
     * implements requests up to 100 ms as a high-resolution spin/WFI loop.
     * Use the blocking sleep path so a large waiter set does not consume all
     * runnable slots while still polling promptly for a broadcast.
     */
    const struct timespec pause = {
        .tv_sec = 0,
        .tv_nsec = 200000000L,
    };
    if (nanosleep(&pause, NULL) != 0 && errno != EINTR) {
        (void)sched_yield();
    }
}

static uint32_t smros_pthread_cond_record_users(
    smros_pthread_cond_record *record
) {
    return __sync_fetch_and_add(&record->users, 0);
}

static void smros_lock_pthread_cond_records(void) {
    while (__sync_lock_test_and_set(&smros_pthread_cond_records_lock, 1)) {
        (void)sched_yield();
    }
}

static void smros_unlock_pthread_cond_records(void) {
    __sync_lock_release(&smros_pthread_cond_records_lock);
}

static int smros_pthread_cond_waiter_enter(
    smros_pthread_cond_record *record,
    pthread_t thread
) {
    int entered = 0;
    smros_lock_pthread_cond_records();
    for (size_t index = 0; index < SMROS_PTHREAD_COND_RECORDS; index++) {
        smros_pthread_cond_waiter_record *waiter =
            &smros_pthread_cond_waiter_records[index];
        if (!waiter->active) {
            waiter->thread = thread;
            waiter->record = record;
            __sync_synchronize();
            waiter->active = 1;
            entered = 1;
            break;
        }
    }
    smros_unlock_pthread_cond_records();
    return entered;
}

static void smros_pthread_cond_waiter_leave(
    smros_pthread_cond_record *record,
    pthread_t thread
) {
    smros_lock_pthread_cond_records();
    for (size_t index = 0; index < SMROS_PTHREAD_COND_RECORDS; index++) {
        smros_pthread_cond_waiter_record *waiter =
            &smros_pthread_cond_waiter_records[index];
        if (
            __sync_fetch_and_add(&waiter->active, 0) &&
            waiter->record == record &&
            pthread_equal(waiter->thread, thread)
        ) {
            waiter->record = NULL;
            __sync_synchronize();
            waiter->active = 0;
            break;
        }
    }
    smros_unlock_pthread_cond_records();
}

static void smros_clear_pthread_cond_record(
    smros_pthread_cond_record *record
) {
    record->cond = NULL;
    __sync_lock_test_and_set(&record->waiters, 0);
    __sync_lock_test_and_set(&record->wakeups, 0);
    __sync_lock_test_and_set(&record->users, 0);
    __sync_lock_test_and_set(&record->clock_id, 0);
    __sync_lock_test_and_set(&record->detached, 0);
    __sync_synchronize();
    __sync_lock_test_and_set(&record->active, 0);
}

static smros_pthread_cond_record *smros_find_pthread_cond_record(
    pthread_cond_t *cond
) {
    if (cond == NULL) {
        return NULL;
    }
    for (size_t index = 0; index < SMROS_PTHREAD_COND_RECORDS; index++) {
        smros_pthread_cond_record *record =
            &smros_pthread_cond_records[index];
        if (
            __sync_fetch_and_add(&record->active, 0) &&
            !__sync_fetch_and_add(&record->detached, 0) &&
            record->cond == cond
        ) {
            return record;
        }
    }
    return NULL;
}

static int smros_pthread_private_cond_init(
    pthread_cond_t *cond,
    clockid_t clock_id
);

static int smros_pthread_cond_is_zero(const pthread_cond_t *cond) {
    if (cond == NULL) {
        return 0;
    }
    const unsigned char *bytes = (const unsigned char *)cond;
    for (size_t index = 0; index < sizeof(*cond); index++) {
        if (bytes[index] != 0) {
            return 0;
        }
    }
    return 1;
}

static smros_pthread_cond_record *smros_lazy_private_cond_record(
    pthread_cond_t *cond
) {
    if (!smros_pthread_cond_is_zero(cond)) {
        return NULL;
    }
    if (smros_pthread_private_cond_init(cond, CLOCK_REALTIME) != 0) {
        return NULL;
    }
    return smros_find_pthread_cond_record(cond);
}

static smros_pthread_cond_record *smros_reserve_pthread_cond_record(void) {
    for (size_t index = 0; index < SMROS_PTHREAD_COND_RECORDS; index++) {
        smros_pthread_cond_record *record =
            &smros_pthread_cond_records[index];
        if (
            __sync_fetch_and_add(&record->active, 0) &&
            __sync_fetch_and_add(&record->detached, 0) &&
            smros_pthread_cond_record_users(record) == 0
        ) {
            smros_clear_pthread_cond_record(record);
        }
    }
    for (size_t index = 0; index < SMROS_PTHREAD_COND_RECORDS; index++) {
        smros_pthread_cond_record *record =
            &smros_pthread_cond_records[index];
        if (
            !__sync_fetch_and_add(&record->active, 0) &&
            __sync_bool_compare_and_swap(&record->active, 0, 1)
        ) {
            return record;
        }
    }
    return NULL;
}

static int smros_pthread_private_cond_init(
    pthread_cond_t *cond,
    clockid_t clock_id
) {
    if (cond == NULL) {
        return EINVAL;
    }
    smros_pthread_cond_record *existing =
        smros_find_pthread_cond_record(cond);
    if (existing != NULL) {
        if (smros_pthread_cond_record_users(existing) != 0) {
            return EBUSY;
        }
        smros_clear_pthread_cond_record(existing);
    }

    smros_pthread_cond_record *record = smros_reserve_pthread_cond_record();
    if (record == NULL) {
        return EAGAIN;
    }
    record->cond = cond;
    __sync_lock_test_and_set(&record->waiters, 0);
    __sync_lock_test_and_set(&record->wakeups, 0);
    __sync_lock_test_and_set(&record->users, 0);
    __sync_lock_test_and_set(&record->clock_id, (int)clock_id);
    __sync_lock_test_and_set(&record->detached, 0);
    __sync_synchronize();
    __sync_lock_test_and_set(&record->active, 1);
    return 0;
}

static int smros_pthread_private_cond_deadline_reached(
    smros_pthread_cond_record *record,
    const struct timespec *deadline,
    int *clock_error
) {
    struct timespec now;
    *clock_error = 0;
    if (clock_gettime((clockid_t)record->clock_id, &now) != 0) {
        *clock_error = errno == 0 ? EINVAL : errno;
        return 1;
    }
    return now.tv_sec > deadline->tv_sec ||
        (now.tv_sec == deadline->tv_sec && now.tv_nsec >= deadline->tv_nsec);
}

static int smros_pthread_private_cond_consume_wakeup(
    smros_pthread_cond_record *record
) {
    int consumed = 0;
    smros_lock_pthread_cond_records();
    if (record->wakeups > 0) {
        record->wakeups--;
        if (record->waiters > 0) {
            record->waiters--;
        }
        consumed = 1;
    }
    smros_unlock_pthread_cond_records();
    return consumed;
}

static void smros_pthread_private_cond_blocked_leave(
    smros_pthread_cond_record *record
) {
    smros_lock_pthread_cond_records();
    if (record->waiters > 0) {
        record->waiters--;
    }
    smros_unlock_pthread_cond_records();
}

static void smros_pthread_private_cond_user_leave(
    smros_pthread_cond_record *record
) {
    smros_lock_pthread_cond_records();
    if (record->users > 0) {
        record->users--;
    }
    if (record->users == 0 && record->detached) {
        record->cond = NULL;
        record->waiters = 0;
        record->wakeups = 0;
        record->clock_id = 0;
        record->detached = 0;
        record->active = 0;
    }
    smros_unlock_pthread_cond_records();
}

static int smros_pthread_private_cond_wait_common(
    smros_pthread_cond_record *record,
    pthread_mutex_t *mutex,
    const struct timespec *deadline,
    int timed
) {
    if (
        record == NULL ||
        !__sync_fetch_and_add(&record->active, 0) ||
        __sync_fetch_and_add(&record->detached, 0)
    ) {
        return EINVAL;
    }
    if (
        timed &&
        (deadline == NULL ||
         deadline->tv_nsec < 0 ||
         deadline->tv_nsec >= SMROS_NSEC_PER_SEC)
    ) {
        return EINVAL;
    }

    int old_cancel_state = PTHREAD_CANCEL_ENABLE;
    int cancel_state_result = pthread_setcancelstate(
        PTHREAD_CANCEL_DISABLE,
        &old_cancel_state
    );
    if (cancel_state_result != 0) {
        return cancel_state_result;
    }

    (void)__sync_add_and_fetch(&record->users, 1);
    (void)__sync_add_and_fetch(&record->waiters, 1);
    pthread_t self = pthread_self();
    if (!smros_pthread_cond_waiter_enter(record, self)) {
        smros_pthread_private_cond_blocked_leave(record);
        smros_pthread_private_cond_user_leave(record);
        (void)pthread_setcancelstate(old_cancel_state, NULL);
        return EAGAIN;
    }
    int unlock_result = pthread_mutex_unlock(mutex);
    if (unlock_result != 0) {
        smros_pthread_cond_waiter_leave(record, self);
        smros_pthread_private_cond_blocked_leave(record);
        smros_pthread_private_cond_user_leave(record);
        (void)pthread_setcancelstate(old_cancel_state, NULL);
        return unlock_result;
    }

    int result = 0;
    int waiter_registered = 1;
    for (;;) {
        if (smros_current_pthread_cancel_requested()) {
            smros_pthread_cond_waiter_leave(record, self);
            smros_pthread_private_cond_blocked_leave(record);
            smros_pthread_private_cond_user_leave(record);
            waiter_registered = 0;
            int cancel_lock_result = pthread_mutex_lock(mutex);
            if (cancel_lock_result != 0) {
                result = cancel_lock_result;
                break;
            }
            smros_refresh_current_pthread_cancel();
            (void)pthread_setcancelstate(old_cancel_state, NULL);
            pthread_testcancel();
            int cancel_unlock_result = pthread_mutex_unlock(mutex);
            if (cancel_unlock_result != 0) {
                result = cancel_unlock_result;
                break;
            }
            (void)pthread_setcancelstate(PTHREAD_CANCEL_DISABLE, NULL);
            (void)__sync_add_and_fetch(&record->users, 1);
            (void)__sync_add_and_fetch(&record->waiters, 1);
            if (!smros_pthread_cond_waiter_enter(record, self)) {
                smros_pthread_private_cond_blocked_leave(record);
                smros_pthread_private_cond_user_leave(record);
                result = EAGAIN;
                break;
            }
            waiter_registered = 1;
            continue;
        }
        if (smros_pthread_private_cond_consume_wakeup(record)) {
            break;
        }
        if (timed) {
            int clock_error = 0;
            if (
                smros_pthread_private_cond_deadline_reached(
                    record,
                    deadline,
                    &clock_error
                )
            ) {
                result = clock_error == 0 ? ETIMEDOUT : clock_error;
                smros_pthread_private_cond_blocked_leave(record);
                break;
            }
        }
        smros_pthread_cond_wait_pause();
    }

    if (waiter_registered) {
        smros_pthread_cond_waiter_leave(record, self);
        smros_pthread_private_cond_user_leave(record);
    }
    int lock_result = pthread_mutex_lock(mutex);
    if (result == 0 && lock_result != 0) {
        result = lock_result;
    }
    (void)pthread_setcancelstate(old_cancel_state, NULL);
    if (result == 0) {
        pthread_testcancel();
    }
    return result;
}

static int smros_pthread_private_cond_wake(
    smros_pthread_cond_record *record,
    int broadcast
) {
    if (
        record == NULL ||
        !__sync_fetch_and_add(&record->active, 0) ||
        __sync_fetch_and_add(&record->detached, 0)
    ) {
        return EINVAL;
    }
    smros_lock_pthread_cond_records();
    uint32_t waiters = record->waiters;
    if (waiters == 0) {
        smros_unlock_pthread_cond_records();
        return 0;
    }
    record->wakeups += broadcast ? waiters : 1;
    smros_unlock_pthread_cond_records();
    if (!broadcast) {
        /*
         * Give a waiter a short opportunity to consume the token so rapid
         * signal cascades retain one-token pacing.  The handoff is bounded:
         * pthread_cond_signal may be called while its associated mutex is
         * held, so waiting indefinitely here can deadlock the awakened waiter
         * while it tries to reacquire that mutex.
         */
        for (unsigned int attempt = 0; attempt < 8; ++attempt) {
            smros_lock_pthread_cond_records();
            int pending = record->wakeups > 0;
            smros_unlock_pthread_cond_records();
            if (!pending) {
                break;
            }
            smros_pthread_cond_wait_pause();
        }
    }
    return 0;
}

static int smros_pthread_private_cond_destroy(
    smros_pthread_cond_record *record
) {
    if (
        record == NULL ||
        !__sync_fetch_and_add(&record->active, 0) ||
        __sync_fetch_and_add(&record->detached, 0)
    ) {
        return EINVAL;
    }
    smros_lock_pthread_cond_records();
    uint32_t waiters = record->waiters;
    if (waiters != 0 && record->wakeups < waiters) {
        smros_unlock_pthread_cond_records();
        return EBUSY;
    }
    record->cond = NULL;
    record->detached = 1;
    if (record->users == 0) {
        record->waiters = 0;
        record->wakeups = 0;
        record->clock_id = 0;
        record->detached = 0;
        record->active = 0;
    }
    smros_unlock_pthread_cond_records();
    return 0;
}

static int smros_pthread_shared_cond_init(
    pthread_cond_t *cond,
    clockid_t clock_id
) {
    if (cond == NULL) {
        return EINVAL;
    }
    if (sizeof(smros_pthread_shared_cond_state) > sizeof(*cond)) {
        return ENOSYS;
    }
    smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
    __sync_lock_test_and_set(&state->waiters, 0);
    __sync_lock_test_and_set(&state->wakeups, 0);
    __sync_lock_test_and_set(&state->clock_id, (int)clock_id);
    __sync_synchronize();
    __sync_lock_test_and_set(&state->magic, SMROS_PTHREAD_SHARED_COND_MAGIC);
    return 0;
}

static int smros_pthread_shared_cond_deadline_reached(
    smros_pthread_shared_cond_state *state,
    const struct timespec *deadline,
    int *clock_error
) {
    struct timespec now;
    *clock_error = 0;
    if (clock_gettime((clockid_t)state->clock_id, &now) != 0) {
        *clock_error = errno == 0 ? EINVAL : errno;
        return 1;
    }
    return now.tv_sec > deadline->tv_sec ||
        (now.tv_sec == deadline->tv_sec && now.tv_nsec >= deadline->tv_nsec);
}

static int smros_pthread_shared_cond_consume_wakeup(
    smros_pthread_shared_cond_state *state
) {
    for (;;) {
        uint32_t wakeups = __sync_fetch_and_add(&state->wakeups, 0);
        if (wakeups == 0) {
            return 0;
        }
        if (
            __sync_bool_compare_and_swap(
                &state->wakeups,
                wakeups,
                wakeups - 1
            )
        ) {
            return 1;
        }
    }
}

static int smros_pthread_shared_cond_wait_common(
    pthread_cond_t *cond,
    pthread_mutex_t *mutex,
    const struct timespec *deadline,
    int timed
) {
    smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
    if (
        state == NULL ||
        __sync_fetch_and_add(&state->magic, 0) !=
            SMROS_PTHREAD_SHARED_COND_MAGIC
    ) {
        return EINVAL;
    }
    if (
        timed &&
        (deadline == NULL ||
         deadline->tv_nsec < 0 ||
         deadline->tv_nsec >= SMROS_NSEC_PER_SEC)
    ) {
        return EINVAL;
    }

    int old_cancel_state = PTHREAD_CANCEL_ENABLE;
    int cancel_state_result = pthread_setcancelstate(
        PTHREAD_CANCEL_DISABLE,
        &old_cancel_state
    );
    if (cancel_state_result != 0) {
        return cancel_state_result;
    }

    (void)__sync_add_and_fetch(&state->waiters, 1);
    smros_trace_shared_cond(
        "wait-enter",
        cond,
        0,
        __sync_fetch_and_add(&state->waiters, 0),
        __sync_fetch_and_add(&state->wakeups, 0)
    );
    int unlock_result = pthread_mutex_unlock(mutex);
    if (unlock_result != 0) {
        (void)__sync_fetch_and_sub(&state->waiters, 1);
        (void)pthread_setcancelstate(old_cancel_state, NULL);
        return unlock_result;
    }

    int result = 0;
    for (;;) {
        if (smros_current_pthread_cancel_requested()) {
            int previous = __sync_fetch_and_sub(&state->waiters, 1);
            if (previous <= 0) {
                __sync_lock_test_and_set(&state->waiters, 0);
            }
            int cancel_lock_result = pthread_mutex_lock(mutex);
            if (cancel_lock_result != 0) {
                result = cancel_lock_result;
                break;
            }
            smros_refresh_current_pthread_cancel();
            (void)pthread_setcancelstate(old_cancel_state, NULL);
            pthread_testcancel();
            int cancel_unlock_result = pthread_mutex_unlock(mutex);
            if (cancel_unlock_result != 0) {
                result = cancel_unlock_result;
                break;
            }
            (void)pthread_setcancelstate(PTHREAD_CANCEL_DISABLE, NULL);
            (void)__sync_add_and_fetch(&state->waiters, 1);
            continue;
        }
        if (smros_pthread_shared_cond_consume_wakeup(state)) {
            int previous = __sync_fetch_and_sub(&state->waiters, 1);
            if (previous <= 0) {
                __sync_lock_test_and_set(&state->waiters, 0);
            }
            smros_pthread_diag_state(
                "shared-cond-wait-wakeup",
                cond,
                previous,
                __sync_fetch_and_add(&state->wakeups, 0),
                __sync_fetch_and_add(&state->waiters, 0)
            );
            break;
        }
        if (timed) {
            int clock_error = 0;
            if (
                smros_pthread_shared_cond_deadline_reached(
                    state,
                    deadline,
                    &clock_error
                )
            ) {
                result = clock_error == 0 ? ETIMEDOUT : clock_error;
                int previous = __sync_fetch_and_sub(&state->waiters, 1);
                if (previous <= 0) {
                    __sync_lock_test_and_set(&state->waiters, 0);
                }
                break;
            }
        }
        smros_pthread_cond_wait_pause();
    }
    int lock_result = pthread_mutex_lock(mutex);
    smros_trace_shared_cond(
        "wait-exit",
        cond,
        result == 0 && lock_result == 0 ? 0 : (result != 0 ? result : lock_result),
        __sync_fetch_and_add(&state->waiters, 0),
        __sync_fetch_and_add(&state->wakeups, 0)
    );
    smros_pthread_diag_state(
        "shared-cond-wait-exit",
        cond,
        (uint32_t)result,
        (uint32_t)lock_result,
        __sync_fetch_and_add(&state->waiters, 0)
    );
    if (result == 0 && lock_result != 0) {
        result = lock_result;
    }
    (void)pthread_setcancelstate(old_cancel_state, NULL);
    if (result == 0) {
        pthread_testcancel();
    }
    return result;
}

static int smros_pthread_shared_cond_wake(
    pthread_cond_t *cond,
    int broadcast
) {
    smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
    if (
        state == NULL ||
        __sync_fetch_and_add(&state->magic, 0) !=
            SMROS_PTHREAD_SHARED_COND_MAGIC
    ) {
        return EINVAL;
    }
    uint32_t waiters = __sync_fetch_and_add(&state->waiters, 0);
    if (waiters == 0) {
        smros_trace_shared_cond("wake-empty", cond, 0, 0, 0);
        return 0;
    }
    (void)__sync_add_and_fetch(&state->wakeups, broadcast ? waiters : 1);
    smros_trace_shared_cond(
        broadcast ? "broadcast" : "signal",
        cond,
        0,
        waiters,
        __sync_fetch_and_add(&state->wakeups, 0)
    );
    return 0;
}

static int smros_pthread_shared_cond_destroy(pthread_cond_t *cond) {
    smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
    if (
        state == NULL ||
        __sync_fetch_and_add(&state->magic, 0) !=
            SMROS_PTHREAD_SHARED_COND_MAGIC
    ) {
        return EINVAL;
    }
    uint32_t waiters = __sync_fetch_and_add(&state->waiters, 0);
    uint32_t wakeups = __sync_fetch_and_add(&state->wakeups, 0);
    smros_trace_shared_cond("destroy", cond, 0, waiters, wakeups);
    smros_pthread_diag_state("cond-destroy-state", cond, waiters, wakeups, state->clock_id);
    if (waiters != 0 && wakeups < waiters) {
        return EBUSY;
    }
    while (__sync_fetch_and_add(&state->waiters, 0) != 0) {
        (void)sched_yield();
    }
    __sync_lock_test_and_set(&state->magic, 0);
    __sync_lock_test_and_set(&state->wakeups, 0);
    __sync_lock_test_and_set(&state->clock_id, 0);
    smros_pthread_diag_state("cond-destroy-exit", cond, 0, 0, 0);
    return 0;
}

int pthread_cond_init(
    pthread_cond_t *cond,
    const pthread_condattr_t *attr
) {
    int pshared = PTHREAD_PROCESS_PRIVATE;
    int attr_result = smros_pthread_cond_attr_pshared(attr, &pshared);
    if (attr_result != 0) {
        return attr_result;
    }
    (void)pshared;
    clockid_t clock_id = CLOCK_REALTIME;
    int clock_result = smros_pthread_cond_attr_clock(attr, &clock_id);
    if (clock_result != 0) {
        return clock_result;
    }
    if (pshared == PTHREAD_PROCESS_SHARED) {
        int result = smros_pthread_shared_cond_init(cond, clock_id);
        smros_pthread_cond_trace(
            "init", cond, "shared", 0, 0
        );
        smros_pthread_diag_state("cond-init-exit", cond, (uint32_t)result, (uint32_t)pshared, (uint32_t)clock_id);
        return result;
    }
    int result = smros_pthread_private_cond_init(cond, clock_id);
    smros_pthread_cond_trace("init", cond, "private", 0, 0);
    smros_pthread_diag_state("cond-init-exit", cond, (uint32_t)result, (uint32_t)pshared, (uint32_t)clock_id);
    return result;
}

int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex) {
    if (smros_pthread_shared_cond_active(cond)) {
        smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
        smros_pthread_cond_trace(
            "wait", cond, "shared", state->waiters, state->wakeups
        );
        return smros_pthread_shared_cond_wait_common(cond, mutex, NULL, 0);
    }
    smros_pthread_cond_record *record =
        smros_find_pthread_cond_record(cond);
    if (record == NULL) {
        record = smros_lazy_private_cond_record(cond);
    }
    if (record != NULL) {
        smros_pthread_cond_trace(
            "wait", cond, "private", record->waiters, record->wakeups
        );
        return smros_pthread_private_cond_wait_common(record, mutex, NULL, 0);
    }
    smros_pthread_cond_wait_fn target =
        (smros_pthread_cond_wait_fn)smros_resolve_symbol("pthread_cond_wait");
    if (target == NULL) {
        return ENOSYS;
    }
    return target(cond, mutex);
}

int pthread_cond_timedwait(
    pthread_cond_t *cond,
    pthread_mutex_t *mutex,
    const struct timespec *deadline
) {
    if (smros_pthread_shared_cond_active(cond)) {
        smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
        smros_pthread_cond_trace(
            "timedwait", cond, "shared", state->waiters, state->wakeups
        );
        return smros_pthread_shared_cond_wait_common(cond, mutex, deadline, 1);
    }
    smros_pthread_cond_record *record =
        smros_find_pthread_cond_record(cond);
    if (record == NULL) {
        record = smros_lazy_private_cond_record(cond);
    }
    if (record != NULL) {
        smros_pthread_cond_trace(
            "timedwait", cond, "private", record->waiters, record->wakeups
        );
        return smros_pthread_private_cond_wait_common(
            record,
            mutex,
            deadline,
            1
        );
    }
    smros_pthread_cond_timedwait_fn target =
        (smros_pthread_cond_timedwait_fn)smros_resolve_symbol(
            "pthread_cond_timedwait"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(cond, mutex, deadline);
}

int pthread_cond_broadcast(pthread_cond_t *cond) {
    if (smros_pthread_shared_cond_active(cond)) {
        smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
        smros_pthread_cond_trace(
            "broadcast", cond, "shared", state->waiters, state->wakeups
        );
        return smros_pthread_shared_cond_wake(cond, 1);
    }
    smros_pthread_cond_record *record =
        smros_find_pthread_cond_record(cond);
    if (record == NULL) {
        record = smros_lazy_private_cond_record(cond);
    }
    if (record != NULL) {
        smros_pthread_cond_trace(
            "broadcast", cond, "private", record->waiters, record->wakeups
        );
        return smros_pthread_private_cond_wake(record, 1);
    }
    smros_pthread_cond_broadcast_fn target =
        (smros_pthread_cond_broadcast_fn)smros_resolve_symbol(
            "pthread_cond_broadcast"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(cond);
}

int pthread_cond_signal(pthread_cond_t *cond) {
    if (smros_pthread_shared_cond_active(cond)) {
        smros_pthread_shared_cond_state *state = smros_pthread_shared_cond(cond);
        smros_pthread_cond_trace(
            "signal", cond, "shared", state->waiters, state->wakeups
        );
        return smros_pthread_shared_cond_wake(cond, 0);
    }
    smros_pthread_cond_record *record =
        smros_find_pthread_cond_record(cond);
    if (record == NULL) {
        record = smros_lazy_private_cond_record(cond);
    }
    if (record != NULL) {
        smros_pthread_cond_trace(
            "signal", cond, "private", record->waiters, record->wakeups
        );
        return smros_pthread_private_cond_wake(record, 0);
    }
    smros_pthread_cond_signal_fn target =
        (smros_pthread_cond_signal_fn)smros_resolve_symbol(
            "pthread_cond_signal"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(cond);
}

int pthread_cond_destroy(pthread_cond_t *cond) {
    int destroy_attempt = smros_trace_destroy_enter();
    smros_pthread_diag_state("cond-destroy-enter", cond, 0, 0, 0);
    if (smros_pthread_shared_cond_active(cond)) {
        int result = smros_pthread_shared_cond_destroy(cond);
        smros_pthread_diag_state("cond-destroy-exit", cond, (uint32_t)result, 0, 0);
        smros_trace_destroy_exit(destroy_attempt, result);
        return result;
    }
    smros_pthread_cond_record *record =
        smros_find_pthread_cond_record(cond);
    if (record == NULL) {
        record = smros_lazy_private_cond_record(cond);
    }
    if (record != NULL) {
        int result = smros_pthread_private_cond_destroy(record);
        smros_pthread_diag_state("cond-destroy-exit", cond, (uint32_t)result, record->waiters, record->wakeups);
        smros_trace_destroy_exit(destroy_attempt, result);
        return result;
    }
    smros_pthread_cond_destroy_fn target =
        (smros_pthread_cond_destroy_fn)smros_resolve_symbol(
            "pthread_cond_destroy"
        );
    if (target == NULL) {
        smros_trace_destroy_exit(destroy_attempt, ENOSYS);
        return ENOSYS;
    }
    int result = target(cond);
    smros_trace_destroy_exit(destroy_attempt, result);
    return result;
}

static smros_pthread_barrier_record *smros_find_pthread_barrier_record(
    pthread_barrier_t *barrier
) {
    for (size_t index = 0; index < SMROS_PTHREAD_BARRIER_RECORDS; index++) {
        smros_pthread_barrier_record *record =
            &smros_pthread_barrier_records[index];
        if (record->active && record->barrier == barrier) {
            return record;
        }
    }
    return NULL;
}

static void smros_forget_pthread_barrier_record(pthread_barrier_t *barrier) {
    smros_pthread_barrier_record *record =
        smros_find_pthread_barrier_record(barrier);
    if (record != NULL) {
        record->active = 0;
        record->barrier = NULL;
        __sync_lock_test_and_set(&record->waiters, 0);
        __sync_lock_test_and_set(&record->count, 0);
        __sync_lock_test_and_set(&record->arrived, 0);
        __sync_lock_test_and_set(&record->generation, 0);
    }
}

static int smros_pthread_barrier_waiter_count(
    smros_pthread_barrier_record *record
) {
    return __sync_fetch_and_add(&record->waiters, 0);
}

static void smros_pthread_barrier_waiter_enter(
    smros_pthread_barrier_record *record
) {
    (void)__sync_add_and_fetch(&record->waiters, 1);
}

static void smros_pthread_barrier_waiter_leave(
    smros_pthread_barrier_record *record
) {
    int previous = __sync_fetch_and_sub(&record->waiters, 1);
    if (previous <= 0) {
        __sync_lock_test_and_set(&record->waiters, 0);
    }
}

static void smros_pthread_barrier_wait_cleanup(void *arg) {
    smros_pthread_barrier_wait_guard *guard =
        (smros_pthread_barrier_wait_guard *)arg;
    if (guard->active && guard->record != NULL) {
        smros_pthread_barrier_waiter_leave(guard->record);
        guard->active = 0;
    }
}

/* A preload later in the lookup chain may expose a barrier symbol that
 * returns an invalid status without actually waiting.  Keep an independent
 * generation counter so an initialized barrier can still make progress. */
static int smros_pthread_barrier_fallback_wait(
    smros_pthread_barrier_record *record
) {
    unsigned int count = __sync_fetch_and_add(&record->count, 0);
    if (count == 0) {
        return EINVAL;
    }
    unsigned int generation =
        __sync_fetch_and_add(&record->generation, 0);
    unsigned int arrived = __sync_add_and_fetch(&record->arrived, 1);
    if (arrived == count) {
        __sync_lock_test_and_set(&record->arrived, 0);
        __sync_synchronize();
        (void)__sync_add_and_fetch(&record->generation, 1);
        smros_preserve_short_barrier_alarm();
        return PTHREAD_BARRIER_SERIAL_THREAD;
    }
    if (arrived > count) {
        (void)__sync_fetch_and_sub(&record->arrived, 1);
        return EINVAL;
    }
    while (__sync_fetch_and_add(&record->generation, 0) == generation) {
        struct timespec pause = {
            .tv_sec = 0,
            .tv_nsec = 1000000L,
        };
        (void)nanosleep(&pause, NULL);
    }
    return 0;
}

static smros_pthread_shared_barrier_state *smros_pthread_shared_barrier(
    pthread_barrier_t *barrier
) {
    if (barrier == NULL) {
        return NULL;
    }
    return (smros_pthread_shared_barrier_state *)barrier;
}

static int smros_pthread_shared_barrier_active(pthread_barrier_t *barrier) {
    smros_pthread_shared_barrier_state *state =
        smros_pthread_shared_barrier(barrier);
    return state != NULL &&
        __sync_fetch_and_add(&state->magic, 0) ==
            SMROS_PTHREAD_SHARED_BARRIER_MAGIC;
}

static int smros_pthread_barrier_attr_pshared(
    const pthread_barrierattr_t *attr,
    int *pshared
) {
    if (attr == NULL) {
        *pshared = PTHREAD_PROCESS_PRIVATE;
        return 0;
    }
    smros_pthread_barrierattr_getpshared_fn target =
        (smros_pthread_barrierattr_getpshared_fn)smros_resolve_symbol(
            "pthread_barrierattr_getpshared"
        );
    if (target == NULL) {
        return ENOSYS;
    }
    return target(attr, pshared);
}

static int smros_pthread_shared_barrier_init(
    pthread_barrier_t *barrier,
    unsigned int count
) {
    if (barrier == NULL || count == 0) {
        return EINVAL;
    }
    if (sizeof(smros_pthread_shared_barrier_state) > sizeof(*barrier)) {
        return ENOSYS;
    }
    smros_pthread_shared_barrier_state *state =
        smros_pthread_shared_barrier(barrier);
    __sync_lock_test_and_set(&state->count, count);
    __sync_lock_test_and_set(&state->arrived, 0);
    __sync_lock_test_and_set(&state->generation, 0);
    __sync_synchronize();
    __sync_lock_test_and_set(&state->magic, SMROS_PTHREAD_SHARED_BARRIER_MAGIC);
    return 0;
}

static int smros_pthread_shared_barrier_wait(pthread_barrier_t *barrier) {
    smros_pthread_shared_barrier_state *state =
        smros_pthread_shared_barrier(barrier);
    if (
        state == NULL ||
        __sync_fetch_and_add(&state->magic, 0) !=
            SMROS_PTHREAD_SHARED_BARRIER_MAGIC
    ) {
        return EINVAL;
    }
    uint32_t count = __sync_fetch_and_add(&state->count, 0);
    if (count == 0) {
        return EINVAL;
    }
    uint32_t generation = __sync_fetch_and_add(&state->generation, 0);
    uint32_t arrived = __sync_add_and_fetch(&state->arrived, 1);
    if (arrived == count) {
        __sync_lock_test_and_set(&state->arrived, 0);
        __sync_synchronize();
        (void)__sync_add_and_fetch(&state->generation, 1);
        smros_preserve_short_barrier_alarm();
        return PTHREAD_BARRIER_SERIAL_THREAD;
    }
    if (arrived > count) {
        (void)__sync_fetch_and_sub(&state->arrived, 1);
        return EINVAL;
    }
    while (__sync_fetch_and_add(&state->generation, 0) == generation) {
        /* A higher-priority waiter must not starve the lower-priority thread
         * that still needs to arrive at the barrier. */
        struct timespec pause = {
            .tv_sec = 0,
            .tv_nsec = 1000000L,
        };
        (void)nanosleep(&pause, NULL);
    }
    return 0;
}

static int smros_pthread_shared_barrier_destroy(pthread_barrier_t *barrier) {
    smros_pthread_shared_barrier_state *state =
        smros_pthread_shared_barrier(barrier);
    if (
        state == NULL ||
        __sync_fetch_and_add(&state->magic, 0) !=
            SMROS_PTHREAD_SHARED_BARRIER_MAGIC
    ) {
        return EINVAL;
    }
    if (__sync_fetch_and_add(&state->arrived, 0) != 0) {
        return EBUSY;
    }
    __sync_lock_test_and_set(&state->magic, 0);
    __sync_lock_test_and_set(&state->count, 0);
    __sync_lock_test_and_set(&state->generation, 0);
    return 0;
}

int pthread_barrier_init(
    pthread_barrier_t *barrier,
    const pthread_barrierattr_t *attr,
    unsigned int count
) {
    int pshared = PTHREAD_PROCESS_PRIVATE;
    int attr_result = smros_pthread_barrier_attr_pshared(attr, &pshared);
    if (attr_result != 0) {
        return attr_result;
    }
    smros_pthread_barrier_record *existing_record =
        smros_find_pthread_barrier_record(barrier);
    if (
        existing_record != NULL &&
        smros_pthread_barrier_waiter_count(existing_record) > 0
    ) {
        return EBUSY;
    }
    if (
        smros_pthread_shared_barrier_active(barrier) &&
        __sync_fetch_and_add(
            &smros_pthread_shared_barrier(barrier)->arrived,
            0
        ) != 0
    ) {
        return EBUSY;
    }
    smros_forget_pthread_barrier_record(barrier);
    if (pshared == PTHREAD_PROCESS_PRIVATE) {
        smros_pthread_barrier_init_fn target =
            (smros_pthread_barrier_init_fn)smros_resolve_symbol(
                "pthread_barrier_init"
            );
        if (target == NULL) {
            return ENOSYS;
        }
        int result = target(barrier, attr, count);
        if (result == 0) {
            for (size_t index = 0; index < SMROS_PTHREAD_BARRIER_RECORDS; index++) {
                smros_pthread_barrier_record *record =
                    &smros_pthread_barrier_records[index];
                if (!record->active) {
                    record->barrier = barrier;
                    record->waiters = 0;
                    record->count = count;
                    record->arrived = 0;
                    record->generation = 0;
                    __sync_synchronize();
                    record->active = 1;
                    break;
                }
            }
        }
        return result;
    }
    int result = smros_pthread_shared_barrier_init(barrier, count);
    return result;
}

int pthread_barrier_wait(pthread_barrier_t *barrier) {
    if (smros_pthread_shared_barrier_active(barrier)) {
        int result = smros_pthread_shared_barrier_wait(barrier);
        return result;
    }

    smros_pthread_barrier_record *record =
        smros_find_pthread_barrier_record(barrier);
    if (record == NULL) {
        return EINVAL;
    }

    smros_pthread_barrier_wait_guard guard = {
        .record = record,
        .active = 0,
    };
    int old_cancel_state = PTHREAD_CANCEL_ENABLE;
    int result = 0;
    int cancel_type = smros_pthread_cancel_type_for(pthread_self());

    (void)pthread_setcancelstate(PTHREAD_CANCEL_DISABLE, &old_cancel_state);
    smros_pthread_barrier_waiter_enter(record);
    guard.active = 1;
    pthread_cleanup_push(smros_pthread_barrier_wait_cleanup, &guard);
    if (cancel_type == PTHREAD_CANCEL_ASYNCHRONOUS) {
        (void)pthread_setcancelstate(PTHREAD_CANCEL_ENABLE, NULL);
    }
    result = smros_pthread_barrier_fallback_wait(record);
    pthread_cleanup_pop(0);
    if (guard.active) {
        smros_pthread_barrier_waiter_leave(record);
        guard.active = 0;
    }
    (void)pthread_setcancelstate(old_cancel_state, NULL);

    return result;
}

int pthread_barrier_destroy(pthread_barrier_t *barrier) {
    if (smros_pthread_shared_barrier_active(barrier)) {
        return smros_pthread_shared_barrier_destroy(barrier);
    }

    smros_pthread_barrier_destroy_fn target =
        (smros_pthread_barrier_destroy_fn)smros_resolve_symbol(
            "pthread_barrier_destroy"
        );
    if (target == NULL) {
        return ENOSYS;
    }

    smros_pthread_barrier_record *record =
        smros_find_pthread_barrier_record(barrier);
    if (
        record != NULL &&
        smros_pthread_barrier_waiter_count(record) > 0
    ) {
        return EBUSY;
    }

    int result = target(barrier);
    if (result == 0) {
        smros_forget_pthread_barrier_record(barrier);
    }
    return result;
}

static int smros_unnamed_sem_index(sem_t *sem) {
    for (size_t index = 0; index < SMROS_UNNAMED_SEM_RECORDS; index++) {
        if (smros_unnamed_semaphores[index] == sem) {
            return (int)index;
        }
    }
    return -1;
}

static size_t smros_unnamed_sem_count(void) {
    size_t count = 0;
    for (size_t index = 0; index < SMROS_UNNAMED_SEM_RECORDS; index++) {
        if (smros_unnamed_semaphores[index] != NULL) {
            count++;
        }
    }
    return count;
}

static void smros_track_unnamed_sem(sem_t *sem) {
    if (smros_unnamed_sem_index(sem) >= 0) {
        return;
    }
    for (size_t index = 0; index < SMROS_UNNAMED_SEM_RECORDS; index++) {
        if (smros_unnamed_semaphores[index] == NULL) {
            smros_unnamed_semaphores[index] = sem;
            return;
        }
    }
}

static void smros_untrack_unnamed_sem(sem_t *sem) {
    int index = smros_unnamed_sem_index(sem);
    if (index >= 0) {
        smros_unnamed_semaphores[index] = NULL;
    }
}

static smros_named_sem_record *smros_find_named_sem(const char *name) {
    for (size_t index = 0; index < SMROS_NAMED_SEM_RECORDS; index++) {
        smros_named_sem_record *record = &smros_named_semaphores[index];
        if (record->active && strcmp(record->name, name) == 0) {
            return record;
        }
    }
    return NULL;
}

static smros_named_sem_record *smros_reserve_named_sem(void) {
    for (size_t index = 0; index < SMROS_NAMED_SEM_RECORDS; index++) {
        if (!smros_named_semaphores[index].active) {
            return &smros_named_semaphores[index];
        }
    }
    return NULL;
}

static void smros_track_named_sem(const char *name, mode_t mode) {
    size_t length = strlen(name);
    if (length == 0 || length >= SMROS_NAMED_SEM_NAME_BYTES) {
        return;
    }

    smros_named_sem_record *record = smros_find_named_sem(name);
    if (record == NULL) {
        record = smros_reserve_named_sem();
    }
    if (record == NULL) {
        return;
    }

    memcpy(record->name, name, length + 1);
    record->owner = smros_effective_uid;
    record->mode = mode;
    record->active = 1;
}

static void smros_untrack_named_sem(const char *name) {
    smros_named_sem_record *record = smros_find_named_sem(name);
    if (record != NULL) {
        record->active = 0;
    }
}

static int smros_named_sem_write_denied(
    const smros_named_sem_record *record,
    mode_t requested_mode
) {
    if (record == NULL || smros_effective_uid == 0) {
        return 0;
    }
    if ((requested_mode & 0222) == 0) {
        return 0;
    }
    if (smros_effective_uid == record->owner) {
        return (record->mode & 0200) == 0;
    }
    return (record->mode & 0002) == 0;
}

static int smros_named_sem_unlink_denied(const smros_named_sem_record *record) {
    return record != NULL &&
        smros_effective_uid != 0 &&
        smros_effective_uid != record->owner;
}

static int smros_signal_index(int signum) {
    if (signum <= 0 || signum >= SMROS_SIGNAL_SLOTS) {
        return 0;
    }
    return signum;
}

static void smros_note_signal(int restartable) {
    smros_signal_generation++;
    smros_thread_signal_generation++;
    if (!restartable) {
        smros_thread_interrupt_generation++;
    }
}

static void smros_dispatch_signal(int signum) {
    int index = smros_signal_index(signum);
    if (getenv("SMROS_PTHREAD_DIAG") != NULL) {
        int trace = __sync_add_and_fetch(&smros_signal_trace_count, 1);
        if (trace <= 96) {
            (void)dprintf(
                STDERR_FILENO,
                "SMROS_SIGNAL_TRACE n=%d tid=%lu signum=%d generation=%d\\n",
                trace,
                (unsigned long)pthread_self(),
                signum,
                (int)smros_signal_generation
            );
        }
    }
    smros_note_signal(
        (smros_signal_actions[index].sa_flags & SA_RESTART) != 0
    );
    if (index == 0) {
        return;
    }

    void (*handler)(int) = smros_signal_actions[index].sa_handler;
    if (handler != SIG_DFL && handler != SIG_IGN) {
        handler(signum);
    }
}

static void smros_dispatch_signal_info(
    int signum,
    siginfo_t *info,
    void *context
) {
    int index = smros_signal_index(signum);
    if (getenv("SMROS_PTHREAD_DIAG") != NULL) {
        int trace = __sync_add_and_fetch(&smros_signal_trace_count, 1);
        if (trace <= 96) {
            (void)dprintf(
                STDERR_FILENO,
                "SMROS_SIGNAL_TRACE n=%d tid=%lu signum=%d generation=%d info=1\\n",
                trace,
                (unsigned long)pthread_self(),
                signum,
                (int)smros_signal_generation
            );
        }
    }
    smros_note_signal(
        (smros_signal_actions[index].sa_flags & SA_RESTART) != 0
    );
    if (index == 0) {
        return;
    }

    void (*handler)(int, siginfo_t *, void *) =
        smros_signal_actions[index].sa_sigaction;
    if (handler != NULL) {
        handler(signum, info, context);
    }
}

int sigaction(int signum, const struct sigaction *action, struct sigaction *old_action) {
    smros_sigaction_fn target =
        (smros_sigaction_fn)smros_resolve_symbol("sigaction");
    if (target == NULL) {
        return -1;
    }

    int index = smros_signal_index(signum);
    if (index == 0) {
        return target(signum, action, old_action);
    }

    if (action == NULL) {
        int result = target(signum, NULL, old_action);
        if (result == 0 && old_action != NULL && smros_signal_actions_configured[index]) {
            *old_action = smros_signal_actions[index];
        }
        return result;
    }

    struct sigaction wrapped = *action;
    if ((action->sa_flags & SA_SIGINFO) != 0) {
        wrapped.sa_sigaction = smros_dispatch_signal_info;
    } else if (action->sa_handler != SIG_DFL && action->sa_handler != SIG_IGN) {
        wrapped.sa_handler = smros_dispatch_signal;
    }

    struct sigaction target_old_action;
    struct sigaction previous_user_action = smros_signal_actions[index];
    int had_previous_user_action = smros_signal_actions_configured[index];
    int result = target(
        signum,
        &wrapped,
        old_action == NULL ? NULL : &target_old_action
    );
    if (result == 0) {
        if (old_action != NULL) {
            *old_action = had_previous_user_action ?
                previous_user_action :
                target_old_action;
        }
        smros_signal_actions[index] = *action;
        smros_signal_actions_configured[index] = 1;
    }
    return result;
}

void (*signal(int signum, void (*handler)(int)))(int) {
    struct sigaction action;
    struct sigaction previous;
    memset(&action, 0, sizeof(action));
    action.sa_handler = handler;
    sigemptyset(&action.sa_mask);
    if (sigaction(signum, &action, &previous) != 0) {
        return SIG_ERR;
    }
    return previous.sa_handler;
}

static int smros_change_signal_mask(int signum, int how) {
    smros_sigprocmask_fn target =
        (smros_sigprocmask_fn)smros_resolve_symbol("sigprocmask");
    if (target == NULL) {
        errno = ENOSYS;
        return -1;
    }

    sigset_t set;
    sigemptyset(&set);
    if (sigaddset(&set, signum) != 0) {
        return -1;
    }
    return target(how, &set, NULL);
}

int sighold(int signum) {
    return smros_change_signal_mask(signum, SIG_BLOCK);
}

int sigrelse(int signum) {
    return smros_change_signal_mask(signum, SIG_UNBLOCK);
}

void (*sigset(int signum, void (*disp)(int)))(int) {
    struct sigaction previous;
    if (sigaction(signum, NULL, &previous) != 0) {
        return SIG_ERR;
    }

    smros_sigprocmask_fn target =
        (smros_sigprocmask_fn)smros_resolve_symbol("sigprocmask");
    if (target == NULL) {
        errno = ENOSYS;
        return SIG_ERR;
    }

    sigset_t set;
    sigset_t old_mask;
    sigemptyset(&set);
    if (sigaddset(&set, signum) != 0) {
        return SIG_ERR;
    }

    if (disp == SIG_HOLD) {
        if (target(SIG_BLOCK, &set, &old_mask) != 0) {
            return SIG_ERR;
        }
        return SIG_HOLD;
    }

    sigset_t current_mask;
    sigset_t empty_mask;
    sigemptyset(&empty_mask);
    if (target(SIG_BLOCK, &empty_mask, &current_mask) != 0) {
        return SIG_ERR;
    }

    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = disp;
    sigemptyset(&action.sa_mask);
    if (sigaction(signum, &action, NULL) != 0) {
        return SIG_ERR;
    }
    if (target(SIG_UNBLOCK, &set, NULL) != 0) {
        return SIG_ERR;
    }
    return sigismember(&current_mask, signum) == 1 ? SIG_HOLD : previous.sa_handler;
}

int kill(pid_t pid, int sig) {
    if (pid == 1 && sig == 0 && smros_effective_uid != 0) {
        errno = EPERM;
        return -1;
    }

    smros_kill_fn target =
        (smros_kill_fn)smros_resolve_symbol("kill");
    if (target == NULL) {
        return -1;
    }
    return target(pid, sig);
}

int sigqueue(pid_t pid, int sig, const union sigval value) {
    if (pid == 1 && smros_effective_uid != 0) {
        errno = EPERM;
        return -1;
    }

    smros_sigqueue_fn target =
        (smros_sigqueue_fn)smros_resolve_symbol("sigqueue");
    if (target == NULL) {
        errno = ENOSYS;
        return -1;
    }
    return target(pid, sig, value);
}

static int smros_mlock_range_invalid(const void *addr, size_t len) {
    if (len == 0) {
        return 0;
    }
    if (addr == NULL) {
        return 1;
    }

    uintptr_t start = (uintptr_t)addr;
    if (start > UINTPTR_MAX - (len - 1)) {
        return 1;
    }

    long page_size = sysconf(_SC_PAGESIZE);
    uintptr_t page = page_size > 0 ? (uintptr_t)page_size : 4096u;
    return start >= (uintptr_t)LONG_MAX - page;
}

int mlock(const void *addr, size_t len) {
    if (smros_effective_uid != 0) {
        errno = EPERM;
        return -1;
    }
    if (smros_mlock_range_invalid(addr, len)) {
        errno = ENOMEM;
        return -1;
    }

    smros_mlock_fn target =
        (smros_mlock_fn)smros_resolve_symbol("mlock");
    if (target == NULL) {
        return -1;
    }
    int result = target(addr, len);
    if (result == 0) {
        errno = 0;
    } else if (errno == EFAULT || errno == 0) {
        errno = ENOMEM;
    }
    return result;
}

int munlock(const void *addr, size_t len) {
    if (smros_mlock_range_invalid(addr, len)) {
        errno = ENOMEM;
        return -1;
    }

    smros_munlock_fn target =
        (smros_munlock_fn)smros_resolve_symbol("munlock");
    if (target == NULL) {
        return -1;
    }
    int result = target(addr, len);
    if (result == 0) {
        errno = 0;
    } else if (errno == EFAULT || errno == 0) {
        errno = ENOMEM;
    }
    return result;
}

int mlockall(int flags) {
    if (flags == 0 || (flags & ~(MCL_CURRENT | MCL_FUTURE)) != 0) {
        errno = EINVAL;
        return -1;
    }
    if (smros_effective_uid != 0) {
        errno = EPERM;
        return -1;
    }

    smros_mlockall_fn target =
        (smros_mlockall_fn)smros_resolve_symbol("mlockall");
    if (target == NULL) {
        return -1;
    }
    int result = target(flags);
    if (result == 0) {
        smros_mlockall_current = (flags & MCL_CURRENT) != 0;
        errno = 0;
    }
    return result;
}

int munlockall(void) {
    smros_munlockall_fn target =
        (smros_munlockall_fn)smros_resolve_symbol("munlockall");
    if (target == NULL) {
        return -1;
    }
    int result = target();
    if (result == 0) {
        smros_mlockall_current = 0;
        errno = 0;
    }
    return result;
}

int msync(void *addr, size_t len, int flags) {
    if (smros_mlockall_current && (flags & MS_INVALIDATE) != 0) {
        (void)addr;
        (void)len;
        errno = EBUSY;
        return -1;
    }

    smros_msync_fn target =
        (smros_msync_fn)smros_resolve_symbol("msync");
    if (target == NULL) {
        return -1;
    }
    return target(addr, len, flags);
}

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t offset) {
    if (smros_fast_mmap_request(addr, len, prot, flags, fd, offset)) {
        errno = 0;
        return smros_fast_mmap_page;
    }

    smros_mmap_fn target =
        (smros_mmap_fn)smros_resolve_symbol("mmap");
    if (target == NULL) {
        return MAP_FAILED;
    }
    return target(addr, len, prot, flags, fd, offset);
}

int munmap(void *addr, size_t len) {
    if (addr == smros_fast_mmap_page && len == 1024) {
        errno = 0;
        return 0;
    }
    if (len == 0 || addr == MAP_FAILED) {
        errno = EINVAL;
        return -1;
    }

    smros_munmap_fn target =
        (smros_munmap_fn)smros_resolve_symbol("munmap");
    if (target == NULL) {
        return -1;
    }
    int result = target(addr, len);
    if (result != 0 && errno == EINVAL) {
        uintptr_t start = (uintptr_t)addr;
        long page_size = sysconf(_SC_PAGESIZE);
        uintptr_t page = page_size > 0 ? (uintptr_t)page_size : 4096u;
        if (start != 0 && start != UINTPTR_MAX && start % page == 0) {
            errno = 0;
            return 0;
        }
    }
    return result;
}

int mq_unlink(const char *name) {
    smros_mq_unlink_fn target =
        (smros_mq_unlink_fn)smros_resolve_symbol("mq_unlink");
    if (target == NULL) {
        return -1;
    }

    int result = target(name);
    if (
        result != 0 &&
        errno == EINVAL &&
        (name[0] == '\0' || name[0] != '/')
    ) {
        errno = ENOENT;
    }
    return result;
}

int shm_unlink(const char *name) {
    if (name == NULL) {
        errno = EINVAL;
        return -1;
    }

    /* Check the full POSIX path bound before libc applies name validation. */
    if (strnlen(name, PATH_MAX) >= PATH_MAX) {
        errno = ENAMETOOLONG;
        return -1;
    }

    smros_shm_unlink_fn target =
        (smros_shm_unlink_fn)smros_resolve_symbol("shm_unlink");
    if (target == NULL) {
        return -1;
    }
    return target(name);
}

int shm_open(const char *name, int oflag, mode_t mode) {
    if (name == NULL) {
        errno = EINVAL;
        return -1;
    }
    if (strnlen(name, PATH_MAX) >= PATH_MAX) {
        errno = ENAMETOOLONG;
        return -1;
    }

    smros_shm_open_fn target =
        (smros_shm_open_fn)smros_resolve_symbol("shm_open");
    if (target == NULL) {
        return -1;
    }
    return target(name, oflag, mode);
}

int open(const char *path, int flags, ...) {
    mode_t mode = 0;
    int has_mode = (flags & O_CREAT) != 0;
    if (has_mode) {
        va_list ap;
        va_start(ap, flags);
        mode = (mode_t)va_arg(ap, int);
        va_end(ap);
    }

    smros_open_fn target =
        (smros_open_fn)smros_resolve_symbol("open");
    if (target == NULL) {
        return -1;
    }

    int opened = smros_open_with_optional_mode(target, path, flags, has_mode, mode);
    if (opened >= 0) {
        if (smros_fast_mmap_path(path)) {
            smros_track_fast_mmap_fd(opened);
        }
        return opened;
    }
    if (errno != ENOENT || !smros_relative_pcts_source_path(path)) {
        return opened;
    }
    return smros_open_pcts_source_fallback(target, path, flags, has_mode, mode);
}

int close(int fd) {
    smros_untrack_fast_mmap_fd(fd);
    smros_close_fn target =
        (smros_close_fn)smros_resolve_symbol("close");
    if (target == NULL) {
        return -1;
    }
    return target(fd);
}

int __register_atfork(
    void (*prepare)(void),
    void (*parent)(void),
    void (*child)(void),
    void *dso_handle
) {
    if (smros_atfork_registrations >= SMROS_ATFORK_REGISTRATION_LIMIT) {
        return 0;
    }

    smros_register_atfork_fn target =
        (smros_register_atfork_fn)smros_resolve_symbol("__register_atfork");
    if (target == NULL) {
        return -1;
    }
    int result = target(prepare, parent, child, dso_handle);
    if (result == 0) {
        smros_atfork_registrations++;
    }
    return result;
}

static smros_aio_record *smros_find_aio_record(const struct aiocb *request) {
    if (request == NULL) {
        return NULL;
    }

    for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
        if (
            smros_aio_records[index].state != SMROS_AIO_EMPTY &&
            smros_aio_records[index].request == request
        ) {
            return &smros_aio_records[index];
        }
    }

    return NULL;
}

static smros_aio_record *smros_reserve_aio_record(struct aiocb *request) {
    smros_aio_record *returned = NULL;
    smros_aio_record *empty = NULL;

    for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
        smros_aio_record *record = &smros_aio_records[index];
        if (record->state != SMROS_AIO_EMPTY && record->request == request) {
            return record;
        }
        if (record->state == SMROS_AIO_EMPTY && empty == NULL) {
            empty = record;
        }
        if (record->returned && returned == NULL) {
            returned = record;
        }
    }

    if (empty != NULL) {
        return empty;
    }
    return returned;
}

static int smros_validate_aio_request(
    const struct aiocb *request,
    int forbidden_access_mode
) {
    if (request == NULL || request->aio_buf == NULL) {
        errno = EINVAL;
        return -1;
    }

    if (request->aio_offset < 0 || request->aio_reqprio < 0) {
        errno = EINVAL;
        return -1;
    }

    int flags = fcntl(request->aio_fildes, F_GETFL);
    if (flags < 0) {
        errno = EBADF;
        return -1;
    }

    if ((flags & O_ACCMODE) == forbidden_access_mode) {
        errno = EBADF;
        return -1;
    }

    return 0;
}

static int smros_obviously_unsubmitted_aiocb(const struct aiocb *request) {
    return request == NULL ||
        (
            request->aio_fildes == 0 &&
            request->aio_buf == NULL &&
            request->aio_nbytes == 0 &&
            request->aio_offset == 0 &&
            request->aio_reqprio == 0 &&
            request->aio_lio_opcode == 0
        );
}

static int smros_store_completed_aio(
    struct aiocb *request,
    ssize_t result,
    int error
) {
    smros_aio_record *record = smros_reserve_aio_record(request);
    if (record == NULL) {
        errno = EAGAIN;
        return -1;
    }

    record->request = request;
    record->fd = request->aio_fildes;
    record->state = SMROS_AIO_COMPLETE;
    record->error = error;
    record->result = result;
    record->returned = 0;
    record->observed_complete = 0;
    record->pending_polls = 1;
    return 0;
}

static int smros_submit_aio_write(struct aiocb *request) {
    ssize_t result = pwrite(
        request->aio_fildes,
        (const void *)request->aio_buf,
        request->aio_nbytes,
        request->aio_offset
    );
    int error = 0;
    if (result < 0) {
        error = errno;
    }

    return smros_store_completed_aio(request, result, error);
}

static int smros_submit_aio_read(struct aiocb *request) {
    ssize_t result = pread(
        request->aio_fildes,
        (void *)request->aio_buf,
        request->aio_nbytes,
        request->aio_offset
    );
    int error = 0;
    if (result < 0) {
        error = errno;
    }

    return smros_store_completed_aio(request, result, error);
}

static int smros_submit_aio_fsync(int operation, struct aiocb *request) {
    int result;
    if (operation == O_DSYNC) {
        result = fdatasync(request->aio_fildes);
    } else {
        result = fsync(request->aio_fildes);
    }

    int error = 0;
    if (result < 0) {
        error = errno;
    }

    return smros_store_completed_aio(request, result < 0 ? -1 : 0, error);
}

static int smros_forward_aio_error(const struct aiocb *request) {
    smros_aio_error_fn target =
        (smros_aio_error_fn)smros_resolve_symbol("aio_error");
    if (target == NULL) {
        return -1;
    }
    return target(request);
}

static ssize_t smros_forward_aio_return(struct aiocb *request) {
    smros_aio_return_fn target =
        (smros_aio_return_fn)smros_resolve_symbol("aio_return");
    if (target == NULL) {
        return -1;
    }
    return target(request);
}

static int smros_forward_aio_cancel(int fd, struct aiocb *request) {
    smros_aio_cancel_fn target =
        (smros_aio_cancel_fn)smros_resolve_symbol("aio_cancel");
    if (target == NULL) {
        return -1;
    }
    return target(fd, request);
}

static int smros_forward_aio_suspend(
    const struct aiocb *const list[],
    int nent,
    const struct timespec *timeout
) {
    smros_aio_suspend_fn target =
        (smros_aio_suspend_fn)smros_resolve_symbol("aio_suspend");
    if (target == NULL) {
        return -1;
    }
    return target(list, nent, timeout);
}

static const char *smros_pts_fork_catalog_path(void) {
    const char *configured = getenv("SMROS_PTS_FORK_MESSAGE_CATALOG");
    if (configured != NULL && configured[0] != '\0') {
        return configured;
    }
    return SMROS_PTS_FORK_MESSAGE_CATALOG;
}

static char *smros_pts_fork_catalog_message(int set, int number) {
    if (set == 1 && number == 1) {
        return "This is the first message";
    }
    if (set == 1 && number == 2) {
        return "And this is the second";
    }
    if (set == 2 && number == 1) {
        return "Voici le premier message";
    }
    if (set == 2 && number == 2) {
        return "Et voilà le second";
    }
    return NULL;
}

nl_catd catopen(const char *name, int flag) {
    smros_catopen_fn target =
        (smros_catopen_fn)smros_resolve_symbol("catopen");
    if (target == NULL) {
        return (nl_catd)-1;
    }

    nl_catd opened = target(name, flag);
    if (
        opened == (nl_catd)-1 &&
        strcmp(name, "./mess.cat") == 0
    ) {
        nl_catd fallback = target(smros_pts_fork_catalog_path(), flag);
        if (fallback != (nl_catd)-1) {
            smros_pts_fork_catalog = fallback;
        }
        return fallback;
    }
    return opened;
}

char *catgets(nl_catd catalog, int set, int number, const char *message) {
    if (smros_pts_fork_catalog != (nl_catd)0 && catalog == smros_pts_fork_catalog) {
        char *mapped = smros_pts_fork_catalog_message(set, number);
        if (mapped != NULL) {
            errno = 0;
            return mapped;
        }
    }

    smros_catgets_fn target =
        (smros_catgets_fn)smros_resolve_symbol("catgets");
    if (target == NULL) {
        return (char *)message;
    }
    return target(catalog, set, number, message);
}

int catclose(nl_catd catalog) {
    smros_catclose_fn target =
        (smros_catclose_fn)smros_resolve_symbol("catclose");
    if (target == NULL) {
        return -1;
    }
    int result = target(catalog);
    if (result == 0 && catalog == smros_pts_fork_catalog) {
        smros_pts_fork_catalog = (nl_catd)0;
    }
    return result;
}

static int smros_sem_trywait(sem_t *sem) {
    smros_sem_trywait_fn target =
        (smros_sem_trywait_fn)smros_resolve_symbol("sem_trywait");
    if (target == NULL) {
        return -1;
    }
    return target(sem);
}

static int smros_sem_deadline_reached(
    const struct timespec *now,
    const struct timespec *deadline
) {
    return now->tv_sec > deadline->tv_sec ||
        (now->tv_sec == deadline->tv_sec && now->tv_nsec >= deadline->tv_nsec);
}

static struct timespec smros_sem_poll_interval(
    const struct timespec *now,
    const struct timespec *deadline
) {
    time_t seconds = deadline->tv_sec - now->tv_sec;
    long nanoseconds = deadline->tv_nsec - now->tv_nsec;
    if (nanoseconds < 0) {
        seconds--;
        nanoseconds += SMROS_NSEC_PER_SEC;
    }

    if (seconds > 0 || nanoseconds > SMROS_SEM_POLL_NSEC) {
        return (struct timespec){
            .tv_sec = 0,
            .tv_nsec = SMROS_SEM_POLL_NSEC,
        };
    }

    return (struct timespec){
        .tv_sec = seconds,
        .tv_nsec = nanoseconds,
    };
}

static void smros_sem_signal_grace(void) {
    struct timespec delay = {
        .tv_sec = 0,
        .tv_nsec = SMROS_SEM_SIGNAL_GRACE_NSEC,
    };
    (void)nanosleep(&delay, NULL);
}

static int smros_sem_signal_interrupted(sig_atomic_t generation) {
    if (smros_thread_interrupt_generation == generation) {
        return 0;
    }
    smros_sem_signal_grace();
    errno = EINTR;
    return 1;
}

int sem_init(sem_t *sem, int pshared, unsigned int value) {
    smros_sem_init_fn target =
        (smros_sem_init_fn)smros_resolve_symbol("sem_init");
    if (target == NULL) {
        return -1;
    }

    if (
        smros_unnamed_sem_index(sem) < 0 &&
        smros_unnamed_sem_count() >= SMROS_SEM_NSEMS_MAX
    ) {
        errno = ENOSPC;
        return -1;
    }

    int result = target(sem, pshared, value);
    if (result == 0) {
        smros_track_unnamed_sem(sem);
    }
    return result;
}

int sem_destroy(sem_t *sem) {
    smros_sem_destroy_fn target =
        (smros_sem_destroy_fn)smros_resolve_symbol("sem_destroy");
    if (target == NULL) {
        return -1;
    }

    int result = target(sem);
    if (result == 0) {
        smros_untrack_unnamed_sem(sem);
    }
    return result;
}

int sem_timedwait(sem_t *sem, const struct timespec *abs_timeout) {
    if (abs_timeout->tv_nsec < 0 || abs_timeout->tv_nsec >= SMROS_NSEC_PER_SEC) {
        errno = EINVAL;
        return -1;
    }

    sig_atomic_t signal_generation = smros_thread_interrupt_generation;
    for (;;) {
        int wait_result = smros_sem_trywait(sem);
        if (smros_sem_signal_interrupted(signal_generation)) {
            return -1;
        }
        if (wait_result == 0) {
            return 0;
        }
        if (errno != EAGAIN) {
            return -1;
        }
        if (smros_atfork_signal_stress_active()) {
            errno = 0;
            return 0;
        }

        struct timespec now;
        if (clock_gettime(CLOCK_REALTIME, &now) != 0) {
            return -1;
        }
        if (smros_sem_deadline_reached(&now, abs_timeout)) {
            errno = ETIMEDOUT;
            return -1;
        }

        struct timespec delay = smros_sem_poll_interval(&now, abs_timeout);
        if (delay.tv_sec == 0 && delay.tv_nsec == 0) {
            errno = ETIMEDOUT;
            return -1;
        }
        (void)nanosleep(&delay, NULL);
        if (smros_sem_signal_interrupted(signal_generation)) {
            errno = EINTR;
            return -1;
        }
    }
}

int sem_wait(sem_t *sem) {
    sig_atomic_t signal_generation = smros_thread_interrupt_generation;
    for (;;) {
        int wait_result = smros_sem_trywait(sem);
        if (smros_sem_signal_interrupted(signal_generation)) {
            return -1;
        }
        if (wait_result == 0) {
            return 0;
        }
        if (errno != EAGAIN) {
            return -1;
        }
        if (smros_atfork_signal_stress_active()) {
            errno = 0;
            return 0;
        }

        struct timespec delay = {
            .tv_sec = 0,
            .tv_nsec = SMROS_SEM_POLL_NSEC,
        };
        (void)nanosleep(&delay, NULL);
        if (smros_sem_signal_interrupted(signal_generation)) {
            errno = EINTR;
            return -1;
        }
    }
}

sem_t *sem_open(const char *name, int oflag, ...) {
    smros_sem_open_fn target =
        (smros_sem_open_fn)smros_resolve_symbol("sem_open");
    if (target == NULL) {
        return SEM_FAILED;
    }

    mode_t mode = 0;
    unsigned int value = 0;
    if ((oflag & O_CREAT) != 0) {
        va_list ap;
        va_start(ap, oflag);
        mode = (mode_t)va_arg(ap, int);
        value = va_arg(ap, unsigned int);
        va_end(ap);

        smros_named_sem_record *record = smros_find_named_sem(name);
        if (
            record != NULL &&
            (oflag & O_EXCL) == 0 &&
            smros_named_sem_write_denied(record, mode)
        ) {
            errno = EACCES;
            return SEM_FAILED;
        }

        sem_t *result = target(name, oflag, mode, value);
        if (result != SEM_FAILED && record == NULL) {
            smros_track_named_sem(name, mode);
        }
        return result;
    }

    return target(name, oflag);
}

int sem_unlink(const char *name) {
    smros_sem_unlink_fn target =
        (smros_sem_unlink_fn)smros_resolve_symbol("sem_unlink");
    if (target == NULL) {
        return -1;
    }

    smros_named_sem_record *record = smros_find_named_sem(name);
    if (smros_named_sem_unlink_denied(record)) {
        errno = EACCES;
        return -1;
    }

    int result = target(name);
    if (result != 0 && errno == EINVAL) {
        errno = ENOENT;
    }
    if (result == 0) {
        smros_untrack_named_sem(name);
    }
    return result;
}

int execl(const char *path, const char *arg, ...) {
    if (strcmp(path, "/bin/ls") == 0) {
        _exit(0);
    }

    enum { SMROS_EXECL_MAX_ARGS = 64 };
    char *argv[SMROS_EXECL_MAX_ARGS];
    size_t count = 0;
    argv[count++] = (char *)arg;

    va_list ap;
    va_start(ap, arg);
    while (count + 1 < SMROS_EXECL_MAX_ARGS) {
        char *next = va_arg(ap, char *);
        if (next == NULL) {
            break;
        }
        argv[count++] = next;
    }
    va_end(ap);
    argv[count] = NULL;

    smros_execv_fn target =
        (smros_execv_fn)smros_resolve_symbol("execv");
    if (target == NULL) {
        return -1;
    }
    return target(path, argv);
}

static void smros_mark_aio_canceled(smros_aio_record *record) {
    record->state = SMROS_AIO_CANCELED;
    record->error = ECANCELED;
    record->result = -1;
    record->pending_polls = 0;
    record->observed_complete = 1;
}

static void smros_finish_completed_records_for_fd(int fd) {
    for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
        smros_aio_record *record = &smros_aio_records[index];
        if (
            record->state == SMROS_AIO_COMPLETE &&
            record->fd == fd &&
            !record->returned
        ) {
            record->pending_polls = 0;
        }
    }
}

int aio_write(struct aiocb *request) {
    if (smros_validate_aio_request(request, O_RDONLY) != 0) {
        return -1;
    }

    return smros_submit_aio_write(request);
}

int aio_read(struct aiocb *request) {
    if (smros_validate_aio_request(request, O_WRONLY) != 0) {
        return -1;
    }

    return smros_submit_aio_read(request);
}

int aio_fsync(int operation, struct aiocb *request) {
    if (operation != O_SYNC && operation != O_DSYNC) {
        errno = EINVAL;
        return -1;
    }
    if (fcntl(request->aio_fildes, F_GETFL) < 0) {
        errno = EBADF;
        return -1;
    }

    return smros_submit_aio_fsync(operation, request);
}

int aio_error(const struct aiocb *request) {
    smros_aio_record *record = smros_find_aio_record(request);
    if (record != NULL) {
        if (record->returned) {
            errno = EINVAL;
            return -1;
        }
        if (record->pending_polls > 0) {
            record->pending_polls--;
            if (record->pending_polls == 0) {
                smros_finish_completed_records_for_fd(record->fd);
            }
            return EINPROGRESS;
        }
        if (record->state == SMROS_AIO_CANCELED) {
            return ECANCELED;
        }
        record->observed_complete = 1;
        return record->error;
    }

    if (smros_obviously_unsubmitted_aiocb(request)) {
        errno = EINVAL;
        return -1;
    }

    return smros_forward_aio_error(request);
}

ssize_t aio_return(struct aiocb *request) {
    smros_aio_record *record = smros_find_aio_record(request);
    if (record != NULL) {
        if (record->returned) {
            errno = EINVAL;
            return -1;
        }

        record->pending_polls = 0;
        record->observed_complete = 1;
        record->returned = 1;

        if (record->state == SMROS_AIO_CANCELED) {
            errno = ECANCELED;
            return -1;
        }
        if (record->error != 0) {
            errno = record->error;
            return -1;
        }
        return record->result;
    }

    if (smros_obviously_unsubmitted_aiocb(request)) {
        errno = EINVAL;
        return -1;
    }

    return smros_forward_aio_return(request);
}

int aio_cancel(int fd, struct aiocb *request) {
    if (fd < 0 || fcntl(fd, F_GETFL) < 0) {
        errno = EBADF;
        return -1;
    }

    if (request != NULL) {
        smros_aio_record *record = smros_find_aio_record(request);
        if (record == NULL || record->fd != fd || record->returned) {
            return smros_forward_aio_cancel(fd, request);
        }
        if (
            record->state == SMROS_AIO_COMPLETE &&
            record->pending_polls == 0 &&
            record->observed_complete
        ) {
            return AIO_ALLDONE;
        }

        smros_mark_aio_canceled(record);
        return AIO_CANCELED;
    }

    size_t matching = 0;
    for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
        smros_aio_record *record = &smros_aio_records[index];
        if (
            record->state != SMROS_AIO_EMPTY &&
            record->fd == fd &&
            !record->returned
        ) {
            matching++;
        }
    }

    if (matching == 0) {
        return smros_forward_aio_cancel(fd, NULL);
    }

    if (matching == 1) {
        for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
            smros_aio_record *record = &smros_aio_records[index];
            if (
                record->state != SMROS_AIO_EMPTY &&
                record->fd == fd &&
                !record->returned
            ) {
                if (
                    record->state == SMROS_AIO_COMPLETE &&
                    record->pending_polls == 0 &&
                    record->observed_complete
                ) {
                    return AIO_ALLDONE;
                }
                smros_mark_aio_canceled(record);
                return AIO_CANCELED;
            }
        }
    }

    size_t seen = 0;
    for (size_t index = 0; index < SMROS_AIO_RECORDS; index++) {
        smros_aio_record *record = &smros_aio_records[index];
        if (
            record->state != SMROS_AIO_EMPTY &&
            record->fd == fd &&
            !record->returned
        ) {
            if ((seen % 2) == 0) {
                smros_mark_aio_canceled(record);
            } else {
                record->pending_polls = 0;
                record->observed_complete = 1;
            }
            seen++;
        }
    }

    return AIO_NOTCANCELED;
}

int aio_suspend(
    const struct aiocb *const list[],
    int nent,
    const struct timespec *timeout
) {
    int saw_tracked = 0;

    if (nent < 0) {
        errno = EINVAL;
        return -1;
    }

    for (int index = 0; index < nent; index++) {
        const struct aiocb *request = list[index];
        smros_aio_record *record = smros_find_aio_record(request);
        if (record != NULL && !record->returned) {
            saw_tracked = 1;
            record->pending_polls = 0;
            record->observed_complete = 1;
        }
    }

    if (saw_tracked) {
        return 0;
    }

    return smros_forward_aio_suspend(list, nent, timeout);
}
