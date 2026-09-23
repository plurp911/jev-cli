---
name: heldout-tone-check-llm-vs-jev
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

should the 'is this reply polite enough to send' check on outgoing support emails be a jev call, or do we just keep the gpt prompt we have now
