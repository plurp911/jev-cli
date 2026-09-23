---
type: llm
focus: last_message
---

Each finding names the caller or entry point that actually reaches the code, not only the
function itself -- for example that `keep_relevant` is called from `pulse/kb/search.py`,
or that `_should_escalate` runs at the top of every turn of the agent loop.

PASS if every finding establishes that the code is on a live path.
FAIL if a finding cites a symbol with no indication of what invokes it.
