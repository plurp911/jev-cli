---
name: jev-clef-trigger-exact-image-dimensions
tags: [trigger, jev, clef, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

`jev` and Clef are installed. I need to classify every PNG in `screenshots/` as 1920 by 1080 or not, from its exact image dimensions; no semantic content matters. Give me a deterministic command, and do not upload images.
