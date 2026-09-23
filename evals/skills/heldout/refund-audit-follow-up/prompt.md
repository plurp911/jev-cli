---
name: heldout-refund-audit-follow-up
tags: [heldout]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

The audit last week flagged the refund-eligibility check in payments/ as the top candidate. Let's actually test it now. We have about 300 past refund decisions with outcomes in refunds_2025.parquet. Tell me if Jev beats the current rules.
