# Run ELF Launch Identity Design

## Goal

Prevent delayed work from an older ELF launch from mutating, completing, or
releasing a newer launch while preserving reentrant observer callbacks.

## Root Cause

`RunElfLifecycleState` currently applies attachment, prepare-return, clear, and
completion operations to whichever request is active. Async launcher work does
not carry the identity of the request that created it. The AArch64 return path
also redirects `ELR_EL1` to a global resume function without carrying request
identity, because `sys_exit` currently returns zero in `x0`.

## Lifecycle State

Introduce a nonzero typed `RunElfLaunchId` allocated monotonically by
`RunElfLifecycleState`. Successful admission returns the ID. Allocation uses
checked arithmetic, never wraps, and permanently fails closed after issuing the
last representable ID.

Every mutating lifecycle operation accepts an expected ID. A matching operation
may reset signal/timer state, attach or take a resource, or remove the request.
A stale, repeated, or missing operation returns an explicit nonterminal result
without invoking the reset hook or changing the active request. The state keeps
the last completed ID so a duplicate terminal operation can be distinguished
from unrelated stale work.

## Launcher Binding

Pin each ELF launcher thread to the physical CPU on which it is created. Before
the new thread becomes eligible, bind its launch ID into a per-CPU atomic slot.
The slot holds the ID through loader preparation and EL0 execution. Binding and
clearing validate CPU bounds and use compare-exchange, so old launch A cannot
clear a slot already rebound to launch B.

The launcher entry captures the bound ID and uses it for request cloning,
resource attachment, loader failure, and completion. Every synchronous failure
clears only its expected per-CPU binding. A matched terminal path clears its
binding and releases its resource before dispatching the callback, allowing a
reentrant launch B to bind the same CPU safely.

## Resume Token

`prepare_run_elf_return` reads the current pinned CPU binding, validates the
nonzero ID against the active lifecycle state, and performs matched prepare-return.
It returns the raw ID to `sys_exit`. The AArch64 exception path already stores
the syscall result in saved `x0`, restores `x0`, and then executes `eret` to the
EL1 resume address. Therefore the resume function becomes:

```rust
pub extern "C" fn run_elf_launcher_resume(id_raw: usize) -> !
```

The resume function rejects zero or non-representable IDs, reconstructs the
typed ID, and completes only that launch. All supported targets use 64-bit
`usize`; the production contract and integration test pin this requirement.

## Error Handling

- Busy admission returns the original request unchanged.
- Exhausted launch identity allocation fails closed and never reuses an ID.
- Stale prepare-return, clear, attachment, loader failure, and completion are
  nonterminal and do not reset state or dispatch callbacks.
- Rejected attachment drops only the unattached resource supplied by the stale
  operation.
- Matched completion moves and releases the active resource before callback.

## Verification

Production-shared tests execute A completion followed by B admission and then
inject stale A completion, attachment, loader failure, and prepare-return. They
verify B remains active, B's timer is not reset, and A/B resources are released
exactly once. Separate tests cover launch-ID exhaustion and per-CPU binding
bounds/compare-exchange behavior. An integration contract verifies the
`sys_exit` result is preserved through saved `x0` into the resume function ID
argument.
