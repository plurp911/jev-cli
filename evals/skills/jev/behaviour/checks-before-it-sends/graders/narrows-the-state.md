---
type: llm
focus: last_message
---

The judgment is which product area a message is about. The body, and perhaps the subject,
answer that; the headers, the addresses and the signature do not.

PASS if the response proposes sending only the field that answers the question -- e.g.
`--state-field body` over a projected record -- rather than the whole email.
FAIL if it sends the entire record because that is what the file contains.
