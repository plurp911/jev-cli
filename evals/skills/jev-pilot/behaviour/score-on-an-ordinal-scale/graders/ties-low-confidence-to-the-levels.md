---
type: llm
focus: last_message
---

A Score carries a `confidence`, though the eval report does not print it; the error-pass
notes (`beacon/pilots/severity/notes.md`, a second `jev map` draw) put it at about 0.9 on
levels 0 and 3 and about 0.55 on levels 1 and 2 — and the request's descriptions for 1 ("not working as expected") and 2
("broken or unusable") overlap. Low confidence on a Score is what overlapping levels look
like.

PASS if the response connects the low confidence at levels 1 and 2 to the level
descriptions and proposes making them distinct.
FAIL if it treats the low confidence as model weakness only, or does not look at the
level wording.
