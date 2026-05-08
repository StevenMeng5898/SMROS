// C++ SMROS shim for the Fuchsia Zircon microbenchmark port.
//
// SMROS dispatches Linux syscalls below 1000 and modeled Zircon syscalls at
// 1000 + zx syscall ordinal.  This header intentionally keeps the benchmark
// code in C++ while avoiding libc/libstdc++ dependencies that the current SMROS
// ELF runner cannot load from /shared.

#pragma once

#include <stddef.h>
#include <stdint.h>

namespace smros {

constexpr uint64_t kZirconBase = 1000;

constexpr uint64_t kSysLinuxExit = 93;

constexpr uint64_t kSysClockGetMonotonic = kZirconBase + 2;
constexpr uint64_t kSysHandleClose = kZirconBase + 6;
constexpr uint64_t kSysHandleDuplicate = kZirconBase + 8;
constexpr uint64_t kSysChannelCreate = kZirconBase + 20;
constexpr uint64_t kSysChannelRead = kZirconBase + 21;
constexpr uint64_t kSysChannelWrite = kZirconBase + 23;
constexpr uint64_t kSysVmoCreate = kZirconBase + 68;
constexpr uint64_t kSysVmoRead = kZirconBase + 69;
constexpr uint64_t kSysVmoWrite = kZirconBase + 70;
constexpr uint64_t kSysDebugWrite = kZirconBase + 96;

constexpr uint32_t kRightSameRights = 0x80000000u;

inline uint64_t Syscall6(uint64_t num, uint64_t arg0 = 0, uint64_t arg1 = 0,
                         uint64_t arg2 = 0, uint64_t arg3 = 0, uint64_t arg4 = 0,
                         uint64_t arg5 = 0) {
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

inline bool IsError(uint64_t value) { return static_cast<int64_t>(value) < 0; }

inline size_t StringLength(const char* value) {
  size_t len = 0;
  while (value[len] != 0) {
    ++len;
  }
  return len;
}

inline void PrintBytes(const char* data, size_t len) {
  Syscall6(kSysDebugWrite, reinterpret_cast<uint64_t>(data), len);
}

inline void Print(const char* value) { PrintBytes(value, StringLength(value)); }

inline void PrintChar(char value) { PrintBytes(&value, 1); }

inline void PrintUint(uint64_t value) {
  char buffer[32];
  size_t len = 0;
  if (value == 0) {
    PrintChar('0');
    return;
  }
  while (value != 0 && len < sizeof(buffer)) {
    buffer[len++] = static_cast<char>('0' + (value % 10));
    value /= 10;
  }
  while (len != 0) {
    PrintChar(buffer[--len]);
  }
}

inline void PrintInt(int64_t value) {
  if (value < 0) {
    PrintChar('-');
    PrintUint(static_cast<uint64_t>(-value));
  } else {
    PrintUint(static_cast<uint64_t>(value));
  }
}

inline uint64_t ClockGetMonotonic() { return Syscall6(kSysClockGetMonotonic); }

inline void Exit(int code) {
  Syscall6(kSysLinuxExit, static_cast<uint64_t>(code));
  for (;;) {
    asm volatile("wfe");
  }
}

}  // namespace smros
