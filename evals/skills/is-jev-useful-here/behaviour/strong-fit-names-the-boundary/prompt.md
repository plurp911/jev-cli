---
name: useful-here-behaviour-strong-fit
tags: [behaviour, is-jev-useful-here]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

Our docs search returns 50 candidate chunks per query and we send all 50 to Claude asking
which ones are actually relevant, then generate the answer in a second Claude call. About
40k queries a month and the relevance pass is most of our bill. Would Jev help here?
