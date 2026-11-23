/* Explicit-state model of one STOP and one SIGCONT after the task table
 * has been marked Stopped and the process lock has been released.
 *
 * Fixed protocol: SIGCONT sets a pending-continue bit before it reads the
 * scheduler state. block_thread consumes that bit and wakes before it
 * returns. BROKEN_STOP_CONT does not set the bit, which is the lost-continue
 * window (trace shape ABBBA, lost > 0).
 *
 * A state is lost when both actors have finished, the process is not
 * stopped, and the scheduler thread is still Blocked with no bit left.
 */

#include <stdio.h>
#include <stdlib.h>

enum { READY = 0, BLOCKED = 1, RUN = 0, STOPPED = 1 };

struct State {
    unsigned char stop_pc;
    unsigned char cont_pc;
    unsigned char task;
    unsigned char sched;
    unsigned char pending;
    unsigned char proc_stopped;
};

static int terminal(const struct State *s)
{
    return s->stop_pc == 2 && s->cont_pc == 2;
}

static int lost(const struct State *s)
{
    return terminal(s) && s->proc_stopped == 0 && s->sched == BLOCKED && s->pending == 0 &&
           s->task == STOPPED;
}

static unsigned encode(const struct State *s)
{
    unsigned v = s->stop_pc;
    v = v * 3u + s->cont_pc;
    v = v * 2u + s->task;
    v = v * 2u + s->sched;
    v = v * 2u + s->pending;
    v = v * 2u + s->proc_stopped;
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

static void step_stop(struct State *s)
{
    if (s->stop_pc == 0) {
        s->sched = BLOCKED;
        s->stop_pc = 1;
        return;
    }
    if (s->pending) {
        s->pending = 0;
        if (s->task == STOPPED) {
            s->task = RUN;
            s->sched = READY;
        }
    }
    s->stop_pc = 2;
}

static void step_cont(struct State *s)
{
    if (s->cont_pc == 0) {
        s->proc_stopped = 0;
#ifdef BROKEN_STOP_CONT
        if (s->sched == BLOCKED && s->task == STOPPED) {
            s->task = RUN;
            s->sched = READY;
        }
#else
        if (s->task == STOPPED) {
            s->pending = 1;
        }
#endif
        s->cont_pc = 1;
        return;
    }
#ifndef BROKEN_STOP_CONT
    if (s->sched == BLOCKED && s->task == STOPPED) {
        s->pending = 0;
        s->task = RUN;
        s->sched = READY;
    }
#endif
    s->cont_pc = 2;
}

int main(void)
{
    enum { N = 3 * 3 * 2 * 2 * 2 * 2 };
    unsigned char seen[N] = {0};
    struct State *q = NULL;
    int n = 0;
    int cap = 0;
    int read = 0;
    int lost_count = 0;
    int terminals = 0;
    struct State start = {0, 0, STOPPED, READY, 0, 1,};

    push(&q, &n, &cap, start, seen);
    while (read < n) {
        struct State cur = q[read++];
        if (terminal(&cur)) {
            terminals++;
            if (lost(&cur)) {
                lost_count++;
            }
            continue;
        }
        if (cur.stop_pc < 2) {
            struct State next = cur;
            step_stop(&next);
            push(&q, &n, &cap, next, seen);
        }
        if (cur.cont_pc < 2) {
            struct State next = cur;
            step_cont(&next);
            push(&q, &n, &cap, next, seen);
        }
    }
    printf("reachable=%d terminals=%d lost=%d\n", n, terminals, lost_count);
    free(q);
    return lost_count == 0 ? 0 : 1;
}
