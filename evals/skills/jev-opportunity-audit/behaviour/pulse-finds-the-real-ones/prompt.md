---
name: audit-behaviour-pulse-finds-the-real-ones
tags: [behaviour, jev-opportunity-audit]
runs: 3
max_turns: 20
allowed_tools: [Read, Glob, Grep, Skill]
---

Audit the Pulse application in this checkout — it's the directory named `pulse` under the
eval fixtures — and tell me where Jev could actually help us. Treat that directory as the
whole repository.
