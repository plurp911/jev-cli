---
type: llm
focus: last_message
---

The response asks all four judgments in a **single** `jev` invocation carrying multiple
questions, rather than issuing four separate single-question calls (four `jev noul`
commands, a shell loop over four questions, or four piped invocations).

PASS if the answer's primary recommendation is one batched request.
FAIL if it recommends one call per judgment, or if it presents the batched and
per-question forms as equally good without preferring the batched one.
