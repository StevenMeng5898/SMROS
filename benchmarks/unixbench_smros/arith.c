/*******************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *          Module: arith.c   SID: 3.3 5/15/91 19:30:19
 *
 *  SMROS port: bounded arithmetic loops for int, long, float, and double.
 ******************************************************************************/

#include "smros_unix.h"

static int dumb_int(int i) {
  int x = 0;
  int y = 0;
  int z = 0;
  for (i = 2; i <= 101; i++) {
    x = i;
    y = x * x;
    z += y / (y - 1);
  }
  return x + y + z;
}

static long dumb_long(long i) {
  long x = 0;
  long y = 0;
  long z = 0;
  for (i = 2; i <= 101; i++) {
    x = i;
    y = x * x;
    z += y / (y - 1);
  }
  return x + y + z;
}

static double dumb_double(double i) {
  double x = 0;
  double y = 0;
  double z = 0;
  for (i = 2; i <= 101; i += 1.0) {
    x = i;
    y = x * x;
    z += y / (y - 1.0);
  }
  return x + y + z;
}

static uint64_t arith_int(uint64_t iterations) {
  volatile int result = 0;
  for (uint64_t i = 0; i < iterations; ++i) {
    result = dumb_int(result);
  }
  return iterations + (uint64_t)(result == 0);
}

static uint64_t arith_long(uint64_t iterations) {
  volatile long result = 0;
  for (uint64_t i = 0; i < iterations; ++i) {
    result = dumb_long(result);
  }
  return iterations + (uint64_t)(result == 0);
}

static uint64_t arith_double(uint64_t iterations) {
  volatile double result = 0.0;
  for (uint64_t i = 0; i < iterations; ++i) {
    result = dumb_double(result);
  }
  return iterations + (uint64_t)(result == 0.0);
}

int ub_arith_main(void) {
  ub_print("arith.c: Arithmetic\n");
  ub_calibrate("arith/int", arith_int, 64);
  ub_calibrate("arith/long", arith_long, 64);
  ub_calibrate("arith/double", arith_double, 64);
  return 0;
}
