Each D*.diff is only the mutation against the proved dispatcher table.
The previous copies used blob 786ad92 and mixed in the aceff8d fix.
D1-D5 edit stub arms that aceff8d already contained. D6 and D7 edit the
critical set after tkill 130, tgkill 131, and sched_getattr 275 were added.
A killed log says postcondition not satisfied.
