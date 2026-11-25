# Published run records

These files are the committed record of specific past runs. No build, test,
or campaign script writes this tree. A new record is copied here by hand,
under the full commit it measured, and then left alone.

`verification/logs/` is the ignored directory a run rewrites. Do not put a
new log there and commit it.

## `751b308560119ceb3de3571cfaf90787cd8de44d`

The hermes campaign that booted that commit (`utc=2026-11-23T19:58:53Z`,
`git_dirty=false`):

- `hermes-campaign-serial.log.gz` is the guest serial as the launcher wrote
  it. It has no `prompt_seen` or `campaign_finished` line; those are printed
  by `scripts/run-hermes-campaign.sh` on its own stdout and were not saved.
- `campaign-summary.txt` is `scripts/summarize-hermes-campaign.py` run on
  that gzip. `serial_sha256` is the sha256 of the gzip bytes.
- `posix-baseline-results.ndjson.gz` and `posix-baseline-cases.txt` are the
  qemu-user reference. The summarizer compares a campaign's
  `results_sha256` with the ndjson and with the hash recorded in the cases
  file. `posix-baseline-cases.tsv` is the per-test status list.
  `posix-baseline-open-files.txt` is the 40 mqueue attempts whose output
  contained "Too many open files".
- `posix-baseline-status-rerun/` holds the three-run repeats, the 20
  `VERBOSE=2` runs, and the `LD_DEBUG=bindings` log for
  `pthread_cond_init/1-3`.
- `make-verus-summary.txt`, `verus-functions.txt`, and
  `verus-mutation-summary.txt` are the Verus totals, the function table, and
  the mutation-gate result for that commit. `make verus` writes fresh copies
  under `verification/logs/` and does not update these.

One earlier boot of the same code is not in this tree. Its kernel image had
been built while `host_shared/posixtest` still held the previous stage, so
the embedded manifest still said
`smros_commit=500c9486773717d901acb11636874d6d9a3512f2`. That serial is not
evidence.

## `ed2f3464e116b72bd12e59f8d3c7904eaec65a6f`

`notify-gate-sleep.txt` is the 50-run in-guest sample
(`utc_start=2026-11-23T09:00:55Z`). The percentiles in
`docs/NOTIFY_GATE_SLEEP.md` come from this file.

## `3bd7e4162ab2481a3674f7844d40364516163e24`

`notify-gate-sleep-smp.txt` and `notify-gate-sleep-serial.log.gz` are the
`-smp 4` measurement. The kernel still reported one hardware-online CPU.

## `0d201b3dedfd47f48946b3ba2363b9731474f452`

`posix-baseline-mqueue-isolated.txt` and `posix-baseline-mqueue-isolated/`
are three qemu-user runs of the message-queue tests in a private mqueue
namespace. The note's `git_describe` is this commit. The ELF-only follow-up
`479b0b5` did not change that path.

## `e5cbb54c845e99ccd00cb1942303f555b360e88e`

`ring-index-mutants/` is eight hand-recorded body edits of
`fifo_ring_index` and `socket_ring_index`. Each Verus run died on the
postcondition. The mutation gate does not regenerate these diffs.
