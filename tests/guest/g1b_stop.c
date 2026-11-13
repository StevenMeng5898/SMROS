/*
 * Red team: default SIGSTOP must stop the child, not kill it.
 * waitpid(WUNTRACED) raw status must be 0x137f (WIFSTOPPED, WSTOPSIG 19),
 * not 0x13 (WIFSIGNALED). The child is still alive afterwards.
 */
#include "poc_common.h"

int main(void)
{
	volatile unsigned *progress = poc_shared_progress();
	pid_t child = poc_fork_spinning_child(progress);
	unsigned before;
	unsigned after;

	if (kill(child, SIGSTOP) != 0) {
		perror("kill SIGSTOP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, "stop") != 0)
		return 1;

	before = *progress;
	usleep(50000);
	after = *progress;
	if (after != before) {
		printf("FAIL child kept running while stopped (%u -> %u)\n", before, after);
		return 1;
	}
	errno = 0;
	if (kill(child, 0) != 0) {
		printf("FAIL child is gone errno=%d\n", errno);
		return 1;
	}
	printf("PASS\n");
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	return 0;
}
