#include "smros_dhry.h"

uint64_t dhry_now_ns(void) {
  dhry_timespec ts;
  uint64_t status = dhry_syscall6(DHRY_SYS_CLOCK_GETTIME, 1, (uint64_t)&ts, 0,
                                  0, 0, 0);
  if (dhry_is_error(status)) {
    return 1;
  }
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

void dhry_exit(int code) {
  dhry_syscall6(DHRY_SYS_EXIT, (uint64_t)code, 0, 0, 0, 0, 0);
  for (;;) {
    asm volatile("wfe");
  }
}

size_t dhry_strlen(const char* value) {
  size_t len = 0;
  while (value[len] != 0) {
    ++len;
  }
  return len;
}

int dhry_strcmp(const char* left, const char* right) {
  while (*left != 0 && *left == *right) {
    ++left;
    ++right;
  }
  return (unsigned char)*left - (unsigned char)*right;
}

char* dhry_strcpy(char* dest, const char* src) {
  char* out = dest;
  while ((*out++ = *src++) != 0) {
  }
  return dest;
}

void* dhry_memset(void* dest, int value, size_t count) {
  unsigned char* out = (unsigned char*)dest;
  for (size_t i = 0; i < count; ++i) {
    out[i] = (unsigned char)value;
  }
  return dest;
}

void* dhry_memcpy(void* dest, const void* src, size_t count) {
  unsigned char* out = (unsigned char*)dest;
  const unsigned char* in = (const unsigned char*)src;
  for (size_t i = 0; i < count; ++i) {
    out[i] = in[i];
  }
  return dest;
}

void dhry_print(const char* value) {
  dhry_syscall6(DHRY_SYS_WRITE, 1, (uint64_t)value, dhry_strlen(value), 0, 0,
                0);
}

void dhry_print_char(char value) {
  dhry_syscall6(DHRY_SYS_WRITE, 1, (uint64_t)&value, 1, 0, 0, 0);
}

void dhry_print_u64(uint64_t value) {
  char buf[32];
  size_t len = 0;
  if (value == 0) {
    dhry_print_char('0');
    return;
  }
  while (value != 0 && len < sizeof(buf)) {
    buf[len++] = (char)('0' + (value % 10));
    value /= 10;
  }
  while (len != 0) {
    dhry_print_char(buf[--len]);
  }
}

void dhry_print_i64(int64_t value) {
  if (value < 0) {
    dhry_print_char('-');
    dhry_print_u64((uint64_t)(-value));
  } else {
    dhry_print_u64((uint64_t)value);
  }
}

void dhry_print_result(const char* name, uint64_t count, const char* unit,
                       uint64_t elapsed_ns) {
  dhry_print("  ");
  dhry_print(name);
  dhry_print(": COUNT|");
  dhry_print_u64(count);
  dhry_print("|1|");
  dhry_print(unit);
  dhry_print(" elapsed_ns=");
  dhry_print_u64(elapsed_ns);
  dhry_print(" per_iter_ns=");
  dhry_print_u64(count == 0 ? 0 : elapsed_ns / count);
  dhry_print("\n");
}

void* memset(void* dest, int value, size_t count) {
  return dhry_memset(dest, value, count);
}

void* memcpy(void* dest, const void* src, size_t count) {
  return dhry_memcpy(dest, src, count);
}

char* strcpy(char* dest, const char* src) { return dhry_strcpy(dest, src); }

int strcmp(const char* left, const char* right) {
  return dhry_strcmp(left, right);
}

size_t strlen(const char* value) { return dhry_strlen(value); }
