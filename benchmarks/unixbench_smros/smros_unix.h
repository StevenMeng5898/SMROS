// Minimal C runtime and Linux syscall shim for running UnixBench kernels on
// SMROS.  This keeps the port in C and avoids libc dependencies that the
// current SMROS ELF runner does not need.

#ifndef SMROS_UNIX_H
#define SMROS_UNIX_H

#include <stddef.h>
#include <stdint.h>

#define UB_AT_FDCWD ((uint64_t)-100)

#define UB_O_RDONLY 0
#define UB_O_WRONLY 1
#define UB_O_RDWR 2
#define UB_O_CREAT 0100
#define UB_O_TRUNC 01000

#define UB_SEEK_SET 0
#define UB_SEEK_CUR 1
#define UB_SEEK_END 2

#define UB_SYS_DUP 23
#define UB_SYS_PIPE2 59
#define UB_SYS_CLOSE 57
#define UB_SYS_READ 63
#define UB_SYS_WRITE 64
#define UB_SYS_OPENAT 56
#define UB_SYS_LSEEK 62
#define UB_SYS_UNLINKAT 35
#define UB_SYS_EXIT 93
#define UB_SYS_NANOSLEEP 101
#define UB_SYS_CLOCK_GETTIME 113
#define UB_SYS_GETPID 172
#define UB_SYS_GETUID 174
#define UB_SYS_UMASK 166

typedef struct {
  int64_t tv_sec;
  int64_t tv_nsec;
} ub_timespec;

typedef struct {
  int64_t tv_sec;
  int64_t tv_usec;
} ub_timeval;

static inline uint64_t ub_syscall6(uint64_t num, uint64_t arg0, uint64_t arg1, uint64_t arg2,
                                   uint64_t arg3, uint64_t arg4, uint64_t arg5) {
  register uint64_t x0 asm("x0") = arg0;
  register uint64_t x1 asm("x1") = arg1;
  register uint64_t x2 asm("x2") = arg2;
  register uint64_t x3 asm("x3") = arg3;
  register uint64_t x4 asm("x4") = arg4;
  register uint64_t x5 asm("x5") = arg5;
  register uint64_t x8 asm("x8") = num;
  asm volatile("svc #0"
               : "+r"(x0)
               : "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5), "r"(x8)
               : "memory");
  return x0;
}

static inline int ub_is_error(uint64_t value) { return (int64_t)value < 0; }

uint64_t ub_now_ns(void);
void ub_exit(int code);
void ub_sleep_ms(uint64_t ms);

size_t ub_strlen(const char* value);
int ub_strcmp(const char* left, const char* right);
char* ub_strcpy(char* dest, const char* src);
void* ub_memset(void* dest, int value, size_t count);
void* ub_memcpy(void* dest, const void* src, size_t count);
int ub_atoi(const char* value);

void ub_print(const char* value);
void ub_print_char(char value);
void ub_print_u64(uint64_t value);
void ub_print_i64(int64_t value);
void ub_print_result(const char* name, uint64_t count, const char* unit, uint64_t elapsed_ns);
void ub_print_failure(const char* name, int64_t status);

int ub_pipe(int fds[2]);
int ub_close(int fd);
int ub_dup(int fd);
int ub_getpid(void);
int ub_getuid(void);
int ub_umask(int mask);
int ub_open(const char* path, int flags, int mode);
int ub_creat(const char* path, int mode);
int ub_unlink(const char* path);
int64_t ub_read(int fd, void* buf, size_t len);
int64_t ub_write(int fd, const void* buf, size_t len);
int64_t ub_lseek(int fd, int64_t offset, int whence);

typedef uint64_t (*ub_bench_fn)(uint64_t iterations);

uint64_t ub_calibrate(const char* name, ub_bench_fn fn, uint64_t min_iterations);

#endif
