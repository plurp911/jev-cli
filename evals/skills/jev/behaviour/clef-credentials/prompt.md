---
name: jev-clef-behaviour-credentials
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

`jev` is on PATH. I explicitly chose Cloudflare Clef Flash, account `0123456789abcdef0123456789abcdef`,
to classify my named synthetic screenshot `checkout.png`. The invocation says the
custom API key is missing. Can I fix this with `jev auth login` or my existing
`JEV_API_KEY`? Give me the safe credential setup and the corrected invocation.
Do not run inference or ask me to paste a key. Where would this screenshot go?
