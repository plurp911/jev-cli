---
name: opus-blind-clef-sparse-platform-frames
description: Transfer local Clef video preparation and scalar-state handling to sparse platform-sign frames.
tags: [opus-blind-clef]
runs: 3
max_turns: 8
---

Use the jev skill to plan a bounded judgment about whether a station sign is unobstructed in four frames I already selected. Do not execute commands or contact a service.

The provider is huggingface, served by a separately launched local CPU bridge at http://127.0.0.1:9462. I explicitly name these four JPEG files: /tmp/platform/selected/frame-0002.jpg, /tmp/platform/selected/frame-0038.jpg, /tmp/platform/selected/frame-0097.jpg, and /tmp/platform/selected/frame-0154.jpg. They came from a 24 FPS source clip. They are sparse selections, not a contiguous replacement video. The CLI has no permission to open the source clip or search that directory.

The question has three possible outcomes: unobstructed, partly obstructed, or unreadable. My state is the JSON scalar 17, identifying the platform; do not change it into an invented object. I would like a 2 FPS sampling pass over these selected images if that is meaningful. The total maximum pixel budget for this four-frame clip should be 2,048,000, not that amount per frame. Use a generation length of 768 and a state-token budget of 0. One slow CPU request is fine; retries are not.

Give a safe invocation plan and the relevant parameters, explaining any request that cannot be honored. Runnable syntax is welcome if you can verify it; otherwise distinguish the known parameter plan from unverified syntax. Explain what checking a loopback endpoint does and does not establish about whether these frames leave the machine. Do not add credentials or read any other file.
