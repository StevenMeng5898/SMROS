# Open POSIX Test Suite source

SMROS uses the public Emscripten-maintained mirror of the Open POSIX Test
Suite. This mirror contains Emscripten changes in addition to the upstream
suite and targets IEEE Std 1003.1-2001 System Interfaces.

`source.lock.json` pins an immutable, full Git commit. The fetched source is
licensed under GPLv2 (`GPL-2.0-only`), and a checkout is valid only when its
`COPYING` file is present.

By default, the generated checkout lives below `target/posix`; `--work-dir`
may select another generated location. The checkout is not vendored into this
repository.

Checkout identity uses Git tree semantics. Validation independently derives
the expected tree from the pinned commit and captured patch bytes, then
compares it with a temporary Git index of the actual checkout.
`.smros-source.json` records the patch SHA-256 and expected Git tree OID for
diagnostics; it is not the source of truth. Git trees cover file paths and
contents, symlink targets, and the executable bit. They do not represent
empty directories or directory modes, so those are outside source identity.

## Patches

Patch filenames in `patches/series` are applied in listed order. Patches must
not weaken test assertions or turn nonzero test results into passes.

`replace-defective-fork-11-record-lock-test.patch` ports the maintained Linux
Test Project record-lock assertion from commit
`0b69550e055b5385822f001e2a27fedfbef31816`:

```text
https://raw.githubusercontent.com/linux-test-project/ltp/0b69550e055b5385822f001e2a27fedfbef31816/testcases/open_posix_testsuite/conformance/interfaces/fork/11-1.c
sha256: fcf9b794dd054586f65625ee6dd9a5daee61b98c1a43887de57e8c230a7d1626
```

The only compatibility adaptation changes LTP's `test_main` entry point to
the pinned suite's `main`, with explicit `(void)argc` and `(void)argv` casts.
The pinned `posixtest.h` does not define `PTS_ATTRIBUTE_UNUSED`; the maintained
record-lock checks and result propagation are otherwise unchanged.

Updating the pinned commit requires reviewing and regenerating the future
`stub-review.tsv` and `shell-review.tsv` classifications against the new
source.
