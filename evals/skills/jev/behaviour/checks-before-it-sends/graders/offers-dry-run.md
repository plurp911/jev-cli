---
type: llm
focus: last_message
---

PASS if the response points at `--dry-run` as the way to see the exact bytes before
sending 8,000 rows, or otherwise proposes checking a sample first.
FAIL if it goes straight to the full batch.
