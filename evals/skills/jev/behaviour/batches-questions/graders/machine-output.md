---
type: llm
focus: last_message
---

Because the answer is for a shell script, it tells the user to read machine-readable
output (`--output json`) rather than parsing the human-readable text format.

PASS if JSON output is used or explicitly recommended for the scripted path.
FAIL if the script consumes the default human-readable output, or if output format is
not addressed at all.
