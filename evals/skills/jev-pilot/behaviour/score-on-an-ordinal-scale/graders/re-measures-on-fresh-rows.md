---
type: llm
focus: last_message
---

Rewording the levels and re-running on the same seed-11 held-out rows would reuse the
rows the first result was read from.

PASS if the response says a revised question has to be measured on rows that no previous
measurement was read off — a fresh labelled batch, or a new split with the old reported
rows excluded.
FAIL if it proposes rewording and re-running on the same split, or does not say where the
re-measurement comes from.
