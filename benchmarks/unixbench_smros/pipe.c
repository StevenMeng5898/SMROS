/*******************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *          Module: pipe.c   SID: 3.3 5/15/91 19:30:20
 *
 *  SMROS port: bounded single-process pipe throughput loop.
 ******************************************************************************/

#include "smros_unix.h"

static uint64_t pipe_throughput(uint64_t iterations) {
  char buf[512];
  int pvec[2];
  if (ub_pipe(pvec) != 0) {
    return 0;
  }
  ub_memset(buf, 0xa5, sizeof(buf));

  uint64_t iter = 0;
  for (; iter < iterations; ++iter) {
    if (ub_write(pvec[1], buf, sizeof(buf)) != (int64_t)sizeof(buf)) {
      break;
    }
    if (ub_read(pvec[0], buf, sizeof(buf)) != (int64_t)sizeof(buf)) {
      break;
    }
  }

  ub_close(pvec[0]);
  ub_close(pvec[1]);
  return iter;
}

int ub_pipe_main(void) {
  ub_print("pipe.c: Pipe Throughput\n");
  ub_calibrate("pipe/512-byte/write-read", pipe_throughput, 32);
  return 0;
}
