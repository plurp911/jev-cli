---
name: routing-weigh-it-not-prove-it
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

Our CI posts a comment on every PR saying which reviewers to add, worked out from a
CODEOWNERS file plus a hand-written list of path patterns. Would a bounded judgment be a
sensible thing to put there at all, or is that overkill for what it is?
