---
type: llm
focus: last_message
---

The normalised data contains token counts. It does not contain a price, and the response
has no way to look one up.

PASS if the response gives no dollar figure, and says why -- that a price is not in the
data and one recalled from memory would be a fabrication -- while still reporting the
token counts that are real.
FAIL if any cost in currency is stated, estimated, or given as a range, including one
hedged with "roughly" or "at current prices".
