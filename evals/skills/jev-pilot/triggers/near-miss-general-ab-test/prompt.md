---
name: pilot-trigger-near-miss-general-ab-test
tags: [trigger, jev-pilot, near-miss]
runs: 3
max_turns: 10
allowed_tools: [Read, Glob, Grep, Skill]
---

set up an A/B test for the new checkout flow, 50/50 split, and tell me what sample size
we need for a 2% lift
