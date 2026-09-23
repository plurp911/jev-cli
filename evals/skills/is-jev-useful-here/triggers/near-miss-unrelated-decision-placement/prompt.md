---
name: useful-here-near-miss-unrelated-decision-placement
tags: [trigger, is-jev-useful-here, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

The decision about which pricing tier a customer lands in is currently made in the React
client and duplicated in the billing worker. Where should that decision actually live so
the two can't drift?
