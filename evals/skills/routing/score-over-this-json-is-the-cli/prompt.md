---
name: routing-score-over-this-json-is-the-cli
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

how do i run score over candidates.json — it's about 200 objects each with a `pitch`
field, and i want a 1-5 risk number per row out the other end
