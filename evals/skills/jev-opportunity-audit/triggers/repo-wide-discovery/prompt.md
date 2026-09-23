---
name: audit-trigger-repo-wide-discovery
tags: [trigger, jev-opportunity-audit]
runs: 3
max_turns: 10
allowed_tools: [Read, Glob, Grep, Skill]
---

Go through this codebase and tell me everywhere Jev could actually help us. I don't have
a specific thing in mind — I want to know what's in here.
