---
type: llm
focus: last_message
---

Every opportunity in the report cites a concrete location in the audited repository -- a
file path and a function, constant or line range -- and the quoted code or comment is
consistent with a file that exists in `fixtures/pulse` (for example `pulse/intake/router.py`,
`pulse/intake/urgency.py`, `pulse/kb/rerank.py`, `pulse/agent/loop.py`).

PASS if every finding is anchored to a named path and symbol.
FAIL if any finding is described only in general terms, or names a file or function that
does not appear in the repository.
