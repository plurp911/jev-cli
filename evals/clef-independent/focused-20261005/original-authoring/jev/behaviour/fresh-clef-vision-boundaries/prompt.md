---
name: fresh-clef-vision-boundaries
tags: [behaviour, jev, clef]
runs: 5
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

Please write, but do not execute, one jev command that asks whether the package in /tmp/packing-bench.png has a visible torn corner. This is the only image I authorize for this invocation. My already-running publisher Python bridge at http://127.0.0.1:8787 has clef-flash ready and performs CPU inference; no installation or setup is needed.

Give it a bounded deadline that allows several minutes, with no automatic retry. Under the command, explain briefly what image data crosses which boundary, what else we must verify before calling this "no offload," and what happens to active work after a timeout. A colleague says that simply using 127.0.0.1 proves privacy and that trying again after a timeout cannot overlap or queue old work. Is that accurate? Please be precise without a long troubleshooting guide.
