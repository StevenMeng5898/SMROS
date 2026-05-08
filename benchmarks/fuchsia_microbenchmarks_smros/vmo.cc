// Port of Fuchsia fuchsia_microbenchmarks VMO cases to SMROS C++.

#include "perftest.h"
#include "smros_zircon.h"

namespace {

bool VmoCreateClose(perftest::RepeatState* state) {
  while (state->KeepRunning()) {
    const uint64_t handle = smros::Syscall6(smros::kSysVmoCreate, 4096);
    if (smros::IsError(handle)) {
      state->Fail(static_cast<int64_t>(handle));
      return false;
    }

    const uint64_t close_status = smros::Syscall6(smros::kSysHandleClose, handle);
    if (smros::IsError(close_status)) {
      state->Fail(static_cast<int64_t>(close_status));
      return false;
    }
  }
  return true;
}

template <size_t kTransferSize>
bool VmoWriteRead(perftest::RepeatState* state) {
  const uint64_t handle = smros::Syscall6(smros::kSysVmoCreate, 4096);
  if (smros::IsError(handle)) {
    state->Fail(static_cast<int64_t>(handle));
    return false;
  }

  char input[kTransferSize] = {};
  char output[kTransferSize] = {};
  for (size_t i = 0; i < kTransferSize; ++i) {
    input[i] = static_cast<char>('a' + (i % 26));
  }

  while (state->KeepRunning()) {
    const uint64_t write_status = smros::Syscall6(
        smros::kSysVmoWrite, handle, reinterpret_cast<uint64_t>(input), kTransferSize, 0);
    if (smros::IsError(write_status)) {
      state->Fail(static_cast<int64_t>(write_status));
      return false;
    }

    const uint64_t read_status = smros::Syscall6(
        smros::kSysVmoRead, handle, reinterpret_cast<uint64_t>(output), kTransferSize, 0);
    if (smros::IsError(read_status)) {
      state->Fail(static_cast<int64_t>(read_status));
      return false;
    }
  }

  smros::Syscall6(smros::kSysHandleClose, handle);
  return true;
}

}  // namespace

bool RegisterVmoTests() {
  return perftest::RegisterTest("VMO/CreateClose/4K", VmoCreateClose, 32) &&
         perftest::RegisterTest("VMO/WriteRead/32bytes", VmoWriteRead<32>, 32) &&
         perftest::RegisterTest("VMO/WriteRead/128bytes", VmoWriteRead<128>, 32);
}
