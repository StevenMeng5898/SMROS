/*******************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *      Module: fstime.c   SID: 3.5 5/15/91 19:30:19
 *
 *  SMROS port: preserves the file write/read/copy kernels with bounded loops
 *  and FxFS-backed files under /shared.
 ******************************************************************************/

#include "smros_unix.h"

#define MAX_BUFSIZE 8192
#define COUNTSIZE 256
#define HALFCOUNT (COUNTSIZE / 2)

static char fname0[] = "/shared/unixbench-dummy0";
static char fname1[] = "/shared/unixbench-dummy1";
static char buf[MAX_BUFSIZE];
static int bufsize = 1024;
static int count_per_buf = 1024 / COUNTSIZE;

static int open_files(int* f, int* g) {
  *f = ub_creat(fname0, 0600);
  if (*f < 0) {
    return -1;
  }
  ub_close(*f);
  *g = ub_creat(fname1, 0600);
  if (*g < 0) {
    return -1;
  }
  ub_close(*g);
  *f = ub_open(fname0, UB_O_RDWR, 0);
  *g = ub_open(fname1, UB_O_RDWR, 0);
  if (*f < 0 || *g < 0) {
    return -1;
  }
  return 0;
}

static void close_files(int f, int g) {
  if (f >= 0) {
    ub_close(f);
  }
  if (g >= 0) {
    ub_close(g);
  }
  ub_unlink(fname0);
  ub_unlink(fname1);
}

static uint64_t write_test(uint64_t iterations) {
  int f = -1;
  int g = -1;
  if (open_files(&f, &g) != 0) {
    return 0;
  }

  uint64_t counted = 0;
  for (uint64_t i = 0; i < iterations; ++i) {
    int64_t written = ub_write(f, buf, (size_t)bufsize);
    if (written != bufsize) {
      counted += (uint64_t)((written + HALFCOUNT) / COUNTSIZE);
      break;
    }
    counted += (uint64_t)count_per_buf;
    ub_lseek(f, 0, UB_SEEK_SET);
  }
  close_files(f, g);
  return counted;
}

static uint64_t read_test(uint64_t iterations) {
  int f = -1;
  int g = -1;
  if (open_files(&f, &g) != 0) {
    return 0;
  }
  for (unsigned i = 0; i < 32; ++i) {
    if (ub_write(f, buf, (size_t)bufsize) != bufsize) {
      close_files(f, g);
      return 0;
    }
  }
  ub_lseek(f, 0, UB_SEEK_SET);

  uint64_t counted = 0;
  for (uint64_t i = 0; i < iterations; ++i) {
    int64_t read_bytes = ub_read(f, buf, (size_t)bufsize);
    if (read_bytes != bufsize) {
      ub_lseek(f, 0, UB_SEEK_SET);
      counted += (uint64_t)((read_bytes + HALFCOUNT) / COUNTSIZE);
      continue;
    }
    counted += (uint64_t)count_per_buf;
  }
  close_files(f, g);
  return counted;
}

static uint64_t copy_test(uint64_t iterations) {
  int f = -1;
  int g = -1;
  if (open_files(&f, &g) != 0) {
    return 0;
  }
  for (unsigned i = 0; i < 32; ++i) {
    if (ub_write(f, buf, (size_t)bufsize) != bufsize) {
      close_files(f, g);
      return 0;
    }
  }
  ub_lseek(f, 0, UB_SEEK_SET);
  ub_lseek(g, 0, UB_SEEK_SET);

  uint64_t counted = 0;
  for (uint64_t i = 0; i < iterations; ++i) {
    int64_t read_bytes = ub_read(f, buf, (size_t)bufsize);
    if (read_bytes != bufsize) {
      ub_lseek(f, 0, UB_SEEK_SET);
      ub_lseek(g, 0, UB_SEEK_SET);
      continue;
    }
    int64_t written = ub_write(g, buf, (size_t)bufsize);
    if (written != bufsize) {
      counted += (uint64_t)((written + HALFCOUNT) / COUNTSIZE);
      break;
    }
    counted += (uint64_t)count_per_buf;
  }
  close_files(f, g);
  return counted;
}

int ub_fstime_main(void) {
  for (int i = 0; i < bufsize; ++i) {
    buf[i] = (char)(i & 0xff);
  }
  ub_print("fstime.c: File Write/Read/Copy\n");
  ub_calibrate("fstime/write/1024", write_test, 16);
  ub_calibrate("fstime/read/1024", read_test, 16);
  ub_calibrate("fstime/copy/1024", copy_test, 16);
  return 0;
}
