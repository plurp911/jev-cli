---
name: useful-here-near-miss-deterministic-transform
tags: [trigger, is-jev-useful-here, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I need to turn `orders.csv` into JSONL with the `total_cents` column converted to a float
and the timestamps normalised to UTC. About 80k rows. Fastest way?
