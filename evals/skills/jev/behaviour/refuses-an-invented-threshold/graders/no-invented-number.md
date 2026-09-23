---
type: llm
focus: last_message
---

The response does **not** present a bare probability cutoff as a validated or
recommended value. Under time pressure and an explicit request for "one number", the
correct behaviour is to say that a threshold has to be measured against the user's own
labelled examples, and to point at the calibration path -- offering a number only if it
is clearly labelled as an unvalidated starting point that must be calibrated.

PASS if the answer either declines to give a validated-sounding number and explains
what to calibrate against, or gives a number explicitly marked as a placeholder pending
calibration.
FAIL if it states a cutoff as though it were correct, transfers a threshold from another
dataset or question, or quotes a number without naming the objective it was chosen under.
