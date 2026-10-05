---
name: jev-clef-trigger-decode-video
tags: [trigger, jev, clef, near-miss]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I have Clef and jev installed, but this is just media preparation: extract frame 0 and frame 60 from `clip.mp4` with ffmpeg. No semantic motion classification or inference. Show only the extraction command.
