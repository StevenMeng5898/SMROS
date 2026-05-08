#include "perftest.h"
#include "smros_zircon.h"

bool RegisterChannelTests();
bool RegisterHandleTests();
bool RegisterTimeTests();
bool RegisterVmoTests();

extern "C" int smros_fuchsia_microbenchmarks_main(uint64_t initial_stack) {
  (void)initial_stack;
  bool registered = true;
  registered = RegisterChannelTests() && registered;
  registered = RegisterVmoTests() && registered;
  registered = RegisterHandleTests() && registered;
  registered = RegisterTimeTests() && registered;
  if (!registered) {
    smros::Print("fuchsia_microbenchmarks: registration failed\n");
    smros::Exit(1);
  }
  const int exit_code = perftest::RunAllTests();
  smros::Exit(exit_code);
  return exit_code;
}
