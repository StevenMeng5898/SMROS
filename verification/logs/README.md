# Campaign logs

`hermes-campaign-serial.log.gz` is the published guest serial for code commit
`751b308560119ceb3de3571cfaf90787cd8de44d`. Its preamble is
`utc=2026-11-23T19:58:53Z`, `git_head=751b308`, `git_dirty=false`, and every
POSIX event `smros_commit` is that full SHA. `campaign-summary.txt` is the
output of `scripts/summarize-hermes-campaign.py` on that gzip.
`serial_sha256` there is the sha256 of the gzip bytes.

The gzip does not contain `prompt_seen` or `campaign_finished`. Those strings
are printed by `scripts/run-hermes-campaign.sh` on its own stdout. They were
not saved in this tree, so they are not part of the published record.

## Discarded first boot

One earlier boot of the same code commit is not in this tree and is not
evidence. Its kernel image had been built while `host_shared/posixtest` still
held the previous stage, so the embedded manifest still said
`smros_commit=500c9486773717d901acb11636874d6d9a3512f2`. The preamble of that
serial said `git_head=751b308`, and every POSIX event carried the old full
SHA. That file was not committed. It does not support a posixtest, sectest,
or overall result.

The published gzip is the later boot, after the kernel was rebuilt from the
stage whose manifest `smros_commit` is
`751b308560119ceb3de3571cfaf90787cd8de44d`.

## Baseline status notes

`posix-baseline-cases.txt` lists seven qemu-user status changes against the
`500c948` archive. Their classification, and the three-run repeats of the
tests that left pass, are in `docs/NOTIFY_GATE_SLEEP.md` and
`posix-baseline-status-rerun/`.
