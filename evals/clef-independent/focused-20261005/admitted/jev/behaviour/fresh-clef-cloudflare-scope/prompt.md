---
name: fresh-clef-cloudflare-scope
tags: [behaviour, jev, clef]
runs: 5
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---

I need a command-only example for hosted Cloudflare clef-flash, not the local bridge. Our Cloudflare account ID is a013dcbe24904e27b48f70c059f21a68. The test state is authorized synthetic text: "Please cancel the demo booking." Show a jev choice command with cancel and keep options, a 60-second per-attempt timeout, and no automatic repeats. Do not execute anything or inspect credentials.

I already use jev auth login for TypeSafe on this workstation. Explain briefly whether that login or JEV_API_KEY can authenticate this Cloudflare call, and name the correct credential sources without asking for or showing any secret value. Assume our secret manager can provision whichever supported source is needed. Also say where this example sends its state so nobody mistakes "clef-flash" for local inference.
