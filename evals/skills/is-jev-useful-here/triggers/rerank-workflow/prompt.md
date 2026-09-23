---
name: useful-here-trigger-rerank-workflow
tags: [trigger, is-jev-useful-here]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I have 50 retrieved documents per query and I ask Claude which ones are relevant before I
put them in the context for the answer call. Could Jev help with that step?
