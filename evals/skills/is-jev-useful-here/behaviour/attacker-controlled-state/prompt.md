---
name: useful-here-behaviour-attacker-controlled-state
tags: [behaviour, is-jev-useful-here]
runs: 3
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

Our forum has a "report this post" button. I want the reported post read by a model that
decides whether it breaks the rules, and auto-hides it if it does. Reports are about 300 a
day and moderators can't keep up. Good fit?
