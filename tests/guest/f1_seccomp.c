/*
 * F1: seccomp is not implemented. SET must fail, GET must report mode 0,
 * and PR_SET_NO_NEW_PRIVS must keep its real flag. A successful seccomp
 * install used to leave the process alive while claiming STRICT was on.
 */
#include "fail_closed.h"

#define __NR_seccomp 277
#define __NR_prctl 167
#define __NR_getpid 172
#define PR_GET_SECCOMP 21
#define PR_SET_SECCOMP 22
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define SECCOMP_SET_MODE_STRICT 0
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_MODE_FILTER 2

int main(void)
{
	long rc;
	int failed = 0;

	errno = 0;
	rc = syscall(__NR_seccomp, SECCOMP_SET_MODE_STRICT, 0, 0);
	failed |= expect_errno("seccomp STRICT", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_seccomp, SECCOMP_SET_MODE_FILTER, 0, 0);
	failed |= expect_errno("seccomp FILTER", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_prctl, PR_SET_SECCOMP, 1, 0, 0, 0);
	failed |= expect_errno("prctl PR_SET_SECCOMP", rc, EINVAL);

	errno = 0;
	rc = syscall(__NR_prctl, PR_GET_SECCOMP, 0, 0, 0, 0);
	failed |= expect_value("prctl PR_GET_SECCOMP", rc, 0);

	errno = 0;
	rc = syscall(__NR_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
	failed |= expect_value("prctl PR_SET_NO_NEW_PRIVS", rc, 0);

	errno = 0;
	rc = syscall(__NR_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
	failed |= expect_value("prctl PR_GET_NO_NEW_PRIVS", rc, 1);

	errno = 0;
	rc = syscall(__NR_prctl, PR_SET_SECCOMP, SECCOMP_MODE_FILTER, 0, 0, 0);
	failed |= expect_errno("prctl PR_SET_SECCOMP filter", rc, EINVAL);

	errno = 0;
	rc = syscall(__NR_getpid);
	if (rc <= 0) {
		printf("FAIL getpid rc=%ld errno=%d\n", rc, errno);
		failed = 1;
	} else {
		printf("ok getpid=%ld\n", rc);
	}

	if (failed)
		return 1;
	printf("PASS\n");
	return 0;
}
