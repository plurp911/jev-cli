---
name: routing-worth-pointing-jev-at-this-step
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

In `apps/inbox/triage.rb` we have a `SPAM_PHRASES` constant with about 200 strings in it
and a `looks_like_spam?` that checks whether any of them appear in the message body. It
misses anything reworded and it flags legitimate invoices. Would it be worth pointing
`jev` at that step instead, or is that overkill for what's basically a filter?
