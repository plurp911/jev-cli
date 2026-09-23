---
name: audit-behaviour-pixelsort-reports-none
tags: [behaviour, jev-opportunity-audit]
runs: 3
max_turns: 20
allowed_tools: [Read, Glob, Grep, Skill]
---

Audit the pixelsort tool in this checkout — it's the directory named `pixelsort` under the
eval fixtures — and tell me where Jev could actually help us. Treat that directory as the
whole repository.
