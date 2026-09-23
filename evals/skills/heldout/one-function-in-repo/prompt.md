---
name: heldout-one-function-in-repo
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

In this repo, triage_label() in src/inbox/classify.py calls GPT-4 to pick one of 5 labels. Does it make sense to swap just that function over to Jev?
