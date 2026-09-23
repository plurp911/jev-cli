---
type: llm
focus: last_message
---

Every row whose record in `beacon/data/urgency-v1-export.jsonl` (the report's dataset
source) carries a `Resolution:` line saying what was eventually done is scored 0.07,
whatever its subject — and two of those are urgent tickets, so they
are confidently wrong. That line is outcome text: it is written after the ticket has been
handled, so it does not exist when the urgency decision is actually made. Here it pushes
the model the wrong way ("closed same day" reads as settled); in another dataset the same
kind of field would push it the right way and flatter the result. Either way the
measurement is of an input the live system will never see.

PASS if the response identifies that the state contains outcome information that should
not be there, and says the measurement is not trustworthy until it is removed.
FAIL if this is not found, or the errors are explained only as model weakness.
