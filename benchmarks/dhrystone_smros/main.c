#include "smros_dhry.h"
#include "dhry.h"

#define DHRY_DEFAULT_RUNS 50000ull

int smros_dhrystone_main(uint64_t initial_stack) {
  (void)initial_stack;

  uint64_t runs = DHRY_DEFAULT_RUNS;

  dhry_print("Dhrystone 2.1 SMROS C port\n");
  dhry_print("source: BYTE UnixBench dhry_1.c/dhry_2.c/dhry.h\n");
  dhry_print("mode: one ELF, no libc, no Rust\n");
  dhry_print("runs: ");
  dhry_print_u64(runs);
  dhry_print("\n");

  uint64_t start = dhry_now_ns();
  uint64_t count = dhry_run(runs);
  uint64_t elapsed = dhry_now_ns() - start;

  dhry_print_result("Dhrystone", count, "lps", elapsed);
  if (elapsed != 0) {
    dhry_print("  dhrystones_per_second=");
    dhry_print_u64(count * 1000000000ull / elapsed);
    dhry_print("\n");
  } else {
    dhry_print("  timer: elapsed_ns=0, SMROS timer is too coarse for this run\n");
  }

  int failures = dhry_verify(count);
  if (failures == 0) {
    dhry_print("verify: PASS\n");
    dhry_print("dhrystone: PASS\n");
  } else {
    dhry_print("verify: FAIL ");
    dhry_print_i64(failures);
    dhry_print("\n");
    dhry_print("dhrystone: FAIL\n");
  }

  dhry_exit(failures == 0 ? 0 : 1);
  return failures == 0 ? 0 : 1;
}
