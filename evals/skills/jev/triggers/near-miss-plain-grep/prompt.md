---
name: jev-trigger-near-miss-plain-grep
tags: [trigger, jev, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I need to classify every row of `deploy.log` by severity so I can count them. The lines
all start with a literal level token — `ERROR`, `WARN`, `INFO` — in column one, and
there are no exceptions; I checked. `jev` is installed on this box. Give me the command.
