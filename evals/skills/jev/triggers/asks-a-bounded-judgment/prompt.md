---
name: jev-trigger-bounded-judgment
tags: [trigger, jev]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

I've got a `support_tickets.csv` with about 900 rows and a free-text `body` column, and
I need to split them into "billing", "bug report" and "everything else" before I can
route them to the right queue. Nothing in the text is consistent enough for keywords —
people write "you charged me twice" one week and "double payment on my card" the next.
`jev` is on my PATH. What's the sensible way to do this from the terminal?
