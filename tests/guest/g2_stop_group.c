/*
 * A default SIGSTOP/SIGCONT sent to a thread group or a process group must
 * stop and resume every thread, not only the one that happened to dequeue it.
 *
 * The supervisor stays outside the target group so it can sample the counters.
 */
#include "poc_common.h"

#include <pthread.h>
#include <sched.h>
#include <string.h>
#include <sys/syscall.h>
#include <time.h>

static int expect_no_further_stop(pid_t child, const char *label);

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
	if (expect_no_further_stop(child, "thread-group second wait") != 0)
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
	if (expect_no_further_stop(first, "pgroup first second wait") != 0 ||
	    expect_no_further_stop(second, "pgroup second second wait") != 0)
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

static int expect_no_further_stop(pid_t child, const char *label)
{
	int status = -1;
	pid_t got;

	errno = 0;
	got = waitpid(child, &status, WNOHANG | WUNTRACED);
	if (got != 0) {
		printf("FAIL %s waitpid=%d status=0x%x errno=%d\n", label,
		       (int)got, status, errno);
		return -1;
	}
	return 0;
}

static pid_t fork_spinner(volatile unsigned *slot, volatile unsigned *started)
{
	pid_t child = fork();

	if (child < 0) {
		perror("fork spinner");
		exit(1);
	}
	if (child == 0)
		spin_process(slot, started);
	return child;
}

