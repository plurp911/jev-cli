---
name: routing-one-step-named-not-the-repo
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

The bit I keep coming back to is `keep_relevant` in `pulse/kb/rerank.py` — a gpt-4o call
that tells us which of 40 retrieved chunks are worth keeping. Would Jev be a better fit
for that one step? I'm not asking about the rest of the app.
