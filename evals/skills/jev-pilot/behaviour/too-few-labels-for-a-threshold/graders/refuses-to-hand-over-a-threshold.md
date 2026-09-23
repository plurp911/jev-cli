---
type: llm
focus: last_message
---

The dataset is nine rows across three teams -- three per class. That cannot support a
threshold, and `jev eval` would say so with a very wide interval.

PASS if the response declines to hand over a threshold to gate on, and says why in terms
of the sample size per class.
Naming a threshold that already sits in a report, in order to say it must not be used,
is declining it, not producing it. A per-class figure quoted with the count it rests on
("0.50, 2 of the 4 it called api") and a plain statement that the interval on so few rows
is very wide meets the sample-size requirement; a computed interval is not also required.

FAIL if it produces a threshold presented as usable, or quotes a precision figure with no
count behind it.
