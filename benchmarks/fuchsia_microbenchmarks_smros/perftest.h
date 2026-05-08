// Tiny C++ perftest-compatible harness for the SMROS Fuchsia microbenchmark
// port.  Fuchsia's real suite uses src/performance/lib/perftest; this keeps
// the same "RegisterTest + RepeatState" style but strips it down for the
// current SMROS C++ ELF runner.

#pragma once

#include <stddef.h>
#include <stdint.h>

#include "smros_zircon.h"

namespace perftest {

class RepeatState {
 public:
  explicit RepeatState(uint32_t max_iterations)
      : max_iterations_(max_iterations), iteration_(0), failed_(false) {}

  bool KeepRunning() {
    if (failed_ || iteration_ >= max_iterations_) {
      return false;
    }
    ++iteration_;
    return true;
  }

  void Fail(int64_t status) {
    failed_ = true;
    status_ = status;
  }

  uint32_t iterations() const { return iteration_; }
  bool failed() const { return failed_; }
  int64_t status() const { return status_; }

 private:
  uint32_t max_iterations_;
  uint32_t iteration_;
  bool failed_;
  int64_t status_ = 0;
};

using TestFunc = bool (*)(RepeatState*);

struct Test {
  const char* name;
  TestFunc func;
  uint32_t iterations;
};

class Registry {
 public:
  bool Add(const char* name, TestFunc func, uint32_t iterations) {
    if (count_ >= kMaxTests) {
      return false;
    }
    tests_[count_++] = Test{name, func, iterations};
    return true;
  }

  const Test* tests() const { return tests_; }
  size_t count() const { return count_; }

 private:
  static constexpr size_t kMaxTests = 32;
  Test tests_[kMaxTests] = {};
  size_t count_ = 0;
};

inline Registry& GlobalRegistry() {
  static Registry registry;
  return registry;
}

inline bool RegisterTest(const char* name, TestFunc func, uint32_t iterations) {
  return GlobalRegistry().Add(name, func, iterations);
}

inline void PrintResult(const char* name, const RepeatState& state, uint64_t start,
                        uint64_t end) {
  const uint64_t elapsed = end >= start ? end - start : 0;
  smros::Print("  ");
  smros::Print(name);
  smros::Print(": ");
  smros::PrintUint(state.iterations());
  smros::Print(" iterations in ");
  smros::PrintUint(elapsed);
  smros::Print(" ns, ");
  smros::PrintUint(state.iterations() == 0 ? 0 : elapsed / state.iterations());
  smros::Print(" ns/iter");
  if (state.failed()) {
    smros::Print(" FAIL ");
    smros::PrintInt(state.status());
  }
  smros::Print("\n");
}

inline int RunAllTests() {
  const Registry& registry = GlobalRegistry();
  int failures = 0;

  smros::Print("fuchsia_microbenchmarks SMROS C++ port\n");
  smros::Print("suite: zircon_benchmarks compatibility subset\n");
  smros::Print("base syscall: 1000\n");

  for (size_t i = 0; i < registry.count(); ++i) {
    const Test& test = registry.tests()[i];
    RepeatState state(test.iterations);
    const uint64_t start = smros::ClockGetMonotonic();
    const bool ok = test.func(&state);
    const uint64_t end = smros::ClockGetMonotonic();
    if (!ok || state.failed()) {
      ++failures;
    }
    PrintResult(test.name, state, start, end);
  }

  if (failures == 0) {
    smros::Print("fuchsia_microbenchmarks: PASS\n");
  } else {
    smros::Print("fuchsia_microbenchmarks: FAIL ");
    smros::PrintUint(static_cast<uint64_t>(failures));
    smros::Print("\n");
  }

  return failures == 0 ? 0 : 1;
}

}  // namespace perftest
