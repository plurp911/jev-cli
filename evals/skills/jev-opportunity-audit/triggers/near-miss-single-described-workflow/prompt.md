---
name: audit-near-miss-single-described-workflow
tags: [trigger, jev-opportunity-audit, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We ask Claude which of our 50 retrieved chunks are relevant before we generate the answer.
Would Jev help with that step?
