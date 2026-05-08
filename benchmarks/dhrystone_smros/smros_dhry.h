// Minimal C runtime and Linux syscall shim for running Dhrystone on SMROS.
// This keeps the port in C and avoids libc dependencies.

#ifndef SMROS_DHRY_H
#define SMROS_DHRY_H

#include <stddef.h>
#include <stdint.h>

#define DHRY_SYS_WRITE 64
#define DHRY_SYS_EXIT 93
#define DHRY_SYS_CLOCK_GETTIME 113

typedef struct {
  int64_t tv_sec;
  int64_t tv_nsec;
} dhry_timespec;

static inline uint64_t dhry_syscall6(uint64_t num, uint64_t arg0, uint64_t arg1,
                                     uint64_t arg2, uint64_t arg3,
                                     uint64_t arg4, uint64_t arg5) {
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

static inline int dhry_is_error(uint64_t value) { return (int64_t)value < 0; }

uint64_t dhry_now_ns(void);
void dhry_exit(int code);

size_t dhry_strlen(const char* value);
int dhry_strcmp(const char* left, const char* right);
char* dhry_strcpy(char* dest, const char* src);
void* dhry_memset(void* dest, int value, size_t count);
void* dhry_memcpy(void* dest, const void* src, size_t count);

void dhry_print(const char* value);
void dhry_print_char(char value);
void dhry_print_u64(uint64_t value);
void dhry_print_i64(int64_t value);
void dhry_print_result(const char* name, uint64_t count, const char* unit,
                       uint64_t elapsed_ns);

void* memset(void* dest, int value, size_t count);
void* memcpy(void* dest, const void* src, size_t count);
char* strcpy(char* dest, const char* src);
int strcmp(const char* left, const char* right);
size_t strlen(const char* value);

#endif
