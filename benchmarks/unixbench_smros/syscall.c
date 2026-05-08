/*******************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *          Module: syscall.c   SID: 3.3 5/15/91 19:30:21
 *
 *  SMROS port: original standalone main/alarm/report loop is converted into
 *  bounded C functions using the same syscall mix.
 ******************************************************************************/

#include "smros_unix.h"

static int create_fd(void) {
  int fd[2];
  if (ub_pipe(fd) != 0 || ub_close(fd[1]) != 0) {
    return -1;
  }
  return fd[0];
}

static uint64_t syscall_mix(uint64_t iterations) {
  int fd = create_fd();
  if (fd < 0) {
    return 0;
  }

  uint64_t iter = 0;
  for (; iter < iterations; ++iter) {
    int dup_fd = ub_dup(fd);
    if (dup_fd < 0) {
      break;
    }
    if (ub_close(dup_fd) != 0) {
      break;
    }
    (void)ub_getpid();
    (void)ub_getuid();
    (void)ub_umask(022);
  }
  ub_close(fd);
  return iter;
}

static uint64_t syscall_getpid(uint64_t iterations) {
  uint64_t iter = 0;
  for (; iter < iterations; ++iter) {
    (void)ub_getpid();
  }
  return iter;
}

static uint64_t syscall_close_dup(uint64_t iterations) {
  int fd = create_fd();
  if (fd < 0) {
    return 0;
  }

  uint64_t iter = 0;
  for (; iter < iterations; ++iter) {
    int dup_fd = ub_dup(fd);
    if (dup_fd < 0) {
      break;
    }
    if (ub_close(dup_fd) != 0) {
      break;
    }
  }
  ub_close(fd);
  return iter;
}

int ub_syscall_main(void) {
  ub_print("syscall.c: System Call Overhead\n");
  ub_calibrate("syscall/mix", syscall_mix, 64);
  ub_calibrate("syscall/getpid", syscall_getpid, 128);
  ub_calibrate("syscall/close-dup", syscall_close_dup, 64);
  return 0;
}
