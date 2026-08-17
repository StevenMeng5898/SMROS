#define _GNU_SOURCE

#include <aio.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stddef.h>

typedef int (*smros_aio_submit_fn)(struct aiocb *);

static int smros_validate_aio_request(
    const struct aiocb *request,
    int forbidden_access_mode
) {
    if (request == NULL || request->aio_buf == NULL) {
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

static int smros_forward_aio_submit(const char *symbol, struct aiocb *request) {
    void *target = dlsym(RTLD_NEXT, symbol);
    if (target == NULL) {
        errno = ENOSYS;
        return -1;
    }

    return ((smros_aio_submit_fn)target)(request);
}

int aio_write(struct aiocb *request) {
    if (smros_validate_aio_request(request, O_RDONLY) != 0) {
        return -1;
    }

    return smros_forward_aio_submit("aio_write", request);
}

int aio_read(struct aiocb *request) {
    if (smros_validate_aio_request(request, O_WRONLY) != 0) {
        return -1;
    }

    return smros_forward_aio_submit("aio_read", request);
}
