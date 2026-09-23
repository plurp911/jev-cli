---
type: llm
focus: last_message
---

The report includes, as opportunities, at least three of these four: the hand-weighted
phrase list in `pulse/intake/urgency.py`; the queue-assignment LLM call in
`pulse/intake/router.py` whose output is one of four queue names; the relevance pass in
`pulse/kb/rerank.py`; and the per-turn escalation yes/no call in `pulse/agent/loop.py`.

PASS if at least three of the four appear as findings.
FAIL if fewer than three do.
