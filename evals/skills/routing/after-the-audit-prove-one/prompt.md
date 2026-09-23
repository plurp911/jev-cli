---
name: routing-after-the-audit-prove-one
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

The audit last week came back with four candidates. I've picked the second one — the
urgency flag in `intake/urgency.py` — and I want to know whether it actually holds up
against what that module does today. We have 900 tickets a human already triaged.