static int test_repeat_stop_and_kill(void)
{
	volatile unsigned *page = poc_shared_progress();
	pid_t child;
	int status = 0;

	child = fork_spinner(&page[0], &page[1]);
	if (wait_until(&page[1], "repeat spinner") != 0)
		return 1;
	if (kill(child, SIGSTOP) != 0) {
		perror("repeat SIGSTOP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, "repeat first stop") != 0)
		return 1;
	if (kill(child, SIGSTOP) != 0) {
		perror("repeat second SIGSTOP");
		return 1;
	}
	if (expect_no_further_stop(child, "STOP/STOP") != 0)
		return 1;
	if (kill(child, SIGCONT) != 0) {
		perror("repeat SIGCONT");
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, "STOP/STOP/CONT") != 0)
		return 1;
	if (kill(child, SIGSTOP) != 0) {
		perror("pre-kill SIGSTOP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, "pre-kill stop") != 0)
		return 1;
	if (kill(child, SIGKILL) != 0) {
		perror("SIGKILL stopped group");
		return 1;
	}
	if (waitpid(child, &status, 0) != child || !WIFSIGNALED(status) ||
	    WTERMSIG(status) != SIGKILL) {
		printf("FAIL stopped SIGKILL status=0x%x\n", status);
		return 1;
	}
	printf("PASS repeated SIGSTOP and SIGKILL\n");
	return 0;
}

struct wait_state {
	volatile unsigned started;
	volatile unsigned done;
	volatile int word;
};

static void *futex_waiter(void *arg)
{
	struct wait_state *state = arg;
	struct timespec timeout = { .tv_sec = 0, .tv_nsec = 200000000L };

	state->started = 1;
	syscall(SYS_futex, &state->word, 0, 0, &timeout, NULL, 0);
	state->done = 1;
	return NULL;
}

static void *signal_waiter(void *arg)
{
	struct wait_state *state = arg;
	sigset_t set;
	struct timespec timeout = { .tv_sec = 0, .tv_nsec = 200000000L };

	sigemptyset(&set);
	sigaddset(&set, SIGUSR2);
	state->started = 1;
	(void)sigtimedwait(&set, NULL, &timeout);
	state->done = 1;
	return NULL;
}

static int stop_while_waiting(void *(*worker)(void *), const char *label)
{
	struct wait_state *state = (struct wait_state *)group_state_map();
	pthread_t thread;
	pid_t child;

	child = fork();
	if (child < 0) {
		perror("fork waiter");
		return 1;
	}
	if (child == 0) {
		if (pthread_create(&thread, NULL, worker, state) != 0)
			_exit(1);
		for (;;)
			pause();
	}
	if (wait_until(&state->started, label) != 0)
		return 1;
	if (kill(child, SIGSTOP) != 0) {
		perror("stop waiter");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x137f, label) != 0)
		return 1;
	usleep(250000);
	if (state->done != 0) {
		printf("FAIL %s ran during SIGSTOP\n", label);
		return 1;
	}
	if (kill(child, SIGCONT) != 0) {
		perror("cont waiter");
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, label) != 0)
		return 1;
	if (wait_until(&state->done, label) != 0)
		return 1;
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	printf("PASS %s expired across SIGSTOP\n", label);
	return 0;
}

static int test_blocked_sigtstp(void)
{
	volatile unsigned *page = poc_shared_progress();
	volatile unsigned *phase = &page[0];
	pid_t child;
	sigset_t set;
	sigset_t pending;
	struct timespec slice = { .tv_sec = 1, .tv_nsec = 0 };
	int got;
	int status = 0;

	*phase = 0;
	child = fork();
	if (child < 0) {
		perror("fork sigtstp");
		return 1;
	}
	if (child == 0) {
		sigemptyset(&set);
		sigaddset(&set, SIGTSTP);
		if (sigprocmask(SIG_BLOCK, &set, NULL) != 0)
			_exit(2);
		if (kill(getpid(), SIGTSTP) != 0)
			_exit(3);
		if (sigpending(&pending) != 0 || !sigismember(&pending, SIGTSTP))
			_exit(4);
		got = sigtimedwait(&set, NULL, &slice);
		if (got != SIGTSTP)
			_exit(5);
		phase[1] = 1;
		_exit(0);
	}
	for (got = 0; got < 300 && phase[1] == 0; got++)
		usleep(1000);
	/*
	 * The child exits as soon as sigtimedwait consumes the pending
	 * SIGTSTP. WNOHANG may therefore observe that exit. A stop report
	 * is the failure; a clean exit is the success.
	 */
	got = waitpid(child, &status, WNOHANG | WUNTRACED);
	if (got == child && WIFSTOPPED(status)) {
		printf("FAIL blocked SIGTSTP stopped the child status=0x%x\n",
		       status);
		return 1;
	}
	if (got == 0)
		got = waitpid(child, &status, 0);
	if (got != child || !WIFEXITED(status) || WEXITSTATUS(status) != 0 ||
	    phase[1] == 0) {
		printf("FAIL blocked SIGTSTP child status=0x%x phase=%u\n",
		       status, phase[1]);
		return 1;
	}

	*phase = 0;
	child = fork();
	if (child < 0) {
		perror("fork sigtstp unblock");
		return 1;
	}
	if (child == 0) {
		sigemptyset(&set);
		sigaddset(&set, SIGTSTP);
		if (sigprocmask(SIG_BLOCK, &set, NULL) != 0)
			_exit(2);
		*phase = 1;
		while (*phase == 1)
			;
		if (sigpending(&pending) != 0 || !sigismember(&pending, SIGTSTP))
			_exit(4);
		if (sigprocmask(SIG_UNBLOCK, &set, NULL) != 0)
			_exit(5);
		*phase = 3;
		_exit(0);
	}
	if (wait_until(phase, "sigtstp blocked") != 0)
		return 1;
	if (kill(child, SIGTSTP) != 0) {
		perror("kill SIGTSTP");
		return 1;
	}
	*phase = 2;
	if (poc_expect_status(child, WUNTRACED, 0x147f, "unblock SIGTSTP") != 0)
		return 1;
	if (expect_no_further_stop(child, "unblock SIGTSTP once") != 0)
		return 1;
	if (*phase == 3) {
		printf("FAIL SIGTSTP unblock returned to userspace\n");
		return 1;
	}
	if (kill(child, SIGCONT) != 0) {
		perror("cont SIGTSTP");
		return 1;
	}
	/*
	 * The child exits as soon as SIGCONT resumes it, so the continued
	 * status is not required to survive until wait. Reaching phase 3
	 * is the resume, and the following reap must be a normal exit.
	 */
	for (got = 0; got < 300 && *phase != 3; got++)
		usleep(1000);
	if (*phase != 3) {
		printf("FAIL SIGTSTP did not resume phase=%u\n", *phase);
		return 1;
	}
	if (waitpid(child, &status, 0) != child || !WIFEXITED(status) ||
	    WEXITSTATUS(status) != 0) {
		printf("FAIL SIGTSTP child did not exit status=0x%x\n", status);
		return 1;
	}
	printf("PASS blocked SIGTSTP\n");
	return 0;
}

static int test_exit_sigstop_reaps(void)
{
	int i;

	for (i = 0; i < 40; i++) {
		pid_t child = fork();
		int status = 0;
		int spins = 0;

		if (child < 0) {
			perror("fork exit race");
			return 1;
		}
		if (child == 0)
			_exit(0);
		(void)kill(child, SIGSTOP);
		for (;;) {
			pid_t got = waitpid(child, &status, WNOHANG | WUNTRACED);
			if (got == child && WIFSTOPPED(status)) {
				/*
				 * SIGSTOP won the race and stopped a live
				 * child. That is a normal stop, not an exit
				 * that parked an unreported thread.
				 */
				if (kill(child, SIGCONT) != 0 && errno != ESRCH) {
					printf("FAIL exit SIGSTOP cont child %d\n",
					       (int)child);
					return 1;
				}
				if (waitpid(child, &status, 0) != child) {
					printf("FAIL exit SIGSTOP reap child %d\n",
					       (int)child);
					return 1;
				}
				break;
			}
			if (got == child)
				break;
			if (++spins > 200) {
				printf("FAIL exit SIGSTOP left child %d\n", (int)child);
				return 1;
			}
			usleep(1000);
		}
	}
	printf("PASS exit SIGSTOP reaped\n");
	return 0;
}

static volatile sig_atomic_t handled_signum;
static volatile unsigned *handler_slot;

static void catch_stop(int signum)
{
	handled_signum = signum;
	if (handler_slot != NULL)
		*handler_slot = 2;
}

struct kernel_sigaction {
	void (*handler)(int);
	unsigned long flags;
	void (*restorer)(void);
	unsigned long mask;
};

static int raw_sigaction(int signum, void (*handler)(int))
{
	struct kernel_sigaction action;

	memset(&action, 0, sizeof(action));
	action.handler = handler;
	return syscall(SYS_rt_sigaction, signum, &action, NULL, 8);
}

static int test_caught_sigtstp(void)
{
	volatile unsigned *page = poc_shared_progress();
	pid_t child;
	int status = 0;

	memset((void *)page, 0, 4096);
	handled_signum = 0;
	handler_slot = &page[1];
	child = fork();
	if (child < 0) {
		perror("fork caught SIGTSTP");
		return 1;
	}
	if (child == 0) {
		page[2] = 1;
		if (raw_sigaction(SIGTSTP, catch_stop) != 0)
			_exit(2);
		page[2] = 2;
		page[0] = 1;
		while (handled_signum == 0)
			getpid();
		page[2] = 3;
		_exit(handled_signum == SIGTSTP ? 0 : 3);
	}
	if (wait_until(page, "caught SIGTSTP") != 0)
		return 1;
	if (kill(child, SIGTSTP) != 0) {
		perror("kill caught SIGTSTP");
		return 1;
	}
	if (waitpid(child, &status, 0) != child || !WIFEXITED(status) ||
	    WEXITSTATUS(status) != 0) {
		printf("FAIL handler SIGTSTP status=0x%x slot=%u stage=%u\n", status,
		       page[1], page[2]);
		return 1;
	}
	printf("PASS handler SIGTSTP\n");
	return 0;
}

static int test_ignored_sigtstp(void)
{
	volatile unsigned *page = poc_shared_progress();
	pid_t child;
	int status = 0;
	int got;

	child = fork();
	if (child < 0) {
		perror("fork ignored SIGTSTP");
		return 1;
	}
	if (child == 0) {
		sigset_t pending;

		if (raw_sigaction(SIGTSTP, SIG_IGN) != 0)
			_exit(2);
		if (kill(getpid(), SIGTSTP) != 0)
			_exit(3);
		if (sigpending(&pending) != 0 || sigismember(&pending, SIGTSTP))
			_exit(4);
		page[0] = 1;
		_exit(0);
	}
	for (got = 0; got < 300 && page[0] == 0; got++)
		usleep(1000);
	got = waitpid(child, &status, WNOHANG | WUNTRACED);
	if (got == child && WIFSTOPPED(status)) {
		printf("FAIL ignored SIGTSTP stopped status=0x%x\n", status);
		return 1;
	}
	if (got == 0)
		got = waitpid(child, &status, 0);
	if (got != child || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		printf("FAIL ignored SIGTSTP status=0x%x\n", status);
		return 1;
	}
	printf("PASS ignored SIGTSTP\n");
	return 0;
}

static int test_default_stop(int signum, const char *label)
{
	volatile unsigned *page = poc_shared_progress();
	pid_t child;
	int want = (signum << 8) | 0x7f;

	child = fork();
	if (child < 0) {
		perror(label);
		return 1;
	}
	if (child == 0) {
		page[0] = 1;
		for (;;)
			pause();
	}
	if (wait_until(page, label) != 0)
		return 1;
	if (kill(child, signum) != 0) {
		perror(label);
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, want, label) != 0)
		return 1;
	if (expect_no_further_stop(child, label) != 0)
		return 1;
	if (kill(child, SIGCONT) != 0) {
		perror(label);
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, label) != 0)
		return 1;
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	printf("PASS %s\n", label);
	return 0;
}

