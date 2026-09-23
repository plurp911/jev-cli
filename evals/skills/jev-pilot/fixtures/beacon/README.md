# beacon

Support-desk automation for a small SaaS. Tickets arrive by email, get a team and an
urgency, and land in a queue a human works through.

- `beacon/triage/keywords.py` — the rule that assigns the team today.
- `beacon/triage/urgency.py` — the LLM call that decides whether a ticket is urgent.
- `data/` — exports used for offline work.
- `reports/` — output from past measurement runs.
