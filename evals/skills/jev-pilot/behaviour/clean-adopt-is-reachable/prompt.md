---
name: pilot-behaviour-clean-adopt-is-reachable
tags: [behaviour, jev-pilot]
runs: 3
max_turns: 25
allowed_tools: [Read, Glob, Grep, Skill]
---

The beacon repo is in your working directory. We piloted the urgency call — the one
`beacon/triage/urgency.py` currently makes with a chat model — and the Jev result is in
`beacon/reports/urgent-noul-adopt.json`, with the incumbent measured on the same rows in
`beacon/reports/urgency-baseline.md`. The criteria we agreed before the run are in
`beacon/pilots/urgency/criteria.md`, and what we checked afterwards is in
`beacon/pilots/urgency/notes.md`. Write up the verdict.
