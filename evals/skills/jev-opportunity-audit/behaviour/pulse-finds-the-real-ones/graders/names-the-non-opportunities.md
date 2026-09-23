---
type: llm
focus: last_message
---

The report contains an explicit section identifying tempting places Jev should NOT be
introduced, and names at least three of: `pulse/auth/permissions.py` (authorization),
`pulse/billing/money.py` (exact arithmetic), `pulse/config/flags.py` (a lookup table, not
routing), `pulse/ingest/email_headers.py` (deterministic parsing), and
`web/src/ticketList.ts`.

PASS if such a section exists and names at least three, with a reason each.
FAIL if there is no such section, or it is a single vague sentence.
