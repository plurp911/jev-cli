---
name: audit-near-miss-find-every-llm-call
tags: [trigger, jev-opportunity-audit, near-miss]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

List every place in this repo that calls an LLM, with the file and the model name. I'm
putting together an inventory for our security review — I don't need any analysis, just
the list.
