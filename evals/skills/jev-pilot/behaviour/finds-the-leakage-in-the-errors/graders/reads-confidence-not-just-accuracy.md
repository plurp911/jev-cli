---
type: llm
focus: last_message
---

This is a Noul, so there is no `confidence` field — how sure the model was is read from
the probability itself: near 0 or 1 is sure, near the cut is not.

The report's rows carry each ticket's probability as `predicted`. They contain two
different kinds of error. Two are **confidently wrong**: urgent tickets the model put at
0.07, and they carry the leaked `Resolution:` line. One is **uncertain and wrong**: the
urgent T-1044 at 0.46, in the middle of the range, with no leak.

PASS if the response distinguishes the sure-and-wrong rows from the near-the-cut one, and
says why that matters: a confident error points at the question or the state, while a
near-the-cut error is what an escalation band exists to catch.
FAIL if every error is treated as one undifferentiated group, or if the response looks for
a `confidence` field on the Noul as though one existed.
