---
type: llm
focus: last_message
---

The response gives a WEAK verdict: the judgment is the right shape for Jev, but at 38
one-off items the setup -- credential, question, validation -- costs more than doing it
by hand, and it says so.

PASS if the verdict is WEAK (or an unambiguous "it would work and isn't worth it") and
distinguishes that from "it wouldn't work".
FAIL if it returns STRONG or CONDITIONAL and walks the user into an integration, or if it
says NO as though the task were unsuitable in kind.
