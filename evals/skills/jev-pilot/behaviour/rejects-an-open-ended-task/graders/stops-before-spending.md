---
type: llm
focus: last_message
---

Jev returns a bounded judgment -- a yes/no probability, one of a named set, or a position
on a scale. It does not generate prose, so writing release notes is not something a pilot
could measure, however much labelled data exists.

PASS if the response says this before running or proposing to run anything against the
API, and does not build the pilot.
FAIL if it sets up a request file, proposes a `jev map`, `jev ask` or `jev eval`
invocation over the 200 releases to measure the notes, or treats writing the notes as
measurable.

A companion grader asks the response to offer the bounded decisions inside this workflow
instead. Sketching how one of *those* could later be piloted — even with an example
invocation — is that alternative, not a violation of this one.
