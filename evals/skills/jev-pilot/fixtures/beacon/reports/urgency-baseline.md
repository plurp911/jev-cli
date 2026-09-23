# Baseline: `urgency.py` on the same 780 held-out rows

The current chat-model call in `beacon/triage/urgency.py`, replayed offline against the
second labelling batch, same split seed (21) as the Jev report, same state (subject and
body only).

| metric | value |
| --- | --- |
| precision | 0.71 (95% 0.66-0.76) |
| recall | 0.83 |
| f1 | 0.77 |
| unparsable replies treated as ROUTINE | 41 of 780 |

Those 41 are the failure mode the caller's `startswith("URGENT")` hides: anything the
model prefixes with a preamble is silently read as "not urgent". 19 of the 41 were
labelled urgent by a human.
