/*
 * F4: io_uring is not implemented. setup/enter/register return ENOSYS, the
 * same code Linux uses when the syscall slot is not wired. setup must not
 * return fd 0 or leave a caller believing the params were filled in.
 */
#include "fail_closed.h"

#include <string.h>

#define __NR_io_uring_setup 425
#define __NR_io_uring_enter 426
#define __NR_io_uring_register 427

struct io_uring_params {
	unsigned int sq_entries;
	unsigned int cq_entries;
	unsigned int flags;
	unsigned int sq_thread_cpu;
	unsigned int sq_thread_idle;
	unsigned int features;
	unsigned int wq_fd;
	unsigned int resv[3];
	unsigned int sq_off[10];
	unsigned int cq_off[10];
};

int main(void)
{
	struct io_uring_params params;
	unsigned char *raw = (unsigned char *)&params;
	long rc;
	int failed = 0;
	unsigned int i;

	memset(&params, 0xA5, sizeof(params));
	errno = 0;
	rc = syscall(__NR_io_uring_setup, 8, &params);
	failed |= expect_errno("io_uring_setup", rc, ENOSYS);
	for (i = 0; i < sizeof(params); i++) {
		if (raw[i] != 0xA5) {
			printf("FAIL io_uring_setup wrote params at %u\n", i);
			failed = 1;
			break;
		}
	}
	if (!failed)
		printf("ok io_uring params untouched\n");

	errno = 0;
	rc = syscall(__NR_io_uring_enter, 0, 0, 0, 0, NULL, 0);
	failed |= expect_errno("io_uring_enter", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_io_uring_register, 0, 0, NULL, 0);
	failed |= expect_errno("io_uring_register", rc, ENOSYS);

	if (failed)
		return 1;
	printf("PASS\n");
	return 0;
}
