/*******************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *          Module: hanoi.c   SID: 3.3 5/15/91 19:30:20
 *
 *  SMROS port: bounded Towers of Hanoi loop.
 ******************************************************************************/

#include "smros_unix.h"

#define other(i, j) (6 - ((i) + (j)))

static int num[4];

static void mov(int n, int f, int t) {
  int o;
  if (n == 1) {
    num[f]--;
    num[t]++;
    return;
  }
  o = other(f, t);
  mov(n - 1, f, o);
  mov(1, f, t);
  mov(n - 1, o, t);
}

static uint64_t hanoi_10(uint64_t iterations) {
  uint64_t iter = 0;
  for (; iter < iterations; ++iter) {
    num[0] = 0;
    num[1] = 10;
    num[2] = 0;
    num[3] = 0;
    mov(10, 1, 3);
  }
  return iter;
}

int ub_hanoi_main(void) {
  ub_print("hanoi.c: Recursion\n");
  ub_calibrate("hanoi/10-disks", hanoi_10, 16);
  return 0;
}
