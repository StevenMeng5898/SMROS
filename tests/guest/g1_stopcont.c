/*
 * Red-team regression: default SIGSTOP must stop the child, not kill it.
 *
 * kill(child, SIGSTOP) + waitpid(WUNTRACED) must return raw status 0x137f
 * (WIFSTOPPED, WSTOPSIG == SIGSTOP). The old path returned 0x13
 * (WIFSIGNALED, WTERMSIG == 19) and the following kill(SIGCONT) failed
 * with ESRCH. After SIGCONT, waitpid(WCONTINUED) must return 0xffff.
 */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/wait.h>

int main(void)
{
	pid_t child;
	pid_t got;
	int status;

	child = fork();
	if (child < 0) {
		perror("fork");
		return 1;
	}
	if (child == 0) {
		for (;;)
			pause();
	}

	if (kill(child, SIGSTOP) != 0) {
		perror("kill SIGSTOP");
		return 1;
	}
	status = 0;
	got = waitpid(child, &status, WUNTRACED);
	printf("stop waitpid=%d status=0x%x errno=%d\n", got, status, errno);
	if (got != child || status != 0x137f) {
		printf("FAIL stop status, want 0x137f\n");
		return 1;
	}

	errno = 0;
	if (kill(child, SIGCONT) != 0) {
		printf("FAIL SIGCONT errno=%d\n", errno);
		return 1;
	}
	status = 0;
	got = waitpid(child, &status, WCONTINUED);
	printf("cont waitpid=%d status=0x%x errno=%d\n", got, status, errno);
	if (got != child || status != 0xffff) {
		printf("FAIL continue status, want 0xffff\n");
		return 1;
	}

	kill(child, SIGKILL);
	waitpid(child, &status, 0);
	printf("PASS\n");
	return 0;
}
