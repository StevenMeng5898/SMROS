/*
 * F2: this kernel has no Landlock. Linux returns ENOSYS when Landlock is
 * not supported (EOPNOTSUPP is only for a supported Landlock that was
 * disabled at boot). Success would tell the caller it sandboxed itself.
 */
#include "fail_closed.h"

#define __NR_landlock_create_ruleset 444
#define __NR_landlock_add_rule 445
#define __NR_landlock_restrict_self 446

struct landlock_ruleset_attr {
	unsigned long long handled_access_fs;
	unsigned long long handled_access_net;
};

int main(void)
{
	struct landlock_ruleset_attr attr = {
		.handled_access_fs = (1ULL << 0) | (1ULL << 1),
		.handled_access_net = 0,
	};
	long rc;
	int failed = 0;

	errno = 0;
	rc = syscall(__NR_landlock_create_ruleset, &attr, sizeof(attr), 0);
	failed |= expect_errno("landlock_create_ruleset", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_landlock_add_rule, 0, 0, 0, 0);
	failed |= expect_errno("landlock_add_rule", rc, ENOSYS);

	errno = 0;
	rc = syscall(__NR_landlock_restrict_self, 0, 0);
	failed |= expect_errno("landlock_restrict_self", rc, ENOSYS);

	if (failed)
		return 1;
	printf("PASS\n");
	return 0;
}
