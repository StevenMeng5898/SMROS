# POSIX AIO Stale-File Recovery Design

## Problem

The attached `posixtest all` output shows the same setup failure in
`aio_cancel/1-1.c`, `2-1.c`, `2-2.c`, `4-1.c`, `5-1.c`, `6-1.c`, `7-1.c`, and
`8-1.c`:

```text
Error at open(): File exists
```

These tests generate a process-specific file under `/tmp`, call `unlink()` to
remove residue from an earlier run, and then call `open()` with
`O_CREAT | O_RDWR | O_EXCL`. The pinned sources therefore depend on normal
POSIX namespace removal; changing the tests or dropping `O_EXCL` would weaken
their assertions.

The failing campaign used the POSIX stage built at SMROS commit `881880e`.
At that commit, `sys_unlinkat` validated its arguments and returned success but
did not remove the FxFS directory entry. The first execution on a fresh disk
could create its temporary file, but later executions on the persistent disk
could not remove it and received `EEXIST`.

Local `master` now contains commit `fac751c`, which connects `unlinkat` to FxFS,
removes the requested directory entry immediately, retains an unlinked inode
while an open description still references it, and reclaims it after the last
reference closes. A complete `aio_cancel` run at this tip on a copy of the
affected persistent disk produced no `File exists` errors, timeouts, restarts,
or positive resource deltas. However, the generated POSIX stage still carries
the old `881880e` provenance and fails the repository's `--verify-only` check.

## Scope

This work closes the stale-file failure class without weakening the Open POSIX
Test Suite. It includes:

- preserving the current real FxFS-backed `unlink`/`unlinkat` implementation;
- verifying namespace removal and open-inode lifetime through existing host
  tests;
- a same-disk repeated `aio_cancel` runtime regression that would fail with the
  old no-op `unlinkat` implementation;
- rebuilding the generated AArch64 POSIX stage so its manifest identifies the
  current SMROS source commit;
- rebuilding the AArch64 kernel with that verified stage; and
- recording exact pass, unresolved, timeout, restart, and resource-delta
  evidence.

The work does not clear `/tmp` between tests, assign each test a private
filesystem, patch test filenames, remove `O_EXCL`, synthesize PTS results, or
convert unrelated truthful AIO assertion failures into passes. It does not
modify the user's `smros-fxfs.img`; all runtime verification uses private disk
images under `target/posix/aarch64/`.

## Selected Approach

Keep the standards-correct namespace implementation and test it at the same
boundary used by the failure:

1. run the complete staged `aio_cancel` API on a fresh private disk;
2. boot again with the same private disk and run the same API a second time;
3. require both campaigns to finish without `EEXIST`, watchdog recovery, or
   resource growth; and
4. retain every unrelated test's genuine PTS result.

The first campaign creates the same process-specific temporary names used by
the upstream tests. Reusing the disk makes the second campaign a deterministic
regression for stale namespace entries. Under the old implementation, the
tests' cleanup calls returned success without removing their names, so the
second campaign failed at `open(O_EXCL)`. Under the selected implementation,
each cleanup removes the name while any already-open inode remains usable.

## Filesystem Semantics

`unlink` and `unlinkat(..., flags=0)` resolve the exact pathname, reject invalid
pointers and unsupported flags, and remove the matching FxFS directory entry.
The syscall reports `ENOENT` when no name exists; the AIO tests intentionally
ignore this result before their first creation. Directories are not removed by
ordinary `unlink`.

When the final name is removed, pathname lookup must immediately stop finding
the object. `open(..., O_CREAT | O_EXCL, ...)` can then publish a new object at
the same path. Existing descriptors continue to access the old inode through
their FxFS cursor until their last open reference is released. Reclamation is
therefore based on both link count and open-reference lifetime, not on the path
string retained for diagnostics.

Hard links remain correct: unlink removes only the selected parent/name entry,
decrements the inode link count, and retains the inode while another name or
open reference exists. This design does not add test-specific path handling.

## POSIX Stage Provenance

`host_shared/posixtest/` is generated and ignored state. It must be rebuilt
from the pinned revision and reviewed patch series only after all tracked
source and plan commits for this repair are complete. Its `manifest.json` and
`manifest.tsv` must agree, pass `--verify-only`, and report the exact current
SMROS commit rather than `881880e`.

The regenerated stage is embedded in the subsequent kernel build. Runtime
events may continue to identify the pinned upstream revision and patch digest,
but `smros_commit`, build identity, manifest checksum, and binary checksums must
all match the refreshed stage. No generated stage file is committed.

## Test-Driven Verification

The captured failing campaign is the behavioral RED evidence: eight AIO tests
returned `PTS_UNRESOLVED` because cleanup did not remove persistent names. The
current generated-stage verifier supplies a second reproducible RED check by
rejecting the stale `881880e` manifest metadata.

Before changing production code, focused host tests must confirm whether the
current unlink implementation already covers:

- decrementing link counts without underflow;
- retaining an unlinked inode while an open reference exists;
- reclaiming it after both its link and open-reference counts reach zero; and
- routing `sys_unlinkat` through FxFS rather than returning a stub success.

If these checks expose a missing semantic case, add the smallest failing host
regression first, observe its expected failure, and then make the minimal
production change. If they pass, no redundant production rewrite is made; the
remaining change is the stale generated stage followed by runtime regression
evidence.

## Verification Matrix

Acceptance requires:

- focused host tests for FxFS link/unlink and open-reference lifetime;
- Linux syscall integration contracts for real `linkat` and `unlinkat`
  routing;
- the complete host unit and integration suites;
- all configured Verus proof suites;
- formatting and warning-denied optimized AArch64 build checks;
- a freshly generated AArch64 POSIX stage followed by a successful
  `--verify-only` run;
- two complete `aio_cancel` campaigns using the same new private disk; and
- one recovery campaign using a private copy of the previously affected disk.

For each runtime campaign, every selected row must reach one genuine guest
`test_end`. The logs must contain zero occurrences of
`Error at open(): File exists`, no timeout, no QEMU restart, no fatal kernel or
loader marker, and no positive terminal resource delta. The focused cases
`1-1.c`, `2-1.c`, `2-2.c`, `4-1.c`, `5-1.c`, and `6-1.c` must pass. Other
cases retain their truthful results unless separately diagnosed; in
particular, this repair does not claim complete AIO conformance merely because
the stale-file setup error is gone.

Coverage and static-analysis evidence are regenerated when the repository's
tools are available. Coverity or coverage-tool unavailability is reported
explicitly with no fabricated success metric.

## Error Handling And Safety

All verification disk and output paths must be new or explicitly disposable
paths below `target/posix/aarch64/`. The repository root disk image is read only
for this work. Failed stage generation must not replace a verified stage with a
partial directory, and failed runtime attempts remain visible rather than
being retried until they pass.

The runtime result parser continues to distinguish PTS failure, unresolved,
unsupported, untested, launch failure, timeout, infrastructure failure, and
resource leakage. Removing the `EEXIST` setup failure cannot change those
classifications for unrelated assertions.

## Completion Criteria

The stale-file failure class is closed when the source commit and staged
manifest provenance agree, all static gates pass, the same-disk repeated API
regression completes twice, and the copied affected disk recovers without any
`File exists` setup error. Completion does not require unrelated
`aio_cancel` assertions to pass and does not authorize merging or modifying the
user's primary disk image.
