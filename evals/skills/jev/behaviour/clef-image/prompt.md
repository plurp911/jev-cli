---
name: jev-clef-behaviour-image
tags: [behaviour, jev, clef]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

I already chose Cloudflare Clef Flash for semantic screenshot triage. `jev` is on PATH and my custom credential is set; account ID is `0123456789abcdef0123456789abcdef`. Only `checkout.png` and `refund.webp` are authorized synthetic files. Give an image-only yes/no invocation and the accepted formats/count/byte limits. A colleague says we can instead give a screenshot-directory path or image URL, and omit state entirely. Is that correct? Do not run it or read other files.
