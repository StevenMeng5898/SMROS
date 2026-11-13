/*
 * F5: ptrace is refused with EPERM (Linux's "you may not trace" error,
 * including Yama and missing CAP_SYS_PTRACE). process_vm_readv/writev and
 * bpf are absent, so they return ENOSYS rather than a permission error that
 * would imply the operation exists. None of them may return 0.
 */
#include "fail_closed.h"

#define __NR_ptrace 117
#define __NR_process_vm_readv 270
#define __NR_process_vm_writev 271
#define __NR_bpf 280
#define PTRACE_TRACEME 0

struct iovec {
	void *iov_base;
	unsigned long iov_len;
};

int main(void)
{
	char local[8];
	char remote[8];
	struct iovec liov = { local, sizeof(local) };
	struct iovec riov = { remote, sizeof(remote) };
	long rc;
	int failed = 0;

	errno = 0;
	rc = syscall(__NR_ptrace, PTRACE_TRACEME, 0, 0, 0);
	failed |= expect_errno("ptrace TRACEME", rc, EPERM);

	errno = 0;
	rc = syscall(__NR_process_vm_readv, 0, &liov, 1, &riov, 1, 0);
	failed |= expect_errno("process_vm_readv", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_process_vm_writev, 0, &liov, 1, &riov, 1, 0);
	failed |= expect_errno("process_vm_writev", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_bpf, 5, 0, 0);
	failed |= expect_errno("bpf", rc, ENOSYS);

	if (failed)
		return 1;
	printf("PASS\n");
	return 0;
}
