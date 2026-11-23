posix-baseline is the Linux qemu-user reference. It does not run the SMROS kernel.
Totals seen on this host: 1397/203, 1395/205, 1364/236, 1358/242.
The 1397 and 1395 per-test files were overwritten before they were committed,
so the exact 31 names from 1395 to 1364 are not recoverable.
run-1364 and run-1358 are the two saved runs. DIFF-1364-to-1358.txt lists
the 10 tests whose status changed. Seven passed cases became unresolved,
crash, or fail; one crash became pass. Net pass count moved by 6.
The flips are aio, mq_close, pthread, and sigaction results under qemu-user,
including a 30s timeout and signal 11/6 crashes. That is reference variance,
not a guest regression.
