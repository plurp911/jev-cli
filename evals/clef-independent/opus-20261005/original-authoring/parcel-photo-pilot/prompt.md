---
name: opus-blind-clef-parcel-photo-pilot
description: Design a small held-out pilot with duplicate incident groups, uncertain labels, and an honest stop decision.
tags: [opus-blind-clef]
runs: 3
max_turns: 8
---

Use jev-pilot to decide whether Clef could help our warehouse triage, then design a small pilot without running it.

We have 42 explicitly selected PNG parcel photos from 21 incidents: each incident has a close-up and a wider shot of the same parcel. The bounded outcome is intact / visibly damaged / insufficient view. Two staff members disagree on the reference outcome for six incidents. Our existing rule routes everything with a damaged-package scanner flag to manual review; in last week's sample it sent 11 of the 21 incidents to review, took about 4 seconds of software time in total, and staff spent 19 minutes resolving those reviews. That does not give us a trustworthy rule-accuracy estimate yet.

We can afford at most 20 new model judgments in this pilot. We have a separately launched huggingface Clef bridge on a CPU workstation, but nobody has verified whether its loader or execution environment can offload work. No photo may be sent to an external service. We will supply any allowed photo path explicitly; no directory scanning is authorized. I want a plan with a development split, a truly untouched holdout, a comparison against the rule, and a go/no-go rule. Our priority is avoiding unattended acceptance of damaged parcels; a model that sends almost everything to review is not automatically useful.

Address the disagreements, near-duplicate views, abstention, operating cost, and what evidence would justify stopping. If the privacy prerequisite prevents execution, finish the design and say what must be established before any run.
