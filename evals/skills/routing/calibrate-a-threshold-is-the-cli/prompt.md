---
name: routing-calibrate-a-threshold-is-the-cli
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I've got questions.json and 300 labelled rows in labelled.jsonl. What's the exact
`jev eval` invocation to get a threshold at 95% precision, and how do I read the report?
