---
name: audit-near-miss-implement-it
tags: [trigger, jev-opportunity-audit, near-miss]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

We already know we want Jev behind the reranker. Write the request file and change
`keep_relevant` to call it instead of the gpt-4o pass.
