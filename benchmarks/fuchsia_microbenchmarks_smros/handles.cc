// Port of Fuchsia fuchsia_microbenchmarks handle operation cases to SMROS C++.

#include "perftest.h"
#include "smros_zircon.h"

namespace {

bool HandleDuplicateClose(perftest::RepeatState* state) {
  const uint64_t handle = smros::Syscall6(smros::kSysVmoCreate, 4096);
  if (smros::IsError(handle)) {
    state->Fail(static_cast<int64_t>(handle));
    return false;
  }

  while (state->KeepRunning()) {
    const uint64_t dup =
        smros::Syscall6(smros::kSysHandleDuplicate, handle, smros::kRightSameRights);
    if (smros::IsError(dup)) {
      state->Fail(static_cast<int64_t>(dup));
      return false;
    }

    const uint64_t close_status = smros::Syscall6(smros::kSysHandleClose, dup);
    if (smros::IsError(close_status)) {
      state->Fail(static_cast<int64_t>(close_status));
      return false;
    }
  }

  smros::Syscall6(smros::kSysHandleClose, handle);
  return true;
}

}  // namespace

bool RegisterHandleTests() {
  return perftest::RegisterTest("Handle/DuplicateClose", HandleDuplicateClose, 32);
}
