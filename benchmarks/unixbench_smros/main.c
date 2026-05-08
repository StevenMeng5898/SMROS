#include "smros_unix.h"

int ub_syscall_main(void);
int ub_pipe_main(void);
int ub_fstime_main(void);
int ub_arith_main(void);
int ub_hanoi_main(void);

int smros_unixbench_main(uint64_t initial_stack) {
  (void)initial_stack;

  ub_print("UnixBench SMROS C port\n");
  ub_print("source: BYTE UnixBench system benchmark subset\n");
  ub_print("mode: one ELF, no shell/perl/fork orchestration\n");

  int failures = 0;
  failures += ub_syscall_main();
  failures += ub_pipe_main();
  failures += ub_fstime_main();
  failures += ub_arith_main();
  failures += ub_hanoi_main();

  if (failures == 0) {
    ub_print("unixbench: PASS\n");
  } else {
    ub_print("unixbench: FAIL ");
    ub_print_u64((uint64_t)failures);
    ub_print("\n");
  }

  ub_exit(failures == 0 ? 0 : 1);
  return failures == 0 ? 0 : 1;
}