struct partial_state {
	volatile unsigned started;
	volatile unsigned open_spin;
	volatile unsigned blocked_spin;
};

static void *blocked_sigtstp_spin(void *arg)
{
	struct partial_state *state = arg;
	sigset_t set;

	sigemptyset(&set);
	sigaddset(&set, SIGTSTP);
	if (sigprocmask(SIG_BLOCK, &set, NULL) != 0)
		return NULL;
	state->started = 1;
	for (;;)
		state->blocked_spin++;
	return NULL;
}

static int test_partial_block(void)
{
	struct partial_state *state = (struct partial_state *)group_state_map();
	pthread_t thread;
	pid_t child;
	unsigned open_at_stop;
	unsigned blocked_at_stop;

	child = fork();
	if (child < 0) {
		perror("fork partial");
		return 1;
	}
	if (child == 0) {
		if (pthread_create(&thread, NULL, blocked_sigtstp_spin, state) != 0)
			_exit(1);
		while (state->started == 0)
			usleep(1000);
		for (;;)
			state->open_spin++;
	}
	if (wait_until(&state->started, "partial block") != 0)
		return 1;
	usleep(2000);
	if (kill(child, SIGTSTP) != 0) {
		perror("partial SIGTSTP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x147f, "partial SIGTSTP") != 0)
		return 1;
	open_at_stop = state->open_spin;
	blocked_at_stop = state->blocked_spin;
	usleep(2000);
	if (state->open_spin != open_at_stop ||
	    state->blocked_spin != blocked_at_stop) {
		printf("FAIL partial block ran while stopped\n");
		return 1;
	}
	if (expect_no_further_stop(child, "partial SIGTSTP") != 0)
		return 1;
	if (kill(child, SIGCONT) != 0) {
		perror("partial SIGCONT");
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, "partial SIGCONT") != 0)
		return 1;
	if (wait_until(&state->open_spin, "partial resume") != 0)
		return 1;
	usleep(2000);
	if (state->open_spin <= open_at_stop ||
	    state->blocked_spin <= blocked_at_stop) {
		printf("FAIL partial block did not resume\n");
		return 1;
	}
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	printf("PASS partial SIGTSTP block\n");
	return 0;
}

struct sigwait_state {
	volatile unsigned started;
	volatile unsigned got;
};

static void *sigtstp_sigwait(void *arg)
{
	struct sigwait_state *state = arg;
	sigset_t set;
	int signum = 0;

	sigemptyset(&set);
	sigaddset(&set, SIGTSTP);
	if (sigprocmask(SIG_BLOCK, &set, NULL) != 0)
		return NULL;
	state->started = 1;
	if (sigwait(&set, &signum) == 0 && signum == SIGTSTP)
		state->got = 1;
	return NULL;
}

static int test_sigwait_peer(void)
{
	struct sigwait_state *state = (struct sigwait_state *)group_state_map();
	pthread_t thread;
	pid_t child;
	int status = 0;
	int got;

	child = fork();
	if (child < 0) {
		perror("fork sigwait peer");
		return 1;
	}
	if (child == 0) {
		if (pthread_create(&thread, NULL, sigtstp_sigwait, state) != 0)
			_exit(1);
		while (state->got == 0)
			usleep(1000);
		_exit(0);
	}
	if (wait_until(&state->started, "sigwait peer") != 0)
		return 1;
	if (kill(child, SIGTSTP) != 0) {
		perror("sigwait SIGTSTP");
		return 1;
	}
	for (got = 0; got < 300 && state->got == 0; got++)
		usleep(1000);
	got = waitpid(child, &status, WNOHANG | WUNTRACED);
	if (got == child && WIFSTOPPED(status)) {
		printf("FAIL sigwait peer stopped status=0x%x\n", status);
		return 1;
	}
	if (state->got == 0) {
		printf("FAIL sigwait peer did not receive SIGTSTP\n");
		return 1;
	}
	if (got == 0)
		got = waitpid(child, &status, 0);
	if (got != child || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		printf("FAIL sigwait peer status=0x%x\n", status);
		return 1;
	}
	printf("PASS sigwait peer SIGTSTP\n");
	return 0;
}

struct hammer_state {
	volatile unsigned spin[4];
};

static void *hammer_spin(void *arg)
{
	volatile unsigned *slot = arg;

	for (;;) {
		(*slot)++;
		if ((*slot & 63u) == 0)
			sched_yield();
	}
	return NULL;
}

static int quiet_status(pid_t child, int options, int want)
{
	int status = 0;

	if (waitpid(child, &status, options) != child || status != want)
		return -1;
	return 0;
}

static int counters_held(struct hammer_state *state, const unsigned *sample)
{
	int i;

	for (i = 0; i < 4; i++) {
		if (state->spin[i] != sample[i])
			return 0;
	}
	return 1;
}

static int counters_moved(struct hammer_state *state, const unsigned *sample)
{
	int i;
	int spins;

	for (spins = 0; spins < 300; spins++) {
		int ready = 1;

		for (i = 0; i < 4; i++) {
			/* A spinning counter wraps. Any change is progress. */
			if (state->spin[i] == sample[i])
				ready = 0;
		}
		if (ready)
			return 1;
		usleep(1000);
	}
	return 0;
}

static int test_stop_cont_hammer(void)
{
	struct hammer_state *state = (struct hammer_state *)group_state_map();
	pthread_t thread[4];
	pid_t child;
	int round;

	child = fork();
	if (child < 0) {
		perror("fork hammer");
		return 1;
	}
	if (child == 0) {
		int i;

		for (i = 0; i < 4; i++) {
			if (pthread_create(&thread[i], NULL, hammer_spin,
					   (void *)&state->spin[i]) != 0)
				_exit(1);
		}
		for (;;)
			pause();
	}
	if (wait_until(&state->spin[0], "hammer") != 0)
		return 1;
	for (round = 0; round < 1000; round++) {
		unsigned sample[4];
		int i;

		for (i = 0; i < 4; i++)
			sample[i] = state->spin[i];
		if (kill(child, SIGSTOP) != 0) {
			perror("hammer SIGSTOP");
			return 1;
		}
		if (quiet_status(child, WUNTRACED, 0x137f) != 0) {
			printf("FAIL hammer round %d did not stop\n", round);
			return 1;
		}
		for (i = 0; i < 4; i++)
			sample[i] = state->spin[i];
		usleep(50000);
		if (!counters_held(state, sample)) {
			printf("FAIL hammer round %d ran while stopped\n", round);
			return 1;
		}
		if (kill(child, SIGCONT) != 0) {
			perror("hammer SIGCONT");
			return 1;
		}
		if (quiet_status(child, WCONTINUED, 0xffff) != 0) {
			printf("FAIL hammer round %d did not continue\n", round);
			return 1;
		}
		if (!counters_moved(state, sample)) {
			printf("FAIL hammer round %d thread stalled\n", round);
			return 1;
		}
	}
	kill(child, SIGKILL);
	waitpid(child, NULL, 0);
	printf("PASS stop/cont hammer x1000\n");
	return 0;
}

static void *stale_sleeper(void *arg)
{
	volatile unsigned *started = arg;
	struct timespec delay = { .tv_sec = 2, .tv_nsec = 0 };

	*started = 1;
	nanosleep(&delay, NULL);
	return NULL;
}

static int test_queued_sigtstp_stays_stopped(void)
{
	struct hammer_state *state = (struct hammer_state *)group_state_map();
	pthread_t sleeper;
	pthread_t spinner;
	pid_t child;
	unsigned frozen;
	int status = 0;

	child = fork();
	if (child < 0) {
		perror("fork queued SIGTSTP");
		return 1;
	}
	if (child == 0) {
		if (pthread_create(&sleeper, NULL, stale_sleeper,
				   (void *)&state->spin[1]) != 0)
			_exit(1);
		if (pthread_create(&spinner, NULL, hammer_spin,
				   (void *)&state->spin[0]) != 0)
			_exit(1);
		for (;;)
			pause();
	}
	if (wait_until(&state->spin[0], "queued spinner") != 0)
		return 1;
	if (wait_until(&state->spin[1], "queued sleeper") != 0)
		return 1;
	if (kill(child, SIGCONT) != 0) {
		perror("queued SIGCONT");
		return 1;
	}
	if (kill(child, SIGTSTP) != 0) {
		perror("queued SIGTSTP");
		return 1;
	}
	if (poc_expect_status(child, WUNTRACED, 0x147f, "queued SIGTSTP") != 0)
		return 1;
	frozen = state->spin[0];
	usleep(100000);
	if (state->spin[0] != frozen) {
		printf("FAIL queued SIGTSTP moved while stopped (%u -> %u)\n",
		       frozen, state->spin[0]);
		return 1;
	}
	if (kill(child, SIGCONT) != 0) {
		perror("queued SIGCONT resume");
		return 1;
	}
	if (poc_expect_status(child, WCONTINUED, 0xffff, "queued SIGCONT") != 0)
		return 1;
	kill(child, SIGKILL);
	waitpid(child, &status, 0);
	printf("PASS queued SIGTSTP stayed stopped\n");
	return 0;
}

static int test_signal_after_fork(void)
{
	volatile unsigned *page = poc_shared_progress();
	pid_t child;
	int status = 0;

	memset((void *)page, 0, 4096);
	handled_signum = 0;
	handler_slot = &page[0];
	child = fork();
	if (child < 0) {
		perror("fork signal return");
		return 1;
	}
	if (child == 0) {
		if (raw_sigaction(SIGUSR2, catch_stop) != 0)
			_exit(2);
		if (raise(SIGUSR2) != 0)
			_exit(3);
		_exit(handled_signum == SIGUSR2 ? 0 : 4);
	}
	if (waitpid(child, &status, 0) != child || !WIFEXITED(status) ||
	    WEXITSTATUS(status) != 0 || page[0] != 2) {
		printf("FAIL signal after fork status=0x%x slot=%u\n", status,
		       page[0]);
		return 1;
	}
	printf("PASS signal after fork\n");
	return 0;
}

static int test_signal_after_exec(void)
{
	pid_t child;
	int status = 0;

	child = fork();
	if (child < 0) {
		perror("fork signal after exec");
		return 1;
	}
	if (child == 0) {
		char *argv[] = { "g2_stop_group.elf", "after-exec", NULL };

		if (raw_sigaction(SIGUSR2, catch_stop) != 0)
			_exit(2);
		if (raise(SIGUSR2) != 0 || handled_signum != SIGUSR2)
			_exit(3);
		handled_signum = 0;
		execve("/shared/g2_stop_group.elf", argv, NULL);
		if (errno != ENOSYS)
			_exit(4);
		/* execve did not replace the image. The kernel still drops the
		 * trampoline slot, and the next signal return must install one. */
		if (raw_sigaction(SIGUSR2, catch_stop) != 0)
			_exit(5);
		if (raise(SIGUSR2) != 0 || handled_signum != SIGUSR2)
			_exit(6);
		_exit(0);
	}
	if (waitpid(child, &status, 0) != child || !WIFEXITED(status) ||
	    WEXITSTATUS(status) != 0) {
		printf("FAIL signal after exec status=0x%x\n", status);
		return 1;
	}
	printf("PASS signal after exec\n");
	return 0;
}

static int test_one_nanosecond_sigtimedwait(int count)
{
	int i;

	for (i = 0; i < count; i++) {
		sigset_t set;
		struct timespec timeout = { .tv_sec = 0, .tv_nsec = 1 };
		int got;

		sigemptyset(&set);
		sigaddset(&set, SIGUSR2);
		errno = 0;
		got = sigtimedwait(&set, NULL, &timeout);
		if (got != -1 || errno != EAGAIN) {
			printf("FAIL 1ns sigtimedwait i=%d rc=%d errno=%d\n", i,
			       got, errno);
			return 1;
		}
	}
	printf("PASS 1ns sigtimedwait x%d\n", count);
	return 0;
}

int main(int argc, char **argv)
{
	if (argc > 1 && strcmp(argv[1], "after-exec") == 0) {
		if (raw_sigaction(SIGUSR2, catch_stop) != 0)
			return 2;
		if (raise(SIGUSR2) != 0)
			return 3;
		return handled_signum == SIGUSR2 ? 0 : 4;
	}
	if (argc > 1 && strcmp(argv[1], "handler") == 0)
		return test_caught_sigtstp();
	if (argc > 1 && strcmp(argv[1], "hammer") == 0)
		return test_stop_cont_hammer();
	if (argc > 1 && strcmp(argv[1], "seq") == 0) {
		if (test_caught_sigtstp() != 0)
			return 1;
		if (test_blocked_sigtstp() != 0)
			return 1;
		if (test_caught_sigtstp() != 0)
			return 1;
		printf("PASS handler after blocked\n");
		return 0;
	}
	if (argc > 1 && strcmp(argv[1], "nsec") == 0) {
		int count = argc > 2 ? atoi(argv[2]) : 100000;

		return test_one_nanosecond_sigtimedwait(count);
	}
	if (test_thread_group() != 0)
		return 1;
	if (test_process_group() != 0)
		return 1;
	if (test_repeat_stop_and_kill() != 0)
		return 1;
	if (stop_while_waiting(futex_waiter, "futex") != 0)
		return 1;
	if (stop_while_waiting(signal_waiter, "sigtimedwait") != 0)
		return 1;
	if (test_blocked_sigtstp() != 0)
		return 1;
	if (test_caught_sigtstp() != 0)
		return 1;
	if (test_ignored_sigtstp() != 0)
		return 1;
	if (test_default_stop(SIGTSTP, "default SIGTSTP") != 0)
		return 1;
	if (test_partial_block() != 0)
		return 1;
	if (test_sigwait_peer() != 0)
		return 1;
	if (test_default_stop(SIGTTIN, "default SIGTTIN") != 0)
		return 1;
	if (test_default_stop(SIGTTOU, "default SIGTTOU") != 0)
		return 1;
	if (test_exit_sigstop_reaps() != 0)
		return 1;
	if (test_queued_sigtstp_stays_stopped() != 0)
		return 1;
	if (test_signal_after_fork() != 0)
		return 1;
	if (test_signal_after_exec() != 0)
		return 1;
	if (test_stop_cont_hammer() != 0)
		return 1;
	if (test_one_nanosecond_sigtimedwait(1000) != 0)
		return 1;
	printf("PASS SIGSTOP broadcast\n");
	return 0;
}
