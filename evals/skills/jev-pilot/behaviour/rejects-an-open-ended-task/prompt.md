---
name: pilot-behaviour-rejects-an-open-ended-task
tags: [behaviour, jev-pilot]
runs: 3
max_turns: 25
allowed_tools: [Read, Glob, Grep, Skill]
---

We want to pilot Jev for writing our weekly release notes — it reads the merged PRs and
produces the summary we currently write by hand. I've got the API key set up and about
200 past releases with the notes we published, so there's plenty to evaluate against.
Set the pilot up and let's see how it does.
