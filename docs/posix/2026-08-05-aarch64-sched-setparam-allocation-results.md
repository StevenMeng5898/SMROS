# AArch64 sched_setparam Allocation Results

Campaign date: 2026-08-05

## Scope

This focused AArch64 increment corrects four undersized Open POSIX Test Suite
allocations in `sched_setparam`. Each source allocated a byte count for an
`int *` array and then wrote one `int` per CPU. The reviewed patch multiplies
each element count by `sizeof(*child_pid)` and changes no assertion, result
code, or scheduling expectation.

This is not a POSIX conformance claim. The patched tests still expose missing
process and scheduling semantics, and the complete 1,598-test campaign has not
been rerun for this commit.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| SMROS implementation commit | `db6dd5a83dca3d88e1889b607b06f39d0aa52905` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Patch file SHA-256 | `993a10226cb2c265a55dacb20c1ee8c54a0ff4e3cdebe081c3227f62378cffea` |
| Canonical patch-series SHA-256 | `2354fdb550290652373cd831c7489300bdd20344aa74fe151d8b3cfe0d009724` |
| Canonical manifest SHA-256 | `93bac8ee254ebd8c5af0857d4526135ad5b069eee47835f82f5e50d241a52c12` |
| Manifest file SHA-256 | `a4882d9a352eacfbb865013300b6a2fee9215f67d261cd6cecc2cf9a6dfb81c1` |
| Canonical build-results SHA-256 | `ef58bb15baf69fc731bdb64810bec7a64ab8559a31e7c4152be4852858042e7a` |
| Build-results file SHA-256 | `269abf787a39f4d81922f7e54ee8200c2c9ab1087bbbcbd3a7728a4802e6267c` |
| Build ID | `5c01aeda7fc9a026f0ac45db5563a2a8a7430d18c14097c9aac4dd1ebed25cd4` |

The rebuilt stage retained the full inventory: 1,979 C sources discovered,
1,941 compile passes, 38 compile failures, 1,680 link passes, 2 link failures,
169 unported shell tests, 1,598 runnable tests, and 119,397,843 staged bytes.

## Focused Canaries

Each test ran in a fresh QEMU process against the private generated disk
`target/posix/aarch64/smros-fxfs-sched-allocation-db6dd5a.img`. No run used
the repository-root `smros-fxfs.img`.

| Test | Previous | Patched | Duration | Resource evidence |
| --- | --- | --- | ---: | --- |
| `sched_setparam/2-1.c` | unresolved | unresolved | 90 ms | measured zero |
| `sched_setparam/2-2.c` | unresolved | unresolved | 111 ms | measured zero |
| `sched_setparam/9-1.c` | timeout | fail | 192 ms | measured zero |
| `sched_setparam/10-1.c` | timeout | fail | 241 ms | measured zero |

The two unresolved tests now terminate after `kill()` reports that the modeled
child process does not exist. The two previous timeout tests now reach their
real assertions: `9-1.c` reports that the target process does not preempt the
caller, and `10-1.c` reports that the caller does not relinquish the processor.

| Result directory | Results SHA-256 | Serial SHA-256 |
| --- | --- | --- |
| `target/posix/aarch64/smros-run-sched-allocation-db6dd5a-1/` | `f26a925f78532d27205aa06ad9947ea54b00c03506c386b4479f46e87b346274` | `69201089428c80ebb424bd6a19cfc8fdbcbfd1766ae77b13d1a39b0958050bae` |
| `target/posix/aarch64/smros-run-sched-allocation-db6dd5a-2/` | `5ee54ecfc79b0ed16a3dddaf7272fa1b406fd1dc0549ab2825d2d324f96cbc7e` | `f20e9b3ad406e5d1259a66c32f05ae3f8cfbb00301fb628f67a88f3b821ea54a` |
| `target/posix/aarch64/smros-run-sched-allocation-db6dd5a-3/` | `18b28e5c7c82ed621aba47ad3478f9ece477293f86ec284f81a74bec67fa816b` | `aa1ab388c19b5652afea791b05c41cc31363297f7a99ef9613b54fea7045aee3` |
| `target/posix/aarch64/smros-run-sched-allocation-db6dd5a-4/` | `8711a81bcb68437240c4608ebd82d70db694c32532bee861894c4315c6fecb28` | `c99de40749839255dc471070a6301fc7c1223d8ac6e824c70670dcf8c87e1992` |

## Scheduling Group

The complete scheduling group ran after the four canaries.

| Status | Previous | Patched | Change |
| --- | ---: | ---: | ---: |
| pass | 26 | 26 | 0 |
| fail | 11 | 13 | +2 |
| unresolved | 4 | 4 | 0 |
| unsupported | 20 | 20 | 0 |
| untested | 6 | 6 | 0 |
| timeout | 2 | 0 | -2 |
| QEMU restarts | 2 | 0 | -2 |

All 69 selected tests completed, all 69 resource snapshots were measured, and
all resource deltas were zero. The results and serial hashes are:

- Results: `ecc33dea34eaafb02df2db9731bcabca3c5c315c9ea5b3a55fe0cf027b2240ee`
- Serial: `086407991bd7991e46268a03a4009852c6c296a0f6ecf144b4811899f9eea048`

The serial logs contain no `malloc(): corrupted top size`, `Kernel panic`,
`Fatal glibc error`, `failed to map segment`, or
`cannot create shared object descriptor` marker.

## Verification And Remaining Work

The regression check failed before the patch because the new reviewed patch
was absent, then passed after the exact four replacements were added. The full
POSIX host tooling suite passed 469 tests. Patch application, stage
verification, the AArch64 release build, all four canaries, and the complete
scheduling group also completed successfully.

This focused increment did not rerun Tarpaulin, Verus coverage, or Coverity.
The last merged quality baseline remains 99.09% Tarpaulin coverage below the
required 100%, 23 Verus coverage-audit findings, and unavailable Coverity
commands. None of those gates is claimed as improved here.

The next semantic foundation is real forked process state, address-space
separation or copy-on-write, child lifecycle and wait status, and shared-memory
visibility between processes. The 20 unsupported optional scheduling tests
also remain required work.
