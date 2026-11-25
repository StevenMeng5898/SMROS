# Known issues

## pthread_cond_init/1-3 host reference is out of scope

Status: closed.

Closure: out of scope — host glibc + qemu-user environment.

The published `751b308` qemu-user reference failed
`conformance/interfaces/pthread_cond_init/1-3.c` in about 200 ms, with no
timeout token, and the text "The cond vars behave differently across
processes." Three repeats of the staged campaign binary were pass, fail,
pass. That binary's sha256 is
`a1f87d3014ca5d768688a6d4eaed6382fcb6086d7eec9115504f82c9e505ea6a`.
It was built with `VERBOSE` 1, so the wakeup-count lines are compiled out.

The raw `LD_DEBUG=bindings` log is
`verification/evidence/751b308560119ceb3de3571cfaf90787cd8de44d/posix-baseline-status-rerun/pthread_cond_init-1-3-ld-debug/ld-debug.stderr`.
`SHA256SUMS` in that directory checks with `sha256sum -c --strict`. The run
used the staged campaign binary, `qemu-aarch64-static`, and the baseline
environment: `PATH` from `os.defpath`, `LANG=C`, `LC_ALL=C`, a private
`TMPDIR`, `LD_LIBRARY_PATH=/lib:/usr/lib`, and no `LD_PRELOAD`. The guest
bindings for `pthread_cond_init`, `pthread_cond_wait`, `pthread_cond_signal`,
`pthread_cond_broadcast`, and `pthread_cond_destroy` name `/lib/libc.so.6`.
They do not name `libsmros-posix-compat.so`.

This host baseline does not cover the SMROS compat library's `pthread_cond_*`.
It must not be used as compat-library coverage evidence. SMROS's gate remains
guest `posixtest`.

A separate counting probe recompiled the existing source with `-DVERBOSE=2`
and did not edit the source or the 16 patches. That binary's sha256 is
`8e0fdcdff19c7b369f156acecf4118dabe28974f60cab9cf9374201f5e58e61c`. It is not
the campaign binary, so the 20 runs are a counting probe, not a timing
reproduction of the campaign failure. Raw stdout and stderr are in
`verification/evidence/751b308560119ceb3de3571cfaf90787cd8de44d/posix-baseline-status-rerun/pthread_cond_init-1-3-verbose/`.
The counts (reference, null attribute, default attribute) show that no side is
0. The test's pass/fail rule is a fragile exact-equality comparison: run 04
is 200/199/169 and passes, run 15 is 200/199/197 and passes, while failures
are short by 1 to 7 (200/200/199, 199/199/198, 200/200/196, 200/200/199,
200/198/200, 200/200/199, 200/200/193). Those numbers do not establish why a
count missed the reference.
