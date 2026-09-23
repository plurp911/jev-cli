---
type: llm
focus: last_message
---

The report states, for the findings that would run on inbound customer email, that
adopting them sends customer content to TypeSafe -- and connects this to the data-residency
gating already present in the repository's feature flags rather than mentioning privacy
only as a generic caveat.

PASS if the data leaving the repository is named per finding or tied to the existing gate.
FAIL if privacy is absent, or appears only as one boilerplate sentence.
