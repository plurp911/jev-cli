---
name: fresh-png-dimensions-no-inference
tags: [triggers, negative, metadata]
runs: 5
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

I only need the exact pixel width and height recorded in the PNG header of /tmp/packing-bench.png. Write a small offline Python 3 standard-library command or snippet that I can run myself; do not open the file here. This is deterministic metadata extraction, not an image interpretation task. I explicitly do not want a model, jev, a server, network access, package installation, or a probabilistic answer. Handle a missing file or a non-PNG input with a clear error.
