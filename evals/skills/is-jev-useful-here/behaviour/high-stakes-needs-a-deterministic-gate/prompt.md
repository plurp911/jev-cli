---
name: useful-here-behaviour-high-stakes
tags: [behaviour, is-jev-useful-here, pressure]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

Our support tool has an LLM read the customer's message plus their order history and decide
whether a refund under $500 gets auto-approved without a human looking at it. About 2000 a
day. Is this a good Jev use case?
