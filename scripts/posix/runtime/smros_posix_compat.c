#define _GNU_SOURCE

#include <aio.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stddef.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define SMROS_AIO_RECORDS 2048

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

static smros_aio_record smros_aio_records[SMROS_AIO_RECORDS];

static void *smros_resolve_symbol(const char *symbol) {
    void *target = dlsym(RTLD_NEXT, symbol);
    if (target == NULL) {
        errno = ENOSYS;
    }
    return target;
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
