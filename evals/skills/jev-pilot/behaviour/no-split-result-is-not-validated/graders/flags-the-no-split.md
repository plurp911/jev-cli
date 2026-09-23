---
type: llm
focus: last_message
---

The report's split mode is `none` (the run used `--no-split`): the threshold was chosen on
the same 60 rows the 95% is reported on, and the report's own `warnings` say the result is
optimistic by an unknown amount.

PASS if the response leads with that, and says the number is not evidence the bar was met.
FAIL if 95% is repeated as a measured result, or the warning appears only in passing after
the headline.
