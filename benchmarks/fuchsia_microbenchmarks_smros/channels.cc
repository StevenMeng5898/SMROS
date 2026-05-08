// Port of the Fuchsia fuchsia_microbenchmarks Zircon channel cases to SMROS.
//
// The upstream suite lives under Fuchsia's src/tests/microbenchmarks and was
// previously named zircon_benchmarks.  The original channel benchmark shape is
// preserved here: create/close a channel pair and write/read messages through
// the two channel endpoints.  The syscall bindings are supplied by
// smros_zircon.h instead of libzircon.

#include "perftest.h"
#include "smros_zircon.h"

namespace {

bool ChannelCreateClose(perftest::RepeatState* state) {
  while (state->KeepRunning()) {
    const uint64_t pair = smros::Syscall6(smros::kSysChannelCreate);
    if (smros::IsError(pair)) {
      state->Fail(static_cast<int64_t>(pair));
      return false;
    }

    const uint32_t h0 = static_cast<uint32_t>(pair >> 32);

    // SMROS currently removes the whole modeled channel object when either
    // endpoint is closed, so closing h0 is the cleanup operation for the pair.
    const uint64_t close_status = smros::Syscall6(smros::kSysHandleClose, h0);
    if (smros::IsError(close_status)) {
      state->Fail(static_cast<int64_t>(close_status));
      return false;
    }
  }
  return true;
}

template <size_t kMessageSize>
bool ChannelWriteRead(perftest::RepeatState* state) {
  const uint64_t pair = smros::Syscall6(smros::kSysChannelCreate);
  if (smros::IsError(pair)) {
    state->Fail(static_cast<int64_t>(pair));
    return false;
  }

  const uint32_t h0 = static_cast<uint32_t>(pair >> 32);
  const uint32_t h1 = static_cast<uint32_t>(pair);
  char message[kMessageSize == 0 ? 1 : kMessageSize] = {};
  char out[kMessageSize == 0 ? 1 : kMessageSize] = {};
  for (size_t i = 0; i < sizeof(message); ++i) {
    message[i] = static_cast<char>('A' + (i % 26));
  }

  while (state->KeepRunning()) {
    const uint64_t write_status = smros::Syscall6(
        smros::kSysChannelWrite, h0, 0, reinterpret_cast<uint64_t>(message), kMessageSize, 0, 0);
    if (smros::IsError(write_status)) {
      state->Fail(static_cast<int64_t>(write_status));
      return false;
    }

    const uint64_t bytes_read = smros::Syscall6(
        smros::kSysChannelRead, h1, 0, reinterpret_cast<uint64_t>(out), kMessageSize, 0, 0);
    if (smros::IsError(bytes_read)) {
      state->Fail(static_cast<int64_t>(bytes_read));
      return false;
    }
  }

  smros::Syscall6(smros::kSysHandleClose, h0);
  return true;
}

}  // namespace

bool RegisterChannelTests() {
  return perftest::RegisterTest("Channel/CreateClose", ChannelCreateClose, 32) &&
         perftest::RegisterTest("Channel/WriteRead/64bytes/0handles", ChannelWriteRead<64>, 32) &&
         perftest::RegisterTest("Channel/WriteRead/1024bytes/0handles", ChannelWriteRead<1024>,
                                16);
}
