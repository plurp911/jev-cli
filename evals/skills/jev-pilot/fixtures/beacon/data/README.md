# data

`tickets-labelled.jsonl` — 60 tickets exported from the queue, with the team a human
actually routed them to and whether it was handled same-day. Subjects and bodies were
rewritten by hand to remove customer detail before export.

`tickets-tiny.jsonl` — the first nine of the same, used when someone wants to eyeball
the format.

`tickets-with-customer-detail.jsonl` — a raw export. **Not scrubbed.** Carries sender
addresses, account identifiers, contract value and internal CRM links. It is here
because someone needed it for a migration and never deleted it.

`urgency-v1-export.jsonl` — the same 60 tickets as exported for the first urgency
attempt, before anyone noticed that the export included the queue's `Resolution` column
on tickets that had already been handled. It is kept because it is what that run
actually measured.
