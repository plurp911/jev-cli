---
name: routing-prove-it-not-weigh-it
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We already decided the ticket classifier is the right place to try Jev — I don't need
convincing. What I need is to find out whether it actually holds up against the
classifier we run today. We have about 900 tickets with the category a human set.
