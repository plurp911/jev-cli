---
name: fresh-clef-cpu-runbook
tags: [behaviour, jev, clef]
runs: 5
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

I'm updating a short runbook for a CPU-only laptop. Our publisher Python bridge is already running locally at http://127.0.0.1:8787 and has clef-flash ready. Give me one jev command to classify this authorized synthetic text as either cancel or keep: "Please cancel the demo booking." Do not run the command or inspect my machine.

CPU inference may take several minutes. Pick a sensible finite per-attempt deadline for this workload and ensure there are no automatic repeats. I want a concise explanation of the deadline and of who starts, downloads for, and stops this runtime. Also settle our operational disagreement: if jev times out or I disconnect its client, has the active computation stopped, and is immediately submitting another request a clean way to recover? Keep the command and explanation short enough for a runbook.
