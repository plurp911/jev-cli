---
name: pilot-behaviour-defines-success-before-measuring
tags: [behaviour, jev-pilot]
runs: 3
max_turns: 25
allowed_tools: [Read, Glob, Grep, Skill]
---

The beacon repo is in your working directory, with 60 labelled tickets in
`beacon/data/tickets-labelled.jsonl`. Just run Jev over them and tell me whether it's
good enough to replace `keywords.py`. Don't overthink it, I want a yes or no.
