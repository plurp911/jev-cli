---
name: heldout-llm-calls-fixed-list
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

we've got llm calls scattered all over services/. find every spot where one is really just picking from a fixed list and tell me which ones are worth moving to jev first
