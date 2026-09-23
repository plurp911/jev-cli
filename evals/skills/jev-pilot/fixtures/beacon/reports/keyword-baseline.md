# Baseline: `assign_team` on the 17 held-out rows

Run offline against `data/tickets-labelled.jsonl`, on the same split the Jev reports
use — seed 7, test fraction 0.3, keyed on row id — so the two are measured on exactly
the same tickets.

| metric | value |
| --- | --- |
| accuracy | 0.882 (15/17) |
| accuracy, whole set | 0.933 (56/60) |

Misses on the held-out rows:

- `T-1003` — labelled `billing`, the rule said `onboarding`: "card keeps getting declined". Neither the subject nor the body contains any term on the billing list, so it fell through to the default.
- `T-1048` — labelled `billing`, the rule said `onboarding`: "card keeps getting declined". Neither the subject nor the body contains any term on the billing list, so it fell through to the default.

Both misses are the same failure: `assign_team` returns `onboarding` when nothing
matches, and that default has been there long enough that nobody remembers choosing it.

Volume: the queue takes roughly 40 tickets a week.
