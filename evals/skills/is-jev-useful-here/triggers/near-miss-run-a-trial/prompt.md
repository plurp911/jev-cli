---
name: useful-here-near-miss-run-a-trial
tags: [trigger, is-jev-useful-here, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We've agreed Jev is worth a look for ticket triage. Take 50 of our resolved tickets from
`tickets-resolved.jsonl` and actually run it over them so we can see whether it beats the
classifier we have now.
