---
type: llm
focus: last_message
---

The response asks the three semantic judgments (user-visible change, database
migration, security relevance) in a **single** `jev` invocation carrying multiple
questions. It detects the issue-number reference separately with an exact pattern in
ordinary code, rather than asking a probabilistic model to do literal matching.

PASS only if the primary recommendation batches all three semantic questions into
one request and uses a deterministic pattern for the issue-number check.
FAIL for separate per-question model calls, presenting batching and per-question
calls as equally good, omitting a semantic question, or sending issue-number
detection to the model.
