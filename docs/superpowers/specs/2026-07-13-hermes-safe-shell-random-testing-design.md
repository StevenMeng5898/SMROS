# Hermes Safe Shell and Random Testing Design

## Goal

Give the native SMROS Hermes agent access to safe shell capabilities and reproducible randomized test campaigns. Hermes must exercise guest commands and request the host test targets `ut`, `it`, and `st` without gaining an arbitrary command-execution channel.

## User Interface

- `hermes exec <command> [args...]` executes one allowed guest shell command.
- `hermes random [seed=<u64>] [iterations=<positive usize>]` selects and executes guest test operations from a safe catalog. The iteration count has no policy maximum; the report includes the effective seed so a finite campaign can be replayed.
- `hermes test-all [seed=<u64>] [iterations=<positive usize>]` runs the deterministic Hermes checks once, a randomized guest campaign for the requested iterations, and the named host jobs `ut`, `it`, and `st` once each.
- Existing `hermes info`, `hermes test`, `hermes skills`, `hermes ask`, `hermes ui`, and `hermes web` behavior remains compatible.

## Safety Model

Hermes uses a positive allowlist. Unknown commands, unknown subcommands, malformed arguments, excessive resource limits, interactive commands, and commands absent from the allowlist are denied before dispatch.

The following commands and forms are permanently forbidden to autonomous Hermes execution: `rm`, `kill`, `reboot`, `exit`, `clear`, `vi`, `run`, `write`, `mkdir`, `mv`, `cp`, `mount`, `vm -k`, `docker rm`, `docker stop`, and any equivalent destructive lifecycle operation added later. A dangerous command cannot be enabled by a runtime flag. Commands such as `vm` and `docker` are validated by subcommand; safe read-only operations such as `vm -s`, `docker images`, `docker ps -a`, `docker inspect`, and `docker logs` may be catalog entries. Random campaigns do not use network-mutating, filesystem-mutating, interactive, reboot, process-kill, VM-stop, or container-removal operations.

Command text, argument count and length, syscall-fuzzer iterations/time, output summaries, log size, and host-job duration remain bounded. Random campaign iterations must be positive and fit in the platform `usize`, but intentionally have no smaller policy maximum. The default random campaign remains small enough for the normal QEMU profile. Failures stop only the affected operation and are included in the final report.

## Architecture

### Guest Command Gateway

Expose a shell-owned Hermes gateway alongside the existing command registry. It parses a structured command request, looks up a policy record, validates command-specific arguments, and invokes the existing handler with the current `ShellContext`. This preserves one implementation of command behavior and avoids recursively feeding text into the interactive shell parser.

The gateway returns structured status metadata. Serial output remains visible normally; Hermes records a bounded command/result summary. Policy classification and random-catalog selection live in pure helper logic so host unit tests can exercise them without booting the kernel.

### Random Campaign Runner

Hermes owns a deterministic PRNG seeded by the user-provided seed or a generated runtime seed. Each catalog entry contains a safe structured command template, weight, and resource class. Selection produces concrete bounded arguments, including small `fuzzsc` seeds and iteration counts. The campaign parser rejects zero, malformed, duplicate, unknown, and platform-overflowing values, while accepting every positive iteration count representable as `usize`. A campaign report contains seed, requested and completed iterations, pass/fail/denied counts, selected commands, and failure summaries.

Reports are persisted below `/data/hermes/tests/` with a latest report plus bounded history. The existing Hermes audit/session facilities record tool use without storing unbounded command output.

### Host Test Jobs

Extend the existing host launcher protocol with a dedicated versioned test request. The guest sends only an enum-like job name: `ut`, `it`, or `st`; it cannot send shell source, Make variables, paths, or additional arguments. The Python launcher maps those names internally to fixed repository commands, applies one-job-at-a-time and timeout limits, captures bounded logs under `target/hermes-tests/`, and returns a structured status and short summary.

The launcher validates the request before starting a subprocess. Host job support remains loopback/QEMU-host scoped like the current VM protocol. A missing launcher or timed-out job is reported as a test failure and does not weaken guest command policy.

## Data Flow

For `hermes exec`, the shell parses the Hermes subcommand, the gateway validates the structured guest request, and the existing shell handler runs only after authorization. For `hermes random`, Hermes chooses a catalog entry from the seeded PRNG and sends the structured request through the same gateway. For `hermes test-all`, the native guest check runs once, the random campaign alone follows the requested iteration count, and then each of the three fixed host jobs is requested exactly once; results are combined and persisted.

No Gemma-generated text is executed directly. Natural-language `hermes ask` may recommend commands, but execution requires an explicit `hermes exec`, `hermes random`, or `hermes test-all` invocation and always passes through policy validation.

## Error Handling

Denials identify the rejected command class without echoing unsafe or oversized input. Guest handler failures, host transport failures, host nonzero exits, timeouts, and persistence failures have distinct report statuses. A partial campaign remains reproducible and records how many iterations completed.

## Testing

- Pure host unit tests cover allowlisted commands, permanent denials, nested `vm`/`docker` restrictions, argument bounds, acceptance above the former 64-iteration ceiling, rejection of zero and overflow, deterministic PRNG selection, and report accounting.
- Integration contract tests verify the shell gateway and Hermes subcommands are wired to the shared policy rather than an unrestricted parser.
- Integration contract tests also verify that `test-all` applies `iterations` only to the random campaign and invokes `ut`, `it`, and `st` once each.
- Python launcher tests cover accepted named jobs, rejection of arbitrary command text and extra fields, timeout behavior, bounded logs, and response parsing.
- Guest system tests run a short fixed-seed campaign and verify a forbidden-command probe is denied.
- Existing `ut`, `it`, `st`, architecture builds, and Hermes/testsc coverage remain regression gates.

## Scope Limits

This feature does not provide arbitrary host shell execution, autonomous scheduling, parallel host jobs, destructive recovery actions, direct execution of Gemma output, or automatic elevation of newly added shell commands. New commands require an explicit policy record and tests before Hermes can use them.
