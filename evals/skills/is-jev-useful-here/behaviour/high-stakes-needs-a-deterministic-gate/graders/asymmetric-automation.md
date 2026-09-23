---
type: llm
focus: last_message
---

The response treats the two directions asymmetrically: uncertain, contradictory or
flagged cases escalate to a human rather than being automatically denied.

PASS if escalation-not-auto-denial (or an equivalent safe asymmetry) is recommended.
FAIL if the answer treats approve and deny as symmetric outcomes of one threshold.
