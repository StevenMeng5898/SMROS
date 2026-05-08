#include "smros_unix.h"

uint64_t ub_now_ns(void) {
  ub_timespec ts;
  uint64_t status = ub_syscall6(UB_SYS_CLOCK_GETTIME, 1, (uint64_t)&ts, 0, 0, 0, 0);
  if (ub_is_error(status)) {
    return 1;
  }
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

void ub_exit(int code) {
  ub_syscall6(UB_SYS_EXIT, (uint64_t)code, 0, 0, 0, 0, 0);
  for (;;) {
    asm volatile("wfe");
  }
}

void ub_sleep_ms(uint64_t ms) {
  ub_timespec ts;
  ts.tv_sec = (int64_t)(ms / 1000);
  ts.tv_nsec = (int64_t)((ms % 1000) * 1000000ull);
  ub_syscall6(UB_SYS_NANOSLEEP, (uint64_t)&ts, 0, 0, 0, 0, 0);
}

size_t ub_strlen(const char* value) {
  size_t len = 0;
  while (value[len] != 0) {
    ++len;
  }
  return len;
}

int ub_strcmp(const char* left, const char* right) {
  while (*left != 0 && *left == *right) {
    ++left;
    ++right;
  }
  return (unsigned char)*left - (unsigned char)*right;
}

char* ub_strcpy(char* dest, const char* src) {
  char* out = dest;
  while ((*out++ = *src++) != 0) {
  }
  return dest;
}

void* ub_memset(void* dest, int value, size_t count) {
  unsigned char* out = (unsigned char*)dest;
  for (size_t i = 0; i < count; ++i) {
    out[i] = (unsigned char)value;
  }
  return dest;
}

void* ub_memcpy(void* dest, const void* src, size_t count) {
  unsigned char* out = (unsigned char*)dest;
  const unsigned char* in = (const unsigned char*)src;
  for (size_t i = 0; i < count; ++i) {
    out[i] = in[i];
  }
  return dest;
}

int ub_atoi(const char* value) {
  int sign = 1;
  int out = 0;
  while (*value == ' ' || *value == '\t' || *value == '\n') {
    ++value;
  }
  if (*value == '-') {
    sign = -1;
    ++value;
  }
  while (*value >= '0' && *value <= '9') {
    out = out * 10 + (*value - '0');
    ++value;
  }
  return out * sign;
}

void ub_print(const char* value) {
  ub_syscall6(UB_SYS_WRITE, 1, (uint64_t)value, ub_strlen(value), 0, 0, 0);
}

void ub_print_char(char value) { ub_syscall6(UB_SYS_WRITE, 1, (uint64_t)&value, 1, 0, 0, 0); }

void ub_print_u64(uint64_t value) {
  char buf[32];
  size_t len = 0;
  if (value == 0) {
    ub_print_char('0');
    return;
  }
  while (value != 0 && len < sizeof(buf)) {
    buf[len++] = (char)('0' + (value % 10));
    value /= 10;
  }
  while (len != 0) {
    ub_print_char(buf[--len]);
  }
}

void ub_print_i64(int64_t value) {
  if (value < 0) {
    ub_print_char('-');
    ub_print_u64((uint64_t)(-value));
  } else {
    ub_print_u64((uint64_t)value);
  }
}

void ub_print_result(const char* name, uint64_t count, const char* unit, uint64_t elapsed_ns) {
  ub_print("  ");
  ub_print(name);
  ub_print(": COUNT|");
  ub_print_u64(count);
  ub_print("|1|");
  ub_print(unit);
  ub_print(" elapsed_ns=");
  ub_print_u64(elapsed_ns);
  ub_print(" per_iter_ns=");
  ub_print_u64(count == 0 ? 0 : elapsed_ns / count);
  ub_print("\n");
}

void ub_print_failure(const char* name, int64_t status) {
  ub_print("  ");
  ub_print(name);
  ub_print(": FAIL ");
  ub_print_i64(status);
  ub_print("\n");
}

int ub_pipe(int fds[2]) {
  uint64_t status = ub_syscall6(UB_SYS_PIPE2, (uint64_t)fds, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : 0;
}

int ub_close(int fd) {
  uint64_t status = ub_syscall6(UB_SYS_CLOSE, (uint64_t)fd, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : 0;
}

int ub_dup(int fd) {
  uint64_t status = ub_syscall6(UB_SYS_DUP, (uint64_t)fd, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : (int)status;
}

int ub_getpid(void) {
  uint64_t status = ub_syscall6(UB_SYS_GETPID, 0, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : (int)status;
}

int ub_getuid(void) {
  uint64_t status = ub_syscall6(UB_SYS_GETUID, 0, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : (int)status;
}

int ub_umask(int mask) {
  uint64_t status = ub_syscall6(UB_SYS_UMASK, (uint64_t)mask, 0, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : (int)status;
}

int ub_open(const char* path, int flags, int mode) {
  uint64_t status =
      ub_syscall6(UB_SYS_OPENAT, UB_AT_FDCWD, (uint64_t)path, (uint64_t)flags, (uint64_t)mode, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : (int)status;
}

int ub_creat(const char* path, int mode) { return ub_open(path, UB_O_WRONLY | UB_O_CREAT | UB_O_TRUNC, mode); }

int ub_unlink(const char* path) {
  uint64_t status = ub_syscall6(UB_SYS_UNLINKAT, UB_AT_FDCWD, (uint64_t)path, 0, 0, 0, 0);
  return ub_is_error(status) ? (int)(int64_t)status : 0;
}

int64_t ub_read(int fd, void* buf, size_t len) {
  uint64_t status = ub_syscall6(UB_SYS_READ, (uint64_t)fd, (uint64_t)buf, len, 0, 0, 0);
  return ub_is_error(status) ? (int64_t)status : (int64_t)status;
}

int64_t ub_write(int fd, const void* buf, size_t len) {
  uint64_t status = ub_syscall6(UB_SYS_WRITE, (uint64_t)fd, (uint64_t)buf, len, 0, 0, 0);
  return ub_is_error(status) ? (int64_t)status : (int64_t)status;
}

int64_t ub_lseek(int fd, int64_t offset, int whence) {
  uint64_t status =
      ub_syscall6(UB_SYS_LSEEK, (uint64_t)fd, (uint64_t)offset, (uint64_t)whence, 0, 0, 0);
  return ub_is_error(status) ? (int64_t)status : (int64_t)status;
}

uint64_t ub_calibrate(const char* name, ub_bench_fn fn, uint64_t min_iterations) {
  uint64_t iterations = min_iterations;
  uint64_t elapsed = 0;
  uint64_t result = 0;
  for (unsigned attempt = 0; attempt < 8; ++attempt) {
    uint64_t start = ub_now_ns();
    result = fn(iterations);
    elapsed = ub_now_ns() - start;
    if (elapsed != 0) {
      break;
    }
    iterations *= 2;
  }
  ub_print_result(name, result, "lps", elapsed);
  return result;
}

void* memset(void* dest, int value, size_t count) { return ub_memset(dest, value, count); }
void* memcpy(void* dest, const void* src, size_t count) { return ub_memcpy(dest, src, count); }
char* strcpy(char* dest, const char* src) { return ub_strcpy(dest, src); }
int strcmp(const char* left, const char* right) { return ub_strcmp(left, right); }
size_t strlen(const char* value) { return ub_strlen(value); }
