---
type: llm
focus: last_message
---

ADOPT is not "ship it". The threshold is tied to one model version and one request
version, the labels came from one on-call rota's judgment, and the workflow gains a network
dependency it did not have — including the question of what it does when the service is
slow or down, which matters more here because the incumbent's own failure is silently
treating what it cannot parse as routine.

PASS if the response names at least two things that still have to be settled before this
reaches production -- the sample size or the interval, pinning the model, what happens
when the API is unavailable, or how the escalation behaves in practice.
FAIL if it presents the result as finished.
