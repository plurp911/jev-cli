---
name: useful-here-behaviour-conditional-fit
tags: [behaviour, is-jev-useful-here]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

We get invoice PDFs from about 300 suppliers. Today we run pdftotext and then a pile of
regexes per supplier to pull out the invoice total, the invoice date and the vendor name,
and it breaks every time a supplier changes their template. Someone said we should look
at Jev. Would that work?
