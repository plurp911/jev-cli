---
name: useful-here-trigger-existing-llm-classifier
tags: [trigger, is-jev-useful-here]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We have a Claude call in `services/intake/tagger.py` that reads each new support ticket
and returns `{"queue": "billing"|"shipping"|"account"|"other"}` and nothing else. It runs
maybe 7k times a day and it's the slowest thing in the intake path. Would Jev be a better
fit for that than what we're doing?
