// Minimal C ABI routines emitted by the optimizer for freestanding C++.

#include <stddef.h>

extern "C" void* memset(void* dest, int value, size_t count) {
  unsigned char* out = static_cast<unsigned char*>(dest);
  for (size_t i = 0; i < count; ++i) {
    out[i] = static_cast<unsigned char>(value);
  }
  return dest;
}

extern "C" void* memcpy(void* dest, const void* src, size_t count) {
  unsigned char* out = static_cast<unsigned char*>(dest);
  const unsigned char* in = static_cast<const unsigned char*>(src);
  for (size_t i = 0; i < count; ++i) {
    out[i] = in[i];
  }
  return dest;
}
