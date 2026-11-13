/*
 * Red team: after a real SIGSTOP, SIGCONT must resume the same child.
 * waitpid(WCONTINUED) raw status must be 0xffff, and the child must run again.
 * The old kernel had already reaped it, so kill(SIGCONT) returned ESRCH.
 */
#include "poc_common.h"

int main(void)
{
	volatile unsigned *progress = poc_shared_progress();
	pid_t child = poc_fork_spinning_child(progress);
	unsigned stopped_at;
	unsigned continued;

	if (kill(child, SIGSTOP) != 0) {
		perror("kill SIGSTOP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, "stop") != 0)
		return 1;

	errno = 0;
	if (kill(child, SIGCONT) != 0) {
		printf("FAIL SIGCONT errno=%d\n", errno);
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, "cont") != 0)
		return 1;

	stopped_at = *progress;
	usleep(50000);
	continued = *progress;
	if (continued == stopped_at) {
		printf("FAIL child did not run after SIGCONT (%u)\n", stopped_at);
		return 1;
	}
	errno = 0;
	if (kill(child, 0) != 0) {
		printf("FAIL child is gone errno=%d\n", errno);
		return 1;
	}
	printf("PASS continued %u -> %u\n", stopped_at, continued);
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	return 0;
}
