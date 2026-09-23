# Severity pilot — notes from the error pass

`reports/severity-score-heldout.json` is the scored run; its `--show-rows` rows carry
each answer but not its confidence. To read confidence, the 120 reported rows were sent
once more with `jev map` (same request, same pinned model). **This is a second draw**: a
row wrong in the scored run could have come back differently here, so these figures
describe the pattern, not the scored answers.

Mean confidence by the on-call engineer's level:

| level | mean confidence |
| --- | --- |
| 0 | 0.90 |
| 1 | 0.55 |
| 2 | 0.57 |
| 3 | 0.90 |

Sampled rows from the map draw:

| id | label | answer | confidence |
| --- | --- | --- | --- |
| S-2001 | 2 | 1 | 0.47 |
| S-2002 | 1 | 2 | 0.45 |
| S-2003 | 3 | 3 | 0.91 |
| S-2004 | 0 | 0 | 0.93 |
| S-2005 | 1 | 1 | 0.58 |
| S-2006 | 2 | 2 | 0.64 |
| S-2007 | 1 | 2 | 0.44 |
| S-2008 | 2 | 1 | 0.49 |
