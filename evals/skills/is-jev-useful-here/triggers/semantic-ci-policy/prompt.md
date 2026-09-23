---
name: useful-here-trigger-semantic-ci-policy
tags: [trigger, is-jev-useful-here]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

We keep merging PRs whose description just says "fix". I want CI to flag a PR when the
description doesn't actually explain what changed and why — not a length check, people
just pad it. Would Jev make sense for that or is it a bad idea to put a model in CI?
