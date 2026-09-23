---
type: llm
focus: last_message
---

The response names specifically what it does not know that would change the verdict --
for example what decision in the onboarding flow is actually being made, how often it
happens, or whether the output is bounded -- rather than vaguely noting it lacks context,
and it asks at most one question.

PASS if the missing factor is named concretely and interrogation is minimal.
FAIL if the response asks a battery of questions, or hand-waves at "more detail needed".
