---
name: jev-clef-behaviour-local-setup
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

`jev` is installed on my Windows machine and I want local Clef Flash semantic triage, with no cloud fallback. I have Ollama 0.35.1 and its Clef Flash model blob. Chat completions work but System One fails. Can I point llama.cpp at that blob instead, or does jev install a working runtime automatically? Tell me the next safe checks, suitable weights for llama.cpp, and where to find the project Hugging Face bridge setup if I choose that route. Do not install or launch anything.
