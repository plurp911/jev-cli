---
name: jev-behaviour-checks-before-it-sends
tags: [behaviour, jev]
runs: 3
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

I've got last quarter's support inbox exported to `inbox-export.jsonl` — 8,000 rows, each
one the full email with headers. I want jev to tag each one with a product area. Give me
the command to run over the whole file.
