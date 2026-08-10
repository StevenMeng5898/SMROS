# AArch64 Zero-Warning Build Design

Date: 2026-08-10

## Status

Approved direction: remove every warning from the AArch64 release build through
targeted ownership cleanup and enforce the result with a warning-as-error build
gate. x86_64 and RISC-V64 cleanup is deferred until this AArch64 milestone is
complete.

## Context

The current command:

```sh
SMROS_LOGICAL_CPUS=4 cargo build --release \
  --target aarch64-unknown-none --message-format short
```

succeeds but emits 45 Rust warnings. The warnings fall into three categories:

1. `aarch64_vm_logic_shared.rs` is declared both by
   `kernel_lowlevel/mod.rs` and privately by `kernel_lowlevel/memory.rs`, so one
   source file has two independent module identities and inconsistent usage.
2. Pure model APIs retained for host tests and Verus are compiled into the
   bare-metal kernel even when the runtime does not consume them.
3. A small number of obsolete helpers, methods, constants, and fields no
   longer have any production, host-test, or proof consumer.

`kernel_lowlevel/memory.rs` also has a module-wide `allow(dead_code)` that can
hide future ownership mistakes. Adding more broad warning allowances would
make the build quiet without correcting these causes.

## Goals

This milestone must:

1. make the optimized `aarch64-unknown-none` kernel build succeed with all
   Rust warnings treated as errors;
2. keep AArch64 runtime behavior and public interfaces unchanged;
3. retain all model code required by host tests and Verus proofs;
4. remove code only after repository-wide reference checks show that it has no
   kernel, host-test, integration-test, or verification consumer;
5. remove the duplicate AArch64 VM module identity and the obsolete broad
   dead-code allowance associated with it; and
6. add a repeatable AArch64-only warning gate to the repository build
   workflow.

## Non-Goals

This milestone does not:

- change x86_64 or RISC-V64 code or require those targets to build cleanly;
- add a non-AArch64 futex implementation or fallback;
- change POSIX API behavior, test dispositions, or conformance claims;
- refactor shared runtime modules beyond the boundaries needed to classify the
  current warnings; or
- introduce crate-wide `allow(dead_code)`, `allow(unused)`, or equivalent
  suppression.

## Chosen Design

### Canonical AArch64 VM Module

`kernel_lowlevel/mod.rs` remains the single declaration site for
`aarch64_vm_logic_shared`. `kernel_lowlevel/memory.rs` imports the canonical
module through its parent instead of declaring the same file privately.

This gives every runtime consumer the same Rust module identity and removes
the artificial unused copy. The import change must preserve existing type and
function visibility; it must not duplicate types or introduce adapter
wrappers merely to retain old paths.

The module-local broad dead-code allowance in `memory.rs` is removed. Any
warning exposed by that removal is classified and corrected using the same
rules as the original 45 warnings.

### Runtime, Host-Test, And Proof Ownership

Functions and types used by the AArch64 kernel remain compiled for
`target_os = "none"`. Pure models used only by the host harness or Verus are
excluded from that kernel configuration at their smallest coherent boundary
while remaining available to both consumers.

Conditional compilation must follow existing SMROS patterns. A gate is placed
on a complete helper, implementation block, or model type when practical,
rather than scattered over individual statements. Imports follow the same
condition as the item using them so the fix cannot create replacement unused
import warnings.

Production logic must not be mislabeled as test-only simply because its
current runtime call path is incomplete. Before gating an item, the
implementation work records its consumers in:

- the AArch64 kernel module graph;
- `tests/host` unit and integration tests; and
- the relevant `verification` harnesses.

If an item is required by both runtime and models, it stays in the shared
module. If it is host/proof-only, it stays source-visible to those builds but
does not enter the bare-metal binary.

### Obsolete Code

An item is deleted only when repository-wide symbol and semantic searches find
no runtime, test, proof, generated-code, or documented extension consumer.
This applies to unused helpers such as stale range or bitmap utilities,
unused table methods, and fields that are written but never read.

When a field is part of an active data contract, the implementation either
uses it in the intended invariant or retains it under the correct ownership
gate. Renaming to an underscore or attaching a local `allow` is not an
acceptable substitute for deciding ownership. ABI-required parameters are the
only exception, and no such exception is currently expected in the AArch64
warning set.

### Warning Regression Gate

Add an AArch64-specific Make target that runs the release build with
`RUSTFLAGS` extended by `-D warnings` and `SMROS_LOGICAL_CPUS` set through the
existing Make configuration. The target must not modify warning policy for
x86_64 or RISC-V64.

The target becomes part of the AArch64 verification path and provides a
stable local and CI command. A normal successful build with visually empty
warning output is useful evidence, but the warning-as-error target is the
authoritative gate.

## Error Handling And Behavioral Safety

The cleanup must not replace missing runtime behavior with silent success or
new `ENOSYS` paths. Conditional compilation errors should fail during the
warning gate rather than be hidden by fallback stubs.

Removing or gating an item must preserve the same AArch64 symbol resolution,
return values, side effects, data layouts, and runtime paths for all reachable
code. Host-test or Verus compilation failures mean the ownership condition is
wrong and must be corrected, not suppressed.

## Verification

Implementation follows a warning-gate-first workflow:

1. Run the new warning-as-error target before cleanup and capture its expected
   failure against the current warning baseline.
2. Apply one warning-category cleanup at a time and rerun the focused build or
   affected host tests.
3. Run the final AArch64 release warning gate and require a successful exit
   with zero warnings.
4. Run the complete existing host test path with `make test`.
5. Run all wired proof harnesses with `make verus`.
6. Boot the AArch64 kernel through the existing smoke path with
   `make st ARCH=aarch64-unknown-none` and require the expected shell/smoke
   completion.
7. Run `git diff --check`.

The final evidence must record the commands and their exit status. A build
that succeeds only without `-D warnings`, a build containing a newly added
broad allowance, or a skipped host/proof/smoke gate does not satisfy this
milestone.

## Deferred Work

After this design is implemented and verified, x86_64 warning cleanup is the
next independent milestone, followed by RISC-V64. Their current
`linux_futex::wake_address` build failure and architecture-conditional
warnings are intentionally untouched here so the AArch64 result can be
established first.
