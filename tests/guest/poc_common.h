#ifndef POC_COMMON_H
#define POC_COMMON_H

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>

/*
 * Shared helpers for the red-team SIGSTOP probes.
 * A stopped child must stay alive. SIG_DFL SIGSTOP used to kill it, so
 * waitpid(WUNTRACED) returned 0x13 and the following SIGCONT got ESRCH.
 */

static pid_t poc_fork_spinning_child(volatile unsigned *progress)
{
	pid_t child = fork();
	if (child < 0) {
		perror("fork");
		exit(1);
	}
	if (child == 0) {
		for (;;)
			(*progress)++;
		_exit(0);
	}
	return child;
}

static int poc_expect_status(pid_t child, int options, int want, const char *label)
{
	int status = 0;
	pid_t got;

	errno = 0;
	got = waitpid(child, &status, options);
	printf("%s waitpid=%d status=0x%x errno=%d\n", label, (int)got, status, errno);
	if (got != child || status != want) {
		printf("FAIL %s status, want 0x%x\n", label, want);
		return -1;
	}
	return 0;
}

static volatile unsigned *poc_shared_progress(void)
{
	void *page = mmap(NULL, 4096, PROT_READ | PROT_WRITE,
			  MAP_SHARED | MAP_ANONYMOUS, -1, 0);
	if (page == MAP_FAILED) {
		perror("mmap");
		exit(1);
	}
	return page;
}

#endif
