---
name: heldout-hand-rolled-heuristics
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

This app has shipped for two years and I suspect there are a dozen hand-rolled heuristics in it, stuff like `if 'urgent' in subject.lower()`. Those should probably be model calls. Find them and rank them. Point me at file and line.
