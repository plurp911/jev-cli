---
name: useful-here-near-miss-debug-the-cli
tags: [trigger, is-jev-useful-here, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

`jev doctor` keeps exiting 3 on our CI runner even though `JEV_API_KEY` is set in the job
environment. Works fine on my laptop. What's going on?
