---
name: jev-behaviour-batches-questions
tags: [behaviour, jev]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

For each incoming pull request description I need four judgments before I let it into
the review queue: does it describe a user-visible change, does it mention a database
migration, does it look security-relevant, and does it reference an issue number. I want
to do this with `jev` from a shell script. Show me what to run.
