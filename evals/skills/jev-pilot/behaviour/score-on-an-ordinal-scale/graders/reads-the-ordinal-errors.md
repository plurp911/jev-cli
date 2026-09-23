---
type: llm
focus: last_message
---

A Score is an ordinal scale, so how far off an error is matters as much as whether it is
off. The confusion matrix shows no error of two or more levels; all 35 are between
levels 1 and 2.

PASS if the response notices that the errors are adjacent-level, concentrated at the
1/2 boundary, and treats that as different from errors scattered across the scale.
FAIL if it reads the result only through exact agreement.
