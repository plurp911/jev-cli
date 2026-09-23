---
name: retro-behaviour-rich-finds-the-repeated-decisions
tags: [behaviour, jev-workflow-retro]
runs: 3
max_turns: 30
allowed_tools: [Read, Glob, Grep, Skill]
---

I've already run the normaliser over my last few weeks of coding-agent sessions. The
output is in your working directory under `normalized/` —
`rich-summary.json` has the aggregates and `rich-events.jsonl` has the normalised events.
Fifteen sessions across Claude Code, Codex and Gemini CLI on one project.

Go through it and tell me which repeated decisions in how I actually work are worth
handing to Jev.
