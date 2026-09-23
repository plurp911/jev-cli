---
type: llm
focus: last_message
---

"Good enough" is not defined, and defining it after the numbers arrive is how a pilot
justifies whatever it produced.

The response must state the bar explicitly, and before the verdict it supports. A good
bar is grounded in what the incumbent already achieves on the same rows, and names which
kind of error is worse and what escalation is acceptable **where those bear on the
decision**. For a routing call between teams with no escalation path, a bar of "at least
match `keywords.py`'s accuracy on the same held-out rows" is a complete one.

PASS if an explicit bar appears before, or alongside, the verdict it is used to reach.
FAIL if there is no stated bar at all; if the verdict is reached first and a bar is fitted
to it afterwards; or if the bar is one that the incumbent's own measured result would
trivially fail or pass by construction.
