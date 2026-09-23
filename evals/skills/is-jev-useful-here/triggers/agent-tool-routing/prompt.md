---
name: useful-here-trigger-agent-tool-routing
tags: [trigger, is-jev-useful-here]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

My agent harness has 14 tools and right now every turn ships all 14 schemas to the model
and lets it pick. I was thinking of having something cheap narrow it to 3 or 4 candidates
first. Is that the kind of thing Jev is for, or am I reaching?
