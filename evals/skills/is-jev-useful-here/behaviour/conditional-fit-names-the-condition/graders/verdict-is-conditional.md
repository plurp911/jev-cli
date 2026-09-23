---
type: llm
focus: last_message
---

The response gives a CONDITIONAL (or equivalently hedged and explicitly named) verdict
rather than an unqualified yes, and names at least one concrete condition that has to
hold first.

PASS if the verdict is conditional and the condition is concrete -- for example that the
text must actually be extractable from the PDF, or that code must generate the candidate
values for Jev to select among.
FAIL if the answer is an unqualified "yes this fits" with the conditions buried or absent.
