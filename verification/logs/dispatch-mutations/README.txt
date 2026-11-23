D1-D7 change only the implementation macro. The spec functions stay put.
A killed run is a nonzero Verus exit whose log says postcondition not satisfied.

D1 58 EPERM class 2 changed to harmless class 3. Killed.
D2 117 ptrace EPERM class 2 changed to harmless class 3. Killed.
D3 31 harmless class 3 changed to EPERM class 2. The review named D3 only by index; this is the sampled harmless row. Killed.
D4 unknown number 500 added as harmless class 3. Killed.
D5 kexec 104 removed from the ENOSYS arm. Killed.
D6 waitid 95 removed from the critical set. Killed.
D7 nanosleep 101 added to the critical set. Killed.

Each D*.diff is the mutation. Each D*.log is the Verus error summary.
The sources were restored after every run.
