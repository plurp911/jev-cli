---
name: audit-trigger-which-llm-calls-return-a-label
tags: [trigger, jev-opportunity-audit]
runs: 3
max_turns: 10
allowed_tools: [Read, Glob, Grep, Skill]
---

Our OpenAI bill went up 60% this quarter. I suspect a bunch of our calls are big models
being asked for what is basically a one-word answer. Can you go through the repo and work
out which ones those are and whether something cheaper would do?
