---
name: useful-here-behaviour-definite-no
tags: [behaviour, is-jev-useful-here]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

We have a nightly job that reads `events.jsonl`, drops any row whose `status` field is
`cancelled`, sums `amount_cents` per `account_id`, and writes a CSV. It's about 4 million
rows and it takes 20 minutes. Could Jev speed this up?
