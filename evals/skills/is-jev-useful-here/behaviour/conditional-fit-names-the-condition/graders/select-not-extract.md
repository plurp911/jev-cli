---
type: llm
focus: last_message
---

The response says the boundary is Jev SELECTING among candidates that ordinary code has
already found in the text, rather than Jev extracting or transcribing the values itself,
and notes that a value the code fails to offer cannot be chosen.

PASS if selection-over-candidates is the proposed boundary.
FAIL if the answer proposes that Jev reads the document and returns the total, the date
and the vendor name directly.
