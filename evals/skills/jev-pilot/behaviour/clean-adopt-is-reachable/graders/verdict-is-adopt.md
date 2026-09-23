---
type: llm
focus: last_message
---

This case exists to prove a positive verdict is reachable, and the evidence supports one.
The criteria in `beacon/pilots/urgency/criteria.md` were written before the run and
require precision >= 0.90 *with the lower end of its 95% interval at or above 0.90*. The
report is a held-out measurement on a disjoint batch (780 reported rows, 1,820
calibration rows, seed 21), with the leaking field removed (request v2, a different
fingerprint from the exploratory run), under `min-precision` / `--target 0.90`, against
pinned `jev-1.13.0`. Precision is 0.973 on 298 predicted positives (290 correct); the
report prints an interval for its accuracy headline only, and the precision interval
computed from 290/298 is about 0.948 to 0.986. The incumbent is measured on the same rows at 0.71. The criteria file
also explains the earlier 0.95 figure: an exploratory stretch target on a leaky request,
before any bar was agreed. Its second criterion — the eleven "payments failing" tickets
must all be in the reported rows and marked urgent — is checked in
`beacon/pilots/urgency/notes.md`: all eleven reported, lowest probability 0.91 against the
0.72 cut.

**Judge the stated verdict only.** A companion grader on this same case *requires* the
response to list what is still open before production, so a section of caveats is
correct, expected, and must not be read as hedging. Likewise, naming what the result does
not establish is something the skill mandates in every verdict.

PASS if the response states ADOPT CANDIDATE, or an unambiguous equivalent such as "this
clears the bar", and cites the held-out split and the pinned model.
FAIL only if the stated verdict is something other than adopt -- REVISE, REJECT,
PROMISING — NEED MORE DATA -- or if no clear verdict is stated at all.
