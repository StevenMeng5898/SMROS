/* CSSTT: SIGCONT on a running group, then a later SIGTSTP delivered to a
 * thread while a sibling is still inside a syscall.
 *
 * Park does not see the in-syscall sibling. Fixed code only marks a thread
 * that is already Stopped, and tasks.block(Stopped) clears the bit in the
 * same step, so the sibling stays stopped. Broken code marks the running
 * sibling and block_current consumes that bit, which undoes the stop.
 *
 * bad is a finished trace whose stop did not leave the sibling Stopped.
 */

#include <stdio.h>
#include <stdlib.h>

enum { RUN = 0, STOPPED = 1 };

struct State {
    unsigned char cont;
    unsigned char park;
    unsigned char block;
    unsigned char mark;
    unsigned char task;
};

static int terminal(const struct State *s)
{
    return s->cont && s->park && s->block;
}

static int bad(const struct State *s)
{
    return terminal(s) && s->task != STOPPED;
}

static unsigned encode(const struct State *s)
{
    unsigned v = s->cont;
    v = v * 2u + s->park;
    v = v * 2u + s->block;
    v = v * 2u + s->mark;
    v = v * 2u + s->task;
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
        *cap = *cap ? *cap * 2 : 16;
        *q = realloc(*q, (size_t)(*cap) * sizeof(**q));
        if (*q == NULL) {
            perror("realloc");
            exit(2);
        }
    }
    (*q)[(*n)++] = s;
}

static void step_cont(struct State *s)
{
#ifdef BROKEN_STALE_CONTINUE
    s->mark = 1;
#else
    if (s->task == STOPPED) {
        s->mark = 1;
    }
#endif
    s->cont = 1;
}

static void step_park(struct State *s)
{
    /* The sibling is in a syscall, so park neither clears nor blocks it. */
    s->park = 1;
}

static void step_block(struct State *s)
{
#ifdef BROKEN_STALE_CONTINUE
    if (s->mark) {
        s->mark = 0;
        s->task = RUN;
    } else {
        s->task = STOPPED;
    }
#else
    s->mark = 0;
    s->task = STOPPED;
#endif
    s->block = 1;
}

int main(void)
{
    enum { N = 2 * 2 * 2 * 2 * 2 };
    unsigned char seen[N] = {0};
    struct State *q = NULL;
    int n = 0;
    int cap = 0;
    int read = 0;
    int bad_count = 0;
    int terminals = 0;
    struct State start = {0, 0, 0, 0, RUN};

    push(&q, &n, &cap, start, seen);
    while (read < n) {
        struct State cur = q[read++];
        if (terminal(&cur)) {
            terminals++;
            if (bad(&cur)) {
                bad_count++;
            }
            continue;
        }
        if (!cur.cont) {
            struct State next = cur;
            step_cont(&next);
            push(&q, &n, &cap, next, seen);
        }
        if (!cur.park) {
            struct State next = cur;
            step_park(&next);
            push(&q, &n, &cap, next, seen);
        }
        if (!cur.block) {
            struct State next = cur;
            step_block(&next);
            push(&q, &n, &cap, next, seen);
        }
    }
    printf("reachable=%d terminals=%d bad=%d\n", n, terminals, bad_count);
    free(q);
    return bad_count == 0 ? 0 : 1;
}
