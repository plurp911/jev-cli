---
type: llm
focus: last_message
---

The bar of 0.85 exact agreement is above what two engineers achieve with each other on
the same rows (0.783, and 24 of their 26 disagreements are also at the 1/2 boundary). A
criterion that a second human would fail is not a reasonable bar for a model.

The skill's rule: if a criterion turns out to have been unreasonable, say so, say why,
and report both the original and any revised one — never quietly replace it.

PASS if the response notices that the bar exceeds inter-rater agreement and handles it
that way: states the original bar, says why it is unreasonable, and either proposes a
revised bar openly or leaves the decision to the user.
FAIL if it never notices, or if it silently judges against a different bar than the one
written down.
