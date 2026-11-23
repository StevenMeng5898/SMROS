# Known issues

## pthread_cond_init/1-3 qemu-user reference is closed

Status: closed.

This is the qemu-user host reference, not the guest kernel futex path.
`LD_DEBUG=bindings` on the staged campaign binary resolves
`pthread_cond_init`, `pthread_cond_wait`, `pthread_cond_signal`, and
`pthread_cond_broadcast` to `libc.so.6`. The baseline runner does not set
`LD_PRELOAD`. `libsmros-posix-compat.so` is in the stage runtime snapshot and
was also copied into the sysroot used below, but it is not the library that
served these calls.

Phenomenon. The `751b308` campaign reference failed
`conformance/interfaces/pthread_cond_init/1-3.c` in about 200 ms, with no
timeout token, and the text "The cond vars behave differently across
processes." Three earlier repeats of the staged binary were pass, fail, pass.
The test compares wakeup counts of a NULL condattr, a default condattr, and a
process-shared reference. It fails when exactly one of the first two equals
the reference. The staged binary was built with the source default
`VERBOSE` 1, so the count lines are compiled out (`#if VERBOSE > 1`).

Classification. Scheduling-sensitive. The cause was unconfirmed on that
binary.

Reproduction of the count probe. The source and the 16 patches were not
modified. One throwaway binary was compiled with `-DVERBOSE=2` and then run
20 times. Its sha256 is
`8e0fdcdff19c7b369f156acecf4118dabe28974f60cab9cf9374201f5e58e61c`.
The compiler command and the per-run counts are in
`verification/logs/posix-baseline-status-rerun/pthread_cond_init-1-3-verbose/NOTES.txt`.
Raw stdout and stderr are beside that note. `SHA256SUMS` is checked with
`sha256sum -c --strict`.

```bash
aarch64-linux-gnu-gcc -std=gnu99 -O2 -D_POSIX_C_SOURCE=200112L \
  -D_XOPEN_SOURCE=600 -DVERBOSE=2 -pthread \
  -I scripts/posix/runtime/include \
  -I target/posix/src/85555325079ea362fa680bd2209c843cfe47e670/include \
  target/posix/src/85555325079ea362fa680bd2209c843cfe47e670/conformance/interfaces/pthread_cond_init/1-3.c \
  -o /tmp/pc13-verbose -lrt -lm
```

The runs use the baseline runner's private `/dev/mqueue` and
`qemu-aarch64-static -L` a sysroot that contains the same ld-linux,
`libc.so.6`, `libgcc_s.so.1`, and `libsmros-posix-compat.so` as the stage.

Close condition. Close this entry if those 20 runs show that every failure
still has all three wakeup counts near the expected 200, with the unequal
side short by a few rather than stuck at the pre-wait count 0, and if a run
can pass while both non-reference counts miss the reference. That pattern is
a non-queued `pthread_cond_signal` or `pthread_cond_broadcast` that arrived
between waits, which the test then treats as a difference. Keep the entry
open if a failure leaves one side at 0, or near 0, while the reference is
near 200.

Result. The condition is met, so this entry is closed. Seven of the 20 runs
failed. Their counts (reference, null, default) were 200/200/199,
199/199/198, 200/200/196, 200/200/199, 200/198/200, 200/200/199, and
200/200/193. The short side is 1 to 7 below the reference. Run 04 passed at
200/199/169: both non-reference counts missed 200, so the test's equality
check did not fail. The totals are not a per-signal trace of whether the
thread was inside `pthread_cond_wait` at the missed call.
