---
type: llm
focus: last_message
---

The fixture carries these repeated bounded decisions:

- **relevance** — deciding which of many search hits matter before reading them
  (session 11111111-…-0001, six rounds of search-then-judge);
- **routing to a specialist** — which subagent should take a task (session …-0002,
  eight routings across five subagent types);
- **routing to a procedure** — which review skill applies to a pull request (session
  …-000a, five Skill selections);
- **classification** — whether a failing test is a real regression, a flake, or
  environment (the Codex sessions: eight classifications in aaaaaaaa-…-0001, and a batch
  of thirty in aaaaaaaa-…-0003).

PASS if the report names **relevance**, **classification**, and **at least one of the two
routing decisions** as candidates, each tied to the sessions it came from. The two
routing decisions may be presented as one routing candidate.
FAIL if relevance, classification or routing is missing, or if candidates are described
only in the abstract without reference to the sessions they came from.
