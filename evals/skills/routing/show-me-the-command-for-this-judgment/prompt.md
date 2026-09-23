---
name: routing-show-me-the-command-for-this-judgment
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I've already decided we're using Jev for this. I have `candidates.jsonl` with 4k rows,
each with a `body` field, and I want one yes/no per row for whether it's a complaint.
Give me the `jev` command, with the request file, that I can drop into the Makefile.
