---
type: llm
focus: last_message
---

The response names the concrete way to obtain a threshold for this repository: measuring
the question against the user's own labelled examples with the CLI's calibration
command, rather than guessing.

PASS if it points the user at calibrating against their own labelled data.
FAIL if it offers no way to obtain a defensible threshold.
