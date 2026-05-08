// Port of Fuchsia fuchsia_microbenchmarks time syscall cases to SMROS C++.

#include "perftest.h"
#include "smros_zircon.h"

namespace {

bool ClockGetMonotonic(perftest::RepeatState* state) {
  while (state->KeepRunning()) {
    const uint64_t value = smros::ClockGetMonotonic();
    if (smros::IsError(value)) {
      state->Fail(static_cast<int64_t>(value));
      return false;
    }
  }
  return true;
}

}  // namespace

bool RegisterTimeTests() {
  return perftest::RegisterTest("Time/ClockGetMonotonic", ClockGetMonotonic, 64);
}
