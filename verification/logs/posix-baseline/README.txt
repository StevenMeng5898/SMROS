posix-baseline is the Linux qemu-user reference, not the SMROS guest.
run-1364.ndjson is the host reference from the campaign whose summary
was posix-baseline=1364/236/0. The earlier 1395/205 and 1397/203 totals
were not saved per test; that ndjson was overwritten before it was
committed. run-1364.tsv is the same rows without logs.
A later official run is compared in DIFF.txt when that file exists.
