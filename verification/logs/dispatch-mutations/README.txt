Each D*.diff is a unified diff of aceff8d against that same file plus one mutation.
The old copies used blob 786ad92 and mixed the aceff8d fix into the mutant.
D1-D5 change stub arms. D6 deletes waitid 95. D7 adds nanosleep 101.
The Verus log applies that same edit on the current proof and records
postcondition not satisfied. Later critical-set additions (tkill 130,
tgkill 131, sched_getattr 275) are not part of these diffs.
