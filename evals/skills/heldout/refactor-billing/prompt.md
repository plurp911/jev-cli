---
name: heldout-refactor-billing
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

Refactor src/billing/ so the invoice generator isn't one 900-line function. Keep the public API the same.
