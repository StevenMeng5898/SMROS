/* Explicit-state model of a 3-thread futex mutex. This file does not include
 * or call pthread_mutex_futex.c. Matching the real slow path is manual.
 *
 * Limits: safety only (no liveness), exchange and the following wait are one
 * step, and trylock is absent. A state with a sleeper, word 0, and nobody
 * holding or retrying is a lost wakeup. BROKEN_SLOW_PATH stores 1 after
 * seeing 0 and must exit non-zero. */

#include <stdio.h>
#include <stdlib.h>

enum { OUT = 0, HOLD = 1, SLEEP = 2, SLOW = 3, N = 3 };

struct State {
    unsigned char th[N];
    unsigned char word;
};

static unsigned slow_store(unsigned word)
{
#ifdef BROKEN_SLOW_PATH
    return word == 0 ? 1u : 2u;
#else
    (void)word;
    return 2u;
#endif
}

static int bad(const struct State *s)
{
    int sleepers = 0;
    int holders = 0;
    int retrying = 0;
    int i;
    for (i = 0; i < N; i++) {
        if (s->th[i] == SLEEP) {
            sleepers++;
        }
        if (s->th[i] == HOLD) {
            holders++;
        }
        if (s->th[i] == SLOW) {
            retrying++;
        }
    }
    if (holders > 1) {
        return 1;
    }
    /* A sleeper with a free word and nobody holding or retrying will not be woken. */
    return sleepers > 0 && s->word == 0 && holders == 0 && retrying == 0;
}

static unsigned encode(const struct State *s)
{
    unsigned v = s->word;
    int i;
    for (i = 0; i < N; i++) {
        v = v * 4u + s->th[i];
    }
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

int main(void)
{
    /* word in 0..2 and three threads in 0..3: 3*4^3 = 192. */
    unsigned char seen[192];
    struct State *q = NULL;
    int n = 0;
    int cap = 0;
    int head = 0;
    int i;
    int lost = 0;
    struct State start;

    for (i = 0; i < 192; i++) {
        seen[i] = 0;
    }
    for (i = 0; i < N; i++) {
        start.th[i] = OUT;
    }
    start.word = 0;
    push(&q, &n, &cap, start, seen);

    while (head < n) {
        struct State cur = q[head++];
        if (bad(&cur)) {
            lost++;
            continue;
        }
        for (i = 0; i < N; i++) {
            struct State next = cur;
            if (cur.th[i] == OUT && cur.word == 0) {
                next.word = 1;
                next.th[i] = HOLD;
                push(&q, &n, &cap, next, seen);
            } else if (cur.th[i] == OUT && cur.word != 0) {
                next.th[i] = SLOW;
                push(&q, &n, &cap, next, seen);
            } else if (cur.th[i] == SLOW) {
#ifdef BROKEN_SLOW_PATH
                if (cur.word == 0) {
                    next.word = 1;
                    next.th[i] = HOLD;
                    push(&q, &n, &cap, next, seen);
                } else
#endif
                {
                    unsigned prev = cur.word;
                    next.word = (unsigned char)slow_store(cur.word);
                    if (prev == 0) {
                        next.th[i] = HOLD;
                    } else if (next.word == 2) {
                        next.th[i] = SLEEP;
                    } else {
                        next.th[i] = SLOW;
                    }
                    push(&q, &n, &cap, next, seen);
                }
            } else if (cur.th[i] == HOLD) {
                unsigned prev = cur.word;
                int w;
                next.word = 0;
                next.th[i] = OUT;
                if (prev == 2) {
                    for (w = 0; w < N; w++) {
                        if (next.th[w] == SLEEP) {
                            next.th[w] = SLOW;
                            break;
                        }
                    }
                }
                push(&q, &n, &cap, next, seen);
            } else if (cur.th[i] == SLEEP && cur.word != 2) {
                next.th[i] = SLOW;
                push(&q, &n, &cap, next, seen);
            }
        }
    }

    free(q);
    if (lost != 0) {
        printf("lost-wakeup states=%d reachable=%d\n", lost, n);
        return 1;
    }
    printf("no lost wakeup reachable=%d\n", n);
    return 0;
}
