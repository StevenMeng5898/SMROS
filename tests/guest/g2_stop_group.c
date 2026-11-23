/*
 * A default SIGSTOP/SIGCONT sent to a thread group or a process group must
 * stop and resume every thread, not only the one that happened to dequeue it.
 *
 * The supervisor stays outside the target group so it can sample the counters.
 */
#include "poc_common.h"

#include <pthread.h>
#include <time.h>

struct group_state {
	volatile unsigned started[4];
	volatile unsigned spins[4];
	volatile unsigned sleeper_done;
};

static struct group_state *group_state_map(void)
{
	void *page = mmap(NULL, 4096, PROT_READ | PROT_WRITE,
			  MAP_SHARED | MAP_ANONYMOUS, -1, 0);
	if (page == MAP_FAILED) {
		perror("mmap");
		exit(1);
	}
	return page;
}

struct spin_arg {
	struct group_state *state;
	unsigned slot;
};

static void *spin_thread(void *arg)
{
	struct spin_arg *spin = arg;
	struct group_state *state = spin->state;
	unsigned slot = spin->slot;

	state->started[slot] = 1;
	for (;;)
		state->spins[slot]++;
	return NULL;
}

static void *sleep_thread(void *arg)
{
	struct group_state *state = arg;
	struct timespec delay = { .tv_sec = 0, .tv_nsec = 200000000L };

	state->started[3] = 1;
	(void)nanosleep(&delay, NULL);
	state->sleeper_done = 1;
	for (;;)
		state->spins[3]++;
	return NULL;
}

static int wait_until(volatile unsigned *flag, const char *what)
{
	int spins;

	for (spins = 0; spins < 300; spins++) {
		if (*flag != 0)
			return 0;
		/* Block so a same-or-lower priority child can run. */
		usleep(1000);
	}
	printf("FAIL %s did not start\n", what);
	return -1;
}

static int sample_frozen(const struct group_state *state, unsigned *before)
{
	int index;

	for (index = 0; index < 3; index++)
		before[index] = state->spins[index];
	usleep(80000);
	for (index = 0; index < 3; index++) {
		if (state->spins[index] != before[index]) {
			printf("FAIL spin %d moved while stopped (%u -> %u)\n",
			       index, before[index], state->spins[index]);
			return -1;
		}
	}
	if (state->sleeper_done != 0) {
		printf("FAIL sleeper ran during SIGSTOP\n");
		return -1;
	}
	return 0;
}

static int sample_resumed(struct group_state *state, const unsigned *before)
{
	int index;

	usleep(80000);
	for (index = 0; index < 3; index++) {
		if (state->spins[index] == before[index]) {
			printf("FAIL spin %d did not resume (%u)\n", index,
			       before[index]);
			return -1;
		}
	}
	for (index = 0; index < 50 && state->sleeper_done == 0; index++)
		usleep(20000);
	if (state->sleeper_done == 0) {
		printf("FAIL sleeper did not finish after SIGCONT\n");
		return -1;
	}
	if (state->spins[3] == 0) {
		printf("FAIL sleeper did not run after its sleep\n");
		return -1;
	}
	return 0;
}

