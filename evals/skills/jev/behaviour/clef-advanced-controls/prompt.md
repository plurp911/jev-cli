---
name: jev-clef-behaviour-advanced-controls
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

I already run the project Hugging Face Clef Flash bridge on loopback and `jev` is on PATH. Give a dry-run command over ordered `a.png`, `b.png` video frames using 2 source FPS, total tokenizer limit 16384, textual state limit 4096, and max_pixels 262144. Explain whether max-length generates that many output tokens, whether max-state-tokens 0 is allowed, and whether media-kwargs can contain both fps and num_frames or arbitrary transformer kwargs. Would these flags work with Ollama?
