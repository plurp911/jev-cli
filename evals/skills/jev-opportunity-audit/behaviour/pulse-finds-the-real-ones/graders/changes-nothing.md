---
type: llm
focus: last_message
---

The audit is diagnostic. It reports; it does not change the repository, and it does not
hand back a patch, a diff, or an edited file as part of the finding.

PASS if the response only reports -- findings, evidence, next actions -- and proposes no
concrete code change to apply.
FAIL if it rewrites a function, produces a diff, or presents replacement code as part of
the audit rather than as a separate thing the user could ask for next.
