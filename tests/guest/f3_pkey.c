/*
 * F3: protection keys are not implemented. Linux returns ENOSYS from
 * pkey_alloc, pkey_free, and pkey_mprotect when the CPU or kernel has no
 * pkeys. A zero return used to leave the page writable while reporting
 * PROT_READ.
 */
#include "fail_closed.h"

#include <sys/mman.h>

#define __NR_pkey_mprotect 288
#define __NR_pkey_alloc 289
#define __NR_pkey_free 290

int main(void)
{
	long key;
	void *page;
	volatile char *p;
	long rc;
	int failed = 0;

	errno = 0;
	key = syscall(__NR_pkey_alloc, 0, 0);
	failed |= expect_errno("pkey_alloc", key, ENOSYS);

	errno = 0;
	rc = syscall(__NR_pkey_free, 0);
	failed |= expect_errno("pkey_free", rc, ENOSYS);

	page = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
		    -1, 0);
	if (page == MAP_FAILED) {
		printf("FAIL mmap errno=%d\n", errno);
		return 1;
	}
	p = page;
	p[0] = 'A';

	errno = 0;
	rc = syscall(__NR_pkey_mprotect, page, 4096, PROT_READ, 1);
	failed |= expect_errno("pkey_mprotect", rc, ENOSYS);
	p[0] = 'B';
	if (p[0] != 'B') {
		printf("FAIL page changed without a successful protect\n");
		failed = 1;
	} else {
		printf("ok page still writable after rejected pkey_mprotect\n");
	}

	if (failed)
		return 1;
	printf("PASS\n");
	return 0;
}
