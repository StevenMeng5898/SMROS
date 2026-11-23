/* Explicit-state model of sigtimedwait versus the precision timer.
 *
 * Fixed protocol: expiring the wait sets the pending-wake bit in the same
 * task-lock step, before the scheduler state is read. block_current and
 * schedule both honor that bit after block_thread. BROKEN_SIGNAL_WAKE reads
 * the scheduler state first and sets the bit only after both honors have
 * been able to miss it (trace shape WXXWWWX, lost > 0).
 *
 * Lost means the wait has timed out, the thread has left the CPU, and the
 * bit is clear, so nobody will wake it.
 */

#include <stdio.h>
#include <stdlib.h>

enum { WAITING = 0, TIMED_OUT = 1, READY = 0, BLOCKED = 1, RUNNABLE = 0, ASLEEP = 1 };

struct State {
    unsigned char wait_pc;
    unsigned char timer_pc;
    unsigned char outcome;
    unsigned char sched;
    unsigned char task;
    unsigned char bit;
};

static int waiter_done(const struct State *s)
{
    return s->wait_pc == 3;
}

static int timer_done(const struct State *s)
{
    return s->timer_pc == 2;
}

static int lost(const struct State *s)
{
    /* Asleep after every honor and the timer step means the timeout will
     * not be observed. A bit set only after those honors does not help. */
    return waiter_done(s) && timer_done(s) && s->outcome == TIMED_OUT && s->sched == BLOCKED &&
           s->task == ASLEEP;
}

static unsigned encode(const struct State *s)
{
    unsigned v = s->wait_pc;
    v = v * 3u + s->timer_pc;
    v = v * 2u + s->outcome;
    v = v * 2u + s->sched;
    v = v * 2u + s->task;
    v = v * 2u + s->bit;
    return v;
}

static void push(struct State **q, int *n, int *cap, struct State s, unsigned char *seen)
{
    unsigned id = encode(&s);
    if (seen[id]) {
        return;
    }
    seen[id] = 1;
    if (*n == *cap) {
        *cap = *cap ? *cap * 2 : 64;
        *q = realloc(*q, (size_t)(*cap) * sizeof(**q));
        if (*q == NULL) {
            perror("realloc");
            exit(2);
        }
    }
    (*q)[(*n)++] = s;
}

static void honor(struct State *s)
{
    if (s->bit && s->outcome == TIMED_OUT) {
        s->bit = 0;
        s->task = RUNNABLE;
        s->sched = READY;
    }
}

static void step_wait(struct State *s)
{
    if (s->wait_pc == 0) {
        s->task = ASLEEP;
        s->wait_pc = 1;
        return;
    }
    if (s->wait_pc == 1) {
        s->sched = BLOCKED;
        s->wait_pc = 2;
        return;
    }
    honor(s);
    s->wait_pc = 3;
}

static void step_timer(struct State *s)
{
#ifdef BROKEN_SIGNAL_WAKE
    if (s->timer_pc == 0) {
        s->outcome = TIMED_OUT;
        if (s->sched == BLOCKED) {
            s->task = RUNNABLE;
            s->sched = READY;
        }
        s->timer_pc = 1;
        return;
    }
    s->bit = 1;
    s->timer_pc = 2;
#else
    if (s->timer_pc == 0) {
        s->outcome = TIMED_OUT;
        s->bit = 1;
        s->timer_pc = 1;
        return;
    }
    if (s->sched == BLOCKED && s->outcome == TIMED_OUT) {
        s->bit = 0;
        s->task = RUNNABLE;
        s->sched = READY;
    }
    s->timer_pc = 2;
#endif
}

int main(void)
{
    enum { N = 4 * 3 * 2 * 2 * 2 * 2 };
    unsigned char seen[N] = {0};
    struct State *q = NULL;
    int n = 0;
    int cap = 0;
    int read = 0;
    int lost_count = 0;
    int terminals = 0;
    struct State start = {0, 0, WAITING, READY, RUNNABLE, 0};

    push(&q, &n, &cap, start, seen);
    while (read < n) {
        struct State cur = q[read++];
        if (waiter_done(&cur) && timer_done(&cur)) {
            terminals++;
            if (lost(&cur)) {
                lost_count++;
            }
            continue;
        }
        if (cur.wait_pc < 3) {
            struct State next = cur;
            step_wait(&next);
            push(&q, &n, &cap, next, seen);
        }
        if (cur.timer_pc < 2) {
            struct State next = cur;
            step_timer(&next);
            push(&q, &n, &cap, next, seen);
        }
    }
    printf("reachable=%d terminals=%d lost=%d\n", n, terminals, lost_count);
    free(q);
    return lost_count == 0 ? 0 : 1;
}
