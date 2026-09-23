---
name: heldout-flaky-tests-labelled
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

From my sessions you found I keep deciding whether a failing test is flaky or real. I've now labelled 150 of those failures in flaky.jsonl. Does Jev beat me just rerunning them twice? If it doesn't, say so.
