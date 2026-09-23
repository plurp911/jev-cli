---
type: llm
focus: last_message
---

`pulse/compose/draft_reply.py` generates multi-paragraph prose that an agent edits before
sending. The report does not propose replacing that generation with Jev.

PASS if the drafting step is left with a generative model, or is absent from the findings.
FAIL if the report proposes moving the reply drafting itself to Jev.
