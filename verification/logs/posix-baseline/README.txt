posix-baseline is the Linux qemu-user reference. It does not run the SMROS kernel.

The 1395/205 per-test file was overwritten before it was committed. Those
rows cannot be reconstructed. run-1364 is the earlier saved capture
(pthread_create/10-1.c is SIGSEGV, status crash, empty stdout).

run-1358.ndjson is the old classifier output. Its 10-1 row says pass while
stdout is "Test FAILED: Did not receive segmentation fault".
run-1358-reclassified.ndjson applies classify_status and _pts_status to the
stored exit code, signal, timeout, and stdout. 10-1 is fail/fail there.

run-1365.ndjson is the 1365/235 campaign capture from
target/posix/aarch64/linux-reference/results.ndjson, with the same
reclassifier applied. 10-1 in that run is signal 11, status crash, pts_status
empty, and the stdout is empty. It is not the 1358 pass row.

run-1363.ndjson is the posix-baseline file written by the new classifier
during the f4af8fc guest campaign (summary count 1363/237). pthread_create/10-1.c
in that run exited by signal 11, so status is crash and stdout is empty.
It is not the run-1358 row.
