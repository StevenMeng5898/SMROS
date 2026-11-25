# Local run logs

Everything in this directory except this file is gitignored. `make ut`,
`make it`, `make skt`, `make futex-stress`, `make cbmc-futex`, `make verus`,
and `hermes test-all` may create or rewrite files here. Those files stay on
disk for the person who ran the command and are never committed, so a later
`git pull` cannot fail with "local changes would be overwritten".

A run writes:

- `futex-stress.log` and `cbmc-futex.log` from the Makefile targets
- `verus-*.log`, `make-verus-summary.txt`, and `verus-functions.txt` from
  `make verus`
- `verus-mutation-summary.txt`, the mutation-gate cache, from
  `scripts/audit-verus-mutation.py`
- `campaign-summary.txt` when `scripts/summarize-hermes-campaign.py` is given
  a serial log

No script writes `verification/evidence/`. That tree is the committed record
of specific past runs. See `verification/evidence/README.md`.
