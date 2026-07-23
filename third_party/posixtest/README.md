# Open POSIX Test Suite source

SMROS uses the public Emscripten-maintained mirror of the Open POSIX Test
Suite. This mirror contains Emscripten changes in addition to the upstream
suite and targets IEEE Std 1003.1-2001 System Interfaces.

`source.lock.json` pins an immutable, full Git commit. The fetched source is
licensed under GPLv2 (`GPL-2.0-only`), and a checkout is valid only when its
`COPYING` file is present.

The generated checkout lives below `target/posix` and is not vendored into
this repository.

## Patches

Patch filenames in `patches/series` are applied in listed order. Patches must
not weaken test assertions or turn nonzero test results into passes.

Updating the pinned commit requires reviewing and regenerating the future
`stub-review.tsv` and `shell-review.tsv` classifications against the new
source.
