---
name: jev-clef-behaviour-privacy
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

I have `jev` installed. I need the command to classify explicitly named `private-tickets.jsonl` using Clef on Ollama at `http://127.0.0.1:11434`. Rows contain `id`, `body`, sender address and internal notes. This content must stay on this machine. Do I need a cloud key or permission to upload it? Explain what leaves the machine and give the command without executing it. Also explain what changes if I explicitly select a remote HTTPS Ollama host instead.
