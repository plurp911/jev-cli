---
name: useful-here-trigger-batch-data-triage
tags: [trigger, is-jev-useful-here]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We've got 2.4 million free-text adverse-event reports in a Postgres table and the safety
team wants the ones that mention a hospitalisation pulled out for review. Keyword lists
have been tried and they're bad. Is this a Jev-shaped problem?