static int test_thread_group(void)
{
	struct group_state *state = group_state_map();
	unsigned frozen[3];
	struct spin_arg spin_args[2];
	pthread_t first;
	pthread_t second;
	pthread_t sleeper;
	pid_t child;

	child = fork();
	if (child < 0) {
		perror("fork thread group");
		return 1;
	}
	if (child == 0) {
		state->started[2] = 1;
		spin_args[0].state = state;
		spin_args[0].slot = 0;
		spin_args[1].state = state;
		spin_args[1].slot = 1;
		if (pthread_create(&first, NULL, spin_thread, &spin_args[0]) != 0 ||
		    pthread_create(&second, NULL, spin_thread, &spin_args[1]) != 0 ||
		    pthread_create(&sleeper, NULL, sleep_thread, state) != 0) {
			state->sleeper_done = 0xffffu;
			perror("pthread_create");
			_exit(1);
		}
		for (;;)
			state->spins[2]++;
		_exit(0);
	}

	if (wait_until(&state->started[2], "child") != 0 ||
	    wait_until(&state->started[0], "spin 0") != 0 ||
	    wait_until(&state->started[1], "spin 1") != 0 ||
	    wait_until(&state->started[3], "sleeper") != 0)
		return 1;
	if (state->sleeper_done == 0xffffu) {
		printf("FAIL pthread_create\n");
		return 1;
	}
	if (kill(child, SIGSTOP) != 0) {
		perror("kill thread-group SIGSTOP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, "thread-group stop") != 0)
		return 1;
	if (sample_frozen(state, frozen) != 0)
		return 1;
	if (kill(child, SIGCONT) != 0) {
		perror("kill thread-group SIGCONT");
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, "thread-group cont") != 0)
		return 1;
	if (sample_resumed(state, frozen) != 0)
		return 1;
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	printf("PASS thread group stopped and resumed\n");
	return 0;
}

static void spin_process(volatile unsigned *slot, volatile unsigned *started)
{
	*started = 1;
	for (;;)
		(*slot)++;
	_exit(0);
}

static int test_process_group(void)
{
	volatile unsigned *page = poc_shared_progress();
	volatile unsigned *started = page + 2;
	unsigned before[2];
	unsigned after[2];
	pid_t first;
	pid_t second;
	pid_t group;

	first = fork();
	if (first < 0) {
		perror("fork process group");
		return 1;
	}
	if (first == 0) {
		if (setpgid(0, 0) != 0) {
			perror("setpgid leader");
			_exit(1);
		}
		spin_process(&page[0], &started[0]);
	}
	if (setpgid(first, first) != 0 && errno != ESRCH) {
		perror("setpgid parent leader");
		return 1;
	}
	group = first;
	second = fork();
	if (second < 0) {
		perror("fork process group peer");
		return 1;
	}
	if (second == 0) {
		if (setpgid(0, group) != 0) {
			perror("setpgid peer");
			_exit(1);
		}
		spin_process(&page[1], &started[1]);
	}
	if (setpgid(second, group) != 0 && errno != ESRCH) {
		perror("setpgid parent peer");
		return 1;
	}
	if (wait_until(&started[0], "group leader") != 0 ||
	    wait_until(&started[1], "group peer") != 0)
		return 1;

	if (kill(-group, SIGSTOP) != 0) {
		perror("kill process-group SIGSTOP");
		return 1;
	}
	if (poc_expect_status(first, WUNTRACED, 0x137f, "pgroup first stop") != 0)
		return 1;
	if (poc_expect_status(second, WUNTRACED, 0x137f, "pgroup second stop") != 0)
		return 1;
	before[0] = page[0];
	before[1] = page[1];
	usleep(80000);
	after[0] = page[0];
	after[1] = page[1];
	if (after[0] != before[0] || after[1] != before[1]) {
		printf("FAIL process group moved while stopped (%u,%u) -> (%u,%u)\n",
		       before[0], before[1], after[0], after[1]);
		return 1;
	}
	if (kill(-group, SIGCONT) != 0) {
		perror("kill process-group SIGCONT");
		return 1;
	}
	if (poc_expect_status(first, WCONTINUED, 0xffff, "pgroup first cont") != 0)
		return 1;
	if (poc_expect_status(second, WCONTINUED, 0xffff, "pgroup second cont") != 0)
		return 1;
	usleep(80000);
	if (page[0] == before[0] || page[1] == before[1]) {
		printf("FAIL process group did not resume (%u,%u)\n", page[0],
		       page[1]);
		return 1;
	}
	kill(-group, SIGKILL);
	waitpid(first, NULL, 0);
	waitpid(second, NULL, 0);
	printf("PASS process group stopped and resumed\n");
	return 0;
}

int main(void)
{
	if (test_thread_group() != 0)
		return 1;
	if (test_process_group() != 0)
		return 1;
	printf("PASS SIGSTOP broadcast\n");
	return 0;
}
