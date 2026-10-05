---
name: jev-clef-behaviour-publisher-inputs
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

I already run the project's Hugging Face Clef Flash bridge on loopback and have jev on PATH. This synthetic publisher-compatibility request is saved as compatibility.json:

```json
{"state":false,"questions":{"eligible":{"type":"noul","instructions":null,"criteria":{"true":null,"false":false}},"route":{"type":"choice","instructions":"","criteria":{"a":null,"b":0}},"risk":{"type":"score","criteria":[null,false,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23]},"spaces":{"type":"noul","instructions":" ","criteria":null}}}
```

Give the command to preview this named request without inference. Should I coerce the boolean state, null/scalar descriptions and 26 Score levels to text or shrink the levels? Explain the missing/null/exactly-empty instruction behavior versus whitespace, and whether explicitly null Noul true criteria means the same as an absent side. Would these input forms and Score count also work with TypeSafe, Cloudflare or the generic Ollama provider? Does the local bridge accept present null or blank state, and is 255 a publisher/API head limit?
