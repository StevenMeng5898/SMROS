# POSIX Guest Live Coverage Design

## Purpose

Make a long-running `posixtest` campaign report useful, truthful coverage
progress on the SMROS serial console. The guest must show how many selected
tests, APIs, and groups have completed and passed without claiming that partial
execution proves full POSIX compliance.

This work targets the existing AArch64 runner. It does not change the staged
suite, add POSIX APIs, or alter the host report's build and program coverage
rules.

## Reporting Boundary

The guest reports coverage of the current manifest selection only. The host
report remains authoritative for source inventory, build coverage, execution
coverage, pass coverage, optional-group completion, provenance, and the final
compliance decision. Guest output must call its measurements selection
coverage, not POSIX compliance.

`SMROS_POSIX_EVENT` schema 1 remains unchanged. Live coverage is emitted as
ordinary `posixtest:` serial text so existing strict event parsers and campaign
controllers continue to accept the event stream without a protocol migration.

## Coverage Model

Create a bounded coverage tracker when the runner constructs its selected test
vector. For every distinct selected API and group, the tracker stores:

- the unit name;
- the number of selected tests in the unit;
- the number of terminal results recorded for the unit; and
- whether every recorded result is `pass`.

The existing manifest limit of 4,096 tests bounds the tracker. Its correctness
must not depend on manifest tests for an API or group being contiguous.

The tracker exposes a snapshot containing:

- completed and selected test counts;
- complete and selected API counts;
- passing and selected API counts;
- complete and selected group counts;
- passing and selected group counts; and
- pass, fail, unresolved, unsupported, untested, and launch-error test counts.

A test becomes complete only after its terminal status has been recorded and
its `test_end` event emitted. An API or group is complete when all selected
tests assigned to it are complete. An API or group passes only when it is
complete and every selected test assigned to it passed. `fail`, `unresolved`,
`unsupported`, `untested`, and `launch-error` all prevent a containing unit
from counting as passed.

The tracker update must succeed exactly once for each completed test. A missing
unit, count overflow, over-completion, or mismatch between runner completion
and coverage completion is a runner invariant failure and terminates the run
with an `infrastructure_error`; SMROS must not print invented coverage.

## Percentage Semantics

All percentages have two decimal places and use integer arithmetic. Calculate
hundredths of a percent as `numerator * 10_000 / denominator`; the existing
manifest bounds make the multiplication safe. A zero denominator prints
`0.00%`. A complete nonempty denominator prints `100.00%`.

The displayed ratios are:

- test execution progress: completed selected tests / selected tests;
- API completion coverage: complete selected APIs / selected APIs;
- API pass coverage: passing selected APIs / selected APIs;
- group completion coverage: complete selected groups / selected groups; and
- group pass coverage: passing selected groups / selected groups.

These percentages are never described as the percentage of the POSIX standard
implemented by SMROS.

## Serial Output

Immediately after `suite_start`, print a selection summary:

```text
posixtest: selection tests=1598 apis=195 groups=9 interval=25 scope=selected
```

After a terminal test result, print one progress line when any of these
conditions is true:

- the completed test count is a multiple of 25;
- the result completes an API; or
- the result completes the suite.

Multiple true conditions still produce one line. The line follows the
corresponding `test_end` event and precedes the next `test_start`. The suite's
last progress line precedes `suite_end`.

```text
posixtest: progress tests=25/1598 (1.56%) apis-complete=3/195 (1.53%) apis-pass=2/195 (1.02%) groups-complete=0/9 (0.00%) groups-pass=0/9 (0.00%) pass=23 fail=1 unresolved=1 unsupported=0 untested=0 launch-errors=0 scope=selected
```

Percentages in examples follow the integer rule, so values are truncated to
two decimal places rather than rounded. Both launched tests and reviewed
not-launched upstream stubs use the same completion path for reporting.

`posixtest status` includes the same coverage snapshot fields when a run is
active. Its existing run identity, filter, current test, and result totals
remain available. An idle status has zero counts and no stale result from a
previous run.

## Component Changes

`src/user_level/services/posix_test_logic_shared.rs` owns pure, host-testable
coverage decisions: percentage calculation, unit state transitions, snapshots,
and progress-trigger selection. These functions contain no serial or scheduler
dependencies.

`src/user_level/services/posix_test.rs` constructs and stores the bounded
tracker in `RunnerState`, updates it alongside terminal status recording,
emits selection/progress text, exposes the snapshot through
`PosixRunnerStatus`, and converts tracker invariant failures into the existing
infrastructure-error path.

`src/user_level/services/user_shell.rs` extends `posixtest status` formatting to
show the shared snapshot fields.

The strict Python event parser, event schema, and report format do not change.
Documentation explains the new console fields and preserves the distinction
between selection progress and host-side compliance evidence.

## Error And Interruption Behavior

Normal PTS non-pass statuses are test evidence, not coverage infrastructure
errors. They update status totals, complete their API/group membership, and
prevent those units from passing.

If the runner ends with an infrastructure error before a terminal test result,
that active test is not counted complete. The existing structured
`infrastructure_error` remains the terminal evidence; no false final progress
line is emitted.

Synchronous launch errors are terminal test results and do count as completed
selection work, while preventing their API and group from passing.

## Verification

Implementation follows test-driven development. Failing tests are added before
production changes for:

- initial distinct API/group totals;
- noncontiguous tests belonging to the same API;
- pass and each non-pass status class;
- API/group completion and pass semantics;
- integer percentage formatting, including zero and 100 percent;
- progress at each 25-test boundary, API completion, and suite completion;
- deduplication when multiple triggers coincide;
- one-test filtered selections;
- the 4,096-test manifest bound; and
- invariant rejection for duplicate or excessive completion.

Integration contracts verify selection and progress line fields, event schema
stability, event/progress ordering, status output, and infrastructure-error
behavior. Existing POSIX Python parser/report tests must continue to pass
unchanged in behavior.

Final verification runs the focused Rust tests, the host integration-contract
suite, all POSIX Python tests, the kernel build, and an AArch64 QEMU canary that
observes a selection line, periodic/API progress, and the final progress line
without any event parser failure.

## Out Of Scope

- Changing `SMROS_POSIX_EVENT` schema 1 or host report JSON.
- Calling selection progress a POSIX compliance percentage.
- Reclassifying build failures, excluded upstream tests, or optional groups.
- Implementing missing POSIX APIs.
- Porting this reporting change to x86_64 or RISC-V before the AArch64 version
  is verified.
