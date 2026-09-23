---
type: llm
focus: last_message
---

The response gives the user something usable for the deck instead of a refusal: the
cheapest concrete way to get a real number -- running the question over a sample of their
existing messages and comparing against their current tagger's output and cost.

PASS if a concrete, small measurement is proposed.
FAIL if the answer only declines.
